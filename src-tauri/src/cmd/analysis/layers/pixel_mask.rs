use crate::cmd::common::load_companions;
use crate::core::imaging::dq_flags::exclusion_map;
use crate::core::imaging::region::{paint_exclusions, RegionShape};

pub(crate) struct DqMask {
    pub map: ndarray::Array2<u8>,
    pub excluded: u64,
}

pub(crate) fn resolve_dq_mask(path: &str, exclude_dq: bool, dims: (usize, usize)) -> Option<DqMask> {
    if !exclude_dq {
        return None;
    }
    let comps = match load_companions(path) {
        Ok(comps) => comps,
        Err(e) => {
            log::warn!("DQ exclusion unavailable for {}: {:#}", path, e);
            return None;
        }
    };
    let exclusion = comps.exclusion_mask();
    let (entry, _) = comps.dq.filter(|_| exclusion != 0)?;
    let plane = entry.int_plane().filter(|plane| plane.bits.dim() == dims)?;
    let map = exclusion_map(&plane.bits, exclusion);
    let excluded = map.iter().filter(|&&v| v != 0).count() as u64;
    Some(DqMask { map, excluded })
}

pub(crate) struct PixelMask {
    pub map: ndarray::Array2<u8>,
    pub dq_excluded: Option<u64>,
    pub region_excluded: u64,
    pub exclude_skipped: usize,
}

impl PixelMask {
    pub fn masked(&self) -> bool {
        self.dq_excluded.is_some() || self.region_excluded > 0
    }

    pub fn notes(&self) -> Vec<String> {
        exclude_skipped_note(self.exclude_skipped).into_iter().collect()
    }
}

fn exclude_skipped_note(skipped: usize) -> Option<String> {
    let plural = if skipped == 1 { "" } else { "s" };
    (skipped > 0).then(|| format!("{skipped} exclude region{plural} skipped: invalid shape"))
}

pub(crate) fn resolve_pixel_mask(
    path: &str,
    exclude_dq: bool,
    exclude: &[RegionShape],
    dims: (usize, usize),
) -> Option<PixelMask> {
    let dq = resolve_dq_mask(path, exclude_dq, dims);
    if dq.is_none() && exclude.is_empty() {
        return None;
    }
    let dq_excluded = dq.as_ref().map(|m| m.excluded);
    let mut map = dq.map_or_else(|| ndarray::Array2::zeros(dims), |m| m.map);
    let paint = paint_exclusions(&mut map, exclude);
    Some(PixelMask { map, dq_excluded, region_excluded: paint.painted, exclude_skipped: paint.skipped })
}
