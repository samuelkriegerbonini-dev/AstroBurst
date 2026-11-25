use std::sync::Arc;

use ndarray::Array2;

use crate::cmd::common::{cached_header, extract_image_resolved, image_ref, try_extract_rgb_resolved};
use crate::cmd::compose::rescale_header_to_grid;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::constants::{COMPOSITE_KEY_B, COMPOSITE_KEY_G, COMPOSITE_KEY_R};
use crate::types::header::HduHeader;

pub(super) struct ExportChannel {
    pub(super) arr: Arc<Array2<f32>>,
    pub(super) header: Option<HduHeader>,
}

pub(super) struct RgbExport {
    pub(super) r: Arc<Array2<f32>>,
    pub(super) g: Arc<Array2<f32>>,
    pub(super) b: Arc<Array2<f32>>,
    pub(super) header: Option<HduHeader>,
}

pub(super) fn load_export_channel(path: &str) -> anyhow::Result<ExportChannel> {
    if image_ref(path).is_synthetic() {
        let entry = GLOBAL_IMAGE_CACHE.get(path).ok_or_else(|| {
            anyhow::anyhow!("'{}' is no longer in memory: re-run the step that produced it", path)
        })?;
        return Ok(ExportChannel { arr: entry.data_arc(), header: entry.header().cloned() });
    }
    let resolved = extract_image_resolved(path)?;
    Ok(ExportChannel { arr: Arc::new(resolved.arr), header: Some(resolved.header) })
}

fn same_file_triplet(paths: [Option<&str>; 3]) -> Option<&str> {
    match paths {
        [Some(r), Some(g), Some(b)] if r == g && g == b && !image_ref(r).is_synthetic() => Some(r),
        _ => None,
    }
}

pub(super) fn rgb_file_export(paths: [Option<&str>; 3]) -> anyhow::Result<Option<RgbExport>> {
    let Some(path) = same_file_triplet(paths) else {
        return Ok(None);
    };
    Ok(try_extract_rgb_resolved(path)?.map(|rgb| RgbExport {
        r: Arc::new(rgb.r),
        g: Arc::new(rgb.g),
        b: Arc::new(rgb.b),
        header: Some(rgb.header),
    }))
}

pub(super) fn resample_to(arr: Arc<Array2<f32>>, dims: (usize, usize)) -> anyhow::Result<Arc<Array2<f32>>> {
    if arr.dim() == dims {
        return Ok(arr);
    }
    Ok(Arc::new(crate::core::imaging::resample::resample_image(&arr, dims.0, dims.1)?))
}

pub(super) fn channels_export(paths: [Option<&str>; 3]) -> anyhow::Result<RgbExport> {
    let [r_path, g_path, b_path] = paths;
    let r = load_export_channel(r_path.ok_or_else(|| anyhow::anyhow!("R channel path required"))?)?;
    let g = load_export_channel(g_path.ok_or_else(|| anyhow::anyhow!("G channel path required"))?)?;
    let b = load_export_channel(b_path.ok_or_else(|| anyhow::anyhow!("B channel path required"))?)?;
    let rows = r.arr.dim().0.max(g.arr.dim().0).max(b.arr.dim().0);
    let cols = r.arr.dim().1.max(g.arr.dim().1).max(b.arr.dim().1);
    let header = [&r, &g, &b].into_iter().find_map(|channel| {
        channel.header.clone().map(|mut header| {
            rescale_header_to_grid(&mut header, channel.arr.dim(), (rows, cols));
            header
        })
    });
    Ok(RgbExport {
        r: resample_to(r.arr, (rows, cols))?,
        g: resample_to(g.arr, (rows, cols))?,
        b: resample_to(b.arr, (rows, cols))?,
        header,
    })
}

fn header_dims(header: &HduHeader) -> Option<(usize, usize)> {
    let rows = usize::try_from(header.get_i64("NAXIS2")?).ok()?;
    let cols = usize::try_from(header.get_i64("NAXIS1")?).ok()?;
    Some((rows, cols))
}

fn channel_header_for_grid(path: &str, grid: (usize, usize)) -> Option<HduHeader> {
    let (mut header, dims) = if image_ref(path).is_synthetic() {
        let entry = GLOBAL_IMAGE_CACHE.get(path)?;
        (entry.header().cloned()?, Some(entry.arr().dim()))
    } else {
        let header = cached_header(path).ok()?;
        let dims = header_dims(&header);
        (header, dims)
    };
    if let Some(dims) = dims {
        rescale_header_to_grid(&mut header, dims, grid);
    }
    Some(header)
}

pub(super) fn composite_export(r_path: Option<&str>) -> Option<RgbExport> {
    let (cr, cg, cb) = match (
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R),
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G),
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B),
    ) {
        (Some(cr), Some(cg), Some(cb)) => (cr, cg, cb),
        _ => return None,
    };
    let header = r_path
        .and_then(|p| channel_header_for_grid(p, cr.arr().dim()))
        .or_else(|| cr.header().cloned());
    Some(RgbExport { r: cr.data_arc(), g: cg.data_arc(), b: cb.data_arc(), header })
}

fn count_real_paths(paths: [Option<&str>; 3]) -> usize {
    paths
        .iter()
        .flatten()
        .filter(|p| !p.starts_with("__") || p.starts_with(crate::types::constants::WIZARD_CACHE_PREFIX))
        .count()
}

pub(super) fn fits_rgb_uses_composite(paths: [Option<&str>; 3]) -> bool {
    count_real_paths(paths) <= 1
}

pub(super) fn png_rgb_uses_composite(paths: [Option<&str>; 3]) -> bool {
    count_real_paths(paths) == 0
}
