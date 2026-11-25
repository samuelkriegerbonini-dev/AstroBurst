use std::fs::File;

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::core::cube::lazy::LazyCube;
use crate::core::ramp::info::RampInfo;
use crate::infra::fits::reader::{extract_int_plane_by_index, list_extensions, HduInfo};
use crate::types::ImageRef;

pub const DATAMODEL_RAMP: &str = "RampModel";
pub const EXT_GROUPDQ: &str = "GROUPDQ";
pub const EXT_PIXELDQ: &str = "PIXELDQ";
pub const NOTE_RAMP_MODEL_DQ: &str = "RampModel input: saturated, jump and do-not-use groups are taken from GROUPDQ and pixel flags from PIXELDQ; the fixed sat_dn threshold is not applied";
pub const NOTE_RAMP_MODEL_NO_PIXELDQ: &str = "RampModel input with no PIXELDQ extension: saturated, jump and do-not-use groups are taken from GROUPDQ and no pixel flags are carried into DQ; the fixed sat_dn threshold is not applied";

pub struct RampDq {
    group: LazyCube,
    pixel: Option<Array2<u32>>,
}

fn hdu_named(hdus: &[HduInfo], name: &str) -> Option<usize> {
    hdus.iter().find(|h| h.extname.as_deref().is_some_and(|n| n.trim().trim_matches('\'').trim().eq_ignore_ascii_case(name))).map(|h| h.index)
}

impl RampDq {
    pub fn open(cube: &LazyCube, info: &RampInfo) -> Result<Option<RampDq>> {
        if info.datamodl.as_deref() != Some(DATAMODEL_RAMP) {
            return Ok(None);
        }
        let path = &cube.source_path;
        let file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
        let hdus = list_extensions(&file)?;
        let Some(group_index) = hdu_named(&hdus, EXT_GROUPDQ) else {
            bail!(
                "{path} is a RampModel without a {EXT_GROUPDQ} extension: its samples are already corrected, so the raw saturation threshold cannot find the saturated groups; use the rate of calwebb_detector1 or a ramp file that keeps {EXT_GROUPDQ}"
            );
        };
        let group = LazyCube::open(&ImageRef::hdu(path, group_index).cache_key()).with_context(|| format!("Cannot read {EXT_GROUPDQ} of {path}"))?;
        let (flags, science) = (&group.geometry, &cube.geometry);
        if (flags.naxis1, flags.naxis2, flags.naxis3, flags.naxis4) != (science.naxis1, science.naxis2, science.naxis3, science.naxis4) {
            bail!(
                "{path}: {EXT_GROUPDQ} is {} x {} x {} x {} but SCI is {} x {} x {} x {}",
                flags.naxis1,
                flags.naxis2,
                flags.naxis3,
                flags.naxis4,
                science.naxis1,
                science.naxis2,
                science.naxis3,
                science.naxis4
            );
        }
        let pixel = match hdu_named(&hdus, EXT_PIXELDQ) {
            Some(index) => {
                let plane = extract_int_plane_by_index(&file, index).with_context(|| format!("Cannot read {EXT_PIXELDQ} of {path}"))?.bits;
                if plane.dim() != (science.naxis2, science.naxis1) {
                    bail!("{path}: {EXT_PIXELDQ} is {} x {} but SCI is {} x {}", plane.ncols(), plane.nrows(), science.naxis1, science.naxis2);
                }
                Some(plane)
            }
            None => None,
        };
        Ok(Some(RampDq { group, pixel }))
    }

    pub fn note(&self) -> &'static str {
        match self.pixel {
            Some(_) => NOTE_RAMP_MODEL_DQ,
            None => NOTE_RAMP_MODEL_NO_PIXELDQ,
        }
    }

    pub fn decode_groups(&self, z_start: usize, z_end: usize, row_start: usize, row_count: usize) -> Result<Vec<Vec<f32>>> {
        self.group.decode_row_band_batch(z_start, z_end, row_start, row_count)
    }

    pub fn pixel_bits(&self, y: usize, x: usize) -> u32 {
        self.pixel.as_ref().and_then(|plane| plane.get((y, x)).copied()).unwrap_or(0)
    }
}
