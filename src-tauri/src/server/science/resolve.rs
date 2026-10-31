use astroburst_lib::infra::cache::ImageEntry;
use astroburst_lib::types::image_ref::ImageRef;

use super::codes;
use super::error::ScienceError;
use super::guard::confine;
use super::state::ScienceState;
use crate::error::AppError;
use crate::session::Session;

pub(crate) struct ResolvedRef {
    pub(crate) name: String,
    pub(crate) entry: ImageEntry,
    pub(crate) wcs_source: String,
}

pub(crate) async fn resolve_ref(
    session: &Session,
    explicit: Option<String>,
) -> Result<ResolvedRef, ScienceError> {
    let name = session.target_ref(explicit).await?;
    let entry = session
        .cache
        .get(&name)
        .ok_or_else(|| AppError::NotFound(format!("image ref {name} not found in session")))?;
    let wcs_source = session.wcs_source_path(&name);
    Ok(ResolvedRef {
        name,
        entry,
        wcs_source,
    })
}

pub(crate) async fn require_file_source(
    science: &ScienceState,
    resolved: &ResolvedRef,
) -> Result<ImageRef, ScienceError> {
    if resolved.wcs_source.is_empty() {
        return Err(ScienceError::bad_request(
            codes::DERIVED_REF_UNSUPPORTED,
            format!(
                "ref {} is a derived product, not a plane of a file",
                resolved.name
            ),
            Some("open the file with POST /v2/sessions/:sid/open and use that ref".into()),
        ));
    }
    let plane = ImageRef::parse(&resolved.wcs_source);
    confine(&science.cfg, vec![plane.path.clone()]).await?;
    Ok(plane)
}
