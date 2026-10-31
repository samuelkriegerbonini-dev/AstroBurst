use std::sync::Arc;

use anyhow::Context;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use ndarray::Array2;
use tokio_util::io::ReaderStream;

use astroburst_lib::infra::cache::{ImageCache, ImageEntry};
use astroburst_lib::infra::fits::writer::{write_fits_mono, write_mef_images, HduData, ImageHdu};
use astroburst_lib::infra::image_source::{load_companions_into, LoadedCompanions};
use astroburst_lib::types::constants::{EXTNAME_DQ, EXTNAME_ERR, EXTNAME_SCI};
use astroburst_lib::types::image::IntPlane;
use astroburst_lib::types::image_ref::sanitize_fragment;

use super::error::ScienceError;
use super::resolve::resolve_ref;
use crate::error::AppError;
use crate::extractors::SessionExtractor;
use crate::v2::images::load_ref;

const FITS_MEDIA_TYPE: &str = "application/fits";
const READ_CHUNK_BYTES: usize = 1 << 16;

pub(crate) async fn delete(
    SessionExtractor(session): SessionExtractor,
    Path((_, name)): Path<(String, String)>,
) -> Result<StatusCode, ScienceError> {
    let had_meta = session.v2.meta.remove(&name).is_some();
    let had_pixels = session.cache.contains(&name);
    session.cache.remove(&name);
    if !(had_meta || had_pixels) {
        return Err(AppError::NotFound(format!("image ref {name} not found in session")).into());
    }
    let mut active = session.v2.active_ref.write().await;
    if active.as_deref() == Some(name.as_str()) {
        *active = None;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn fits(
    SessionExtractor(session): SessionExtractor,
    Path((_, name)): Path<(String, String)>,
) -> Result<Response, ScienceError> {
    let entry = resolve_ref(&session, Some(name.clone())).await?.entry;
    let cache = Arc::clone(&session.cache);
    let file = tokio::task::spawn_blocking(move || written_fits(&cache, &entry))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))??;
    let length = file
        .metadata()
        .context("reading the size of the written FITS file")?
        .len();
    let disposition = HeaderValue::try_from(format!(
        "attachment; filename=\"{}.fits\"",
        sanitize_fragment(&name)
    ))
    .context("building the download file name")?;
    let body = Body::from_stream(ReaderStream::with_capacity(
        tokio::fs::File::from_std(file),
        READ_CHUNK_BYTES,
    ));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(FITS_MEDIA_TYPE),
            ),
            (header::CONTENT_LENGTH, HeaderValue::from(length)),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}

fn written_fits(cache: &ImageCache, entry: &ImageEntry) -> anyhow::Result<std::fs::File> {
    let temp = tempfile::NamedTempFile::new().context("creating a temporary FITS file")?;
    let path = temp
        .path()
        .to_str()
        .context("the temporary folder path is not UTF-8")?;
    if let Some(plane) = entry.int_plane() {
        let signed = signed_bits(plane);
        let flags = ImageHdu {
            data: int_data(plane, signed.as_ref()),
            header: entry.header(),
            extname: EXTNAME_DQ,
            extver: 1,
        };
        write_mef_images(path, None, &[flags])?;
        return temp.reopen().context("reopening the written FITS file");
    }
    let companions = match entry.plane_info() {
        Some(info) if info.is_dq || info.is_err => LoadedCompanions::default(),
        _ => load_companions_into(cache, entry, load_ref),
    };
    let dq = companions
        .dq
        .as_ref()
        .and_then(|(dq, _)| dq.int_plane().map(|plane| (dq, plane)));
    match (companions.err.as_ref(), dq) {
        (None, None) => write_fits_mono(path, entry.arr(), entry.header())?,
        (err, dq) => {
            let signed = dq.and_then(|(_, plane)| signed_bits(plane));
            let mut hdus = vec![ImageHdu {
                data: HduData::F32(entry.arr()),
                header: entry.header(),
                extname: EXTNAME_SCI,
                extver: 1,
            }];
            if let Some(err) = err {
                hdus.push(ImageHdu {
                    data: HduData::F32(err.arr()),
                    header: err.header(),
                    extname: EXTNAME_ERR,
                    extver: 1,
                });
            }
            if let Some((dq, plane)) = dq {
                hdus.push(ImageHdu {
                    data: int_data(plane, signed.as_ref()),
                    header: dq.header(),
                    extname: EXTNAME_DQ,
                    extver: 1,
                });
            }
            write_mef_images(path, None, &hdus)?;
        }
    }
    temp.reopen().context("reopening the written FITS file")
}

fn signed_bits(plane: &IntPlane) -> Option<Array2<i32>> {
    plane.signed.then(|| plane.bits.mapv(|b| b as i32))
}

fn int_data<'a>(plane: &'a IntPlane, signed: Option<&'a Array2<i32>>) -> HduData<'a> {
    match signed {
        Some(bits) => HduData::I32(bits),
        None => HduData::U32(&plane.bits),
    }
}
