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
    let temp = tempfile::NamedTempFile::new()
        .context("creating a temporary FITS file")?
        .into_temp_path();
    let path = temp
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
        return std::fs::File::open(&temp).context("reopening the written FITS file");
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
    std::fs::File::open(&temp).context("reopening the written FITS file")
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

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use astroburst_lib::types::ImageStats;

    use super::*;
    use crate::config::ServerConfig;
    use crate::router::build_router_with;
    use crate::science::config::ScienceConfig;
    use crate::science::state::ScienceState;
    use crate::state::AppState;
    use crate::tests::v2_fixtures::write_mef_with_dq;
    use crate::v2::images::register_and_respond;

    async fn send(app: &Router, request: Request<Body>) -> Response {
        app.clone().oneshot(request).await.unwrap()
    }

    fn product(dir: &std::path::Path, name: &str, write: impl FnOnce(&str) -> anyhow::Result<()>) -> Vec<u8> {
        let path = dir.join(name);
        write(path.to_str().unwrap()).unwrap();
        std::fs::read(path).unwrap()
    }

    #[tokio::test]
    async fn a_downloaded_ref_carries_the_bytes_the_writer_puts_in_its_product() {
        let root = tempfile::tempdir().unwrap();
        let mef = root.path().join("jw_cal.fits");
        write_mef_with_dq(&mef, 5, 4);
        let state = AppState::new(Arc::new(ServerConfig::default()));
        let session = state.create_session().unwrap();
        let sid = session.id.clone();
        let server_root = root.path().to_str().unwrap().to_owned();
        let cfg = ScienceConfig::from_lookup("127.0.0.1:8080".parse().unwrap(), move |name| {
            (name == "ASTROBURST_SERVER_ROOT").then(|| server_root.clone())
        });
        let app = build_router_with(state.clone(), Arc::new(ScienceState::new(cfg)));
        let opened = send(
            &app,
            Request::post(format!("/v2/sessions/{sid}/open"))
                .header(header::HOST, "127.0.0.1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "path": mef, "hdu": 3, "name": "dq" }).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(opened.status(), StatusCode::OK);
        let pixels = Array2::from_shape_vec((2, 3), vec![1.5f32, -0.0, f32::NAN, f32::MIN_POSITIVE, -3.25e7, f32::INFINITY]).unwrap();
        session.cache.insert_synthetic_with_header("odd", Arc::new(pixels), ImageStats::default(), None);
        let odd = session.cache.get("odd").unwrap();
        register_and_respond(&session, "odd".into(), None, None, &odd, Value::Null);

        let products = tempfile::tempdir().unwrap();
        let mono = product(products.path(), "odd.fits", |path| write_fits_mono(path, odd.arr(), odd.header()));
        let dq = session.cache.get("dq").unwrap();
        let plane = dq.int_plane().unwrap();
        let signed = signed_bits(plane);
        let flags = product(products.path(), "dq.fits", |path| {
            write_mef_images(
                path,
                None,
                &[ImageHdu {
                    data: int_data(plane, signed.as_ref()),
                    header: dq.header(),
                    extname: EXTNAME_DQ,
                    extver: 1,
                }],
            )
        });

        for (name, expected) in [("odd", mono), ("dq", flags)] {
            let response = send(
                &app,
                Request::get(format!("/v2/sessions/{sid}/images/{name}/fits"))
                    .header(header::HOST, "127.0.0.1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            assert_eq!(response.headers()[header::CONTENT_LENGTH], expected.len().to_string().as_str(), "{name}");
            let served = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(expected.len() >= 2880, "{name}");
            assert!(served[..] == expected[..], "{name}: served {} bytes, the product has {}", served.len(), expected.len());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_downloaded_ref_is_written_to_a_file_only_its_owner_can_read() {
        use std::os::unix::fs::PermissionsExt;
        let cache = ImageCache::new(4, usize::MAX);
        cache.insert_synthetic("odd", Arc::new(Array2::from_elem((3, 2), 1.5f32)), ImageStats::default());
        let entry = cache.get("odd").unwrap();
        let file = written_fits(&cache, &entry).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(file.metadata().unwrap().len(), 5760);
    }
}
