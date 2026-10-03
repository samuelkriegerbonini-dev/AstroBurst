use std::path::Path;

use ndarray::Array2;

use crate::core::cube::lazy::test_support::write_u16_ramp_mef;
use crate::core::ramp::irs2::{Irs2Layout, Irs2Sample};
use crate::infra::fits::writer::{write_mef_images, HduData, ImageHdu};
use crate::types::header::HduHeader;

pub const OWNER_ROWS: usize = 3200;
pub const OWNER_NGROUPS: usize = 10;
pub const OWNER_TGROUP_S: f64 = 14.589;
pub const ROW_TERM_STEPS: u64 = 10;
pub const UNCAL_MAX_DN: f32 = 65535.0;

pub struct SyntheticIrs2 {
    pub nrs_norm: usize,
    pub nrs_ref: usize,
    pub noutputs: usize,
    pub reversed: bool,
}

pub struct SyntheticUncal {
    pub cols: usize,
    pub rows: usize,
    pub ngroups: usize,
    pub nints: usize,
    pub irs2: Option<SyntheticIrs2>,
    pub tgroup_s: f64,
    pub detector: &'static str,
    pub write_fastaxis: bool,
}

pub fn owner_like_irs2(cols: usize, reversed: bool) -> SyntheticUncal {
    SyntheticUncal {
        cols,
        rows: OWNER_ROWS,
        ngroups: OWNER_NGROUPS,
        nints: 1,
        irs2: Some(SyntheticIrs2 { nrs_norm: 16, nrs_ref: 4, noutputs: 5, reversed }),
        tgroup_s: OWNER_TGROUP_S,
        detector: if reversed { "NRS2" } else { "NRS1" },
        write_fastaxis: true,
    }
}

pub fn write_synthetic_uncal(path: &Path, spec: &SyntheticUncal, value: impl Fn(usize, usize, usize, usize) -> f32) -> String {
    let detector = format!("'{}'", spec.detector);
    let nints = spec.nints.to_string();
    let ngroups = spec.ngroups.to_string();
    let tgroup = format!("{}", spec.tgroup_s);
    let irs2_cards = spec
        .irs2
        .as_ref()
        .map(|irs2| (irs2.nrs_norm.to_string(), irs2.nrs_ref.to_string(), irs2.noutputs.to_string(), irs2.reversed));
    let mut primary: Vec<(&str, &str)> = vec![
        ("TELESCOP", "'JWST'"),
        ("INSTRUME", "'NIRSPEC'"),
        ("DETECTOR", &detector),
        ("DATAMODL", "'Level1bModel'"),
        ("READPATT", "'NRSIRS2RAPID'"),
        ("NINTS", &nints),
        ("NGROUPS", &ngroups),
        ("NFRAMES", "1"),
        ("GROUPGAP", "0"),
        ("TFRAME", &tgroup),
        ("TGROUP", &tgroup),
        ("NEXTEND", "3"),
    ];
    if let Some((norm, reference, outputs, reversed)) = &irs2_cards {
        primary.push(("NRS_NORM", norm));
        primary.push(("NRS_REF", reference));
        primary.push(("NOUTPUTS", outputs));
        if spec.write_fastaxis {
            primary.push(("FASTAXIS", if *reversed { "-2" } else { "2" }));
            primary.push(("SLOWAXIS", if *reversed { "-1" } else { "1" }));
        }
    }
    write_u16_ramp_mef(path, spec.cols, spec.rows, spec.ngroups, spec.nints, &primary, &[("BUNIT", "'DN'")], |i, g, y, x| {
        value(i, g, y, x).round().clamp(0.0, UNCAL_MAX_DN) as u16
    });
    path.to_str().unwrap().to_string()
}

fn splitmix64(mut state: u64) -> u64 {
    state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn row_term(science_row: usize) -> f32 {
    let hash = splitmix64((science_row as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    (hash % ROW_TERM_STEPS) as f32 / ROW_TERM_STEPS as f32
}

pub fn irs2_truth_slope(science_row: usize, x: usize) -> f32 {
    0.1 * x as f32 + row_term(science_row)
}

pub fn deterministic_noise(seed: u64, index: usize) -> f32 {
    let base = seed.wrapping_mul(0xD1B5_4A32_D192_ED03) ^ (index as u64).wrapping_mul(0x8CB9_2BA7_2F3D_8DD7);
    let mut sum = 0.0f64;
    for k in 0..12u64 {
        let bits = splitmix64(base.wrapping_add(k.wrapping_mul(0xA24B_AED4_963E_E407)));
        sum += (bits >> 11) as f64 / (1u64 << 53) as f64;
    }
    (sum - 6.0) as f32
}

pub const SYNTHETIC_DRIFT_DN: [f32; 10] = [0.0, 7.0, -3.0, 12.0, 5.0, -8.0, 2.0, 9.0, -4.0, 6.0];
pub const SYNTHETIC_PEDESTAL_DN: f32 = 1000.0;
pub const SYNTHETIC_REF_OUTPUT_DN: f32 = 500.0;
pub const SYNTHETIC_COLUMN_SLOPE_DN: f32 = 0.5;
pub const SYNTHETIC_TGROUP_S: f64 = 10.0;
pub const SYNTHETIC_ERR: f32 = 0.05;

pub fn irs2_synthetic_value(layout: &Irs2Layout, tgroup_s: f64) -> impl Fn(usize, usize, usize, usize) -> f32 + '_ {
    move |_integration, g, y, x| {
        let drift = SYNTHETIC_DRIFT_DN[g % SYNTHETIC_DRIFT_DN.len()];
        let base = SYNTHETIC_PEDESTAL_DN + SYNTHETIC_COLUMN_SLOPE_DN * x as f32 + drift;
        match layout.sample_kind(y) {
            Irs2Sample::RefOutput => SYNTHETIC_REF_OUTPUT_DN,
            Irs2Sample::Reference => base,
            Irs2Sample::Science => {
                let science_row = layout.science_row_of(y).expect("science rows map to the stripped frame");
                base + irs2_truth_slope(science_row, x) * (tgroup_s * g as f64) as f32
            }
        }
    }
}

pub fn drift_slope_dn_per_s(tgroup_s: f64) -> f64 {
    let n = SYNTHETIC_DRIFT_DN.len() as f64;
    let t_mean = tgroup_s * (n - 1.0) / 2.0;
    let d_mean = SYNTHETIC_DRIFT_DN.iter().map(|d| *d as f64).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (g, d) in SYNTHETIC_DRIFT_DN.iter().enumerate() {
        let dt = g as f64 * tgroup_s - t_mean;
        num += dt * (*d as f64 - d_mean);
        den += dt * dt;
    }
    num / den
}

pub fn truth_rate_planes(layout: &Irs2Layout, cols: usize) -> (Array2<f32>, Array2<f32>, Array2<u32>) {
    let rows = layout.science_rows();
    let sci = Array2::from_shape_fn((rows, cols), |(s, x)| irs2_truth_slope(s, x));
    let err = Array2::from_elem((rows, cols), SYNTHETIC_ERR);
    let dq = Array2::zeros((rows, cols));
    (sci, err, dq)
}

pub fn write_synthetic_rate(path: &Path, sci: &Array2<f32>, err: &Array2<f32>, dq: &Array2<u32>, detector: &str) -> String {
    let mut primary = HduHeader::empty();
    primary.set("TELESCOP", "JWST".to_string());
    primary.set("INSTRUME", "NIRSPEC".to_string());
    primary.set("DETECTOR", detector.to_string());
    let mut sci_header = HduHeader::empty();
    sci_header.set("BUNIT", "DN/s".to_string());
    let hdus = [
        ImageHdu { data: HduData::F32(sci), header: Some(&sci_header), extname: "SCI", extver: 1 },
        ImageHdu { data: HduData::F32(err), header: Some(&sci_header), extname: "ERR", extver: 1 },
        ImageHdu { data: HduData::U32(dq), header: None, extname: "DQ", extver: 1 },
    ];
    let text = path.to_str().unwrap().to_string();
    write_mef_images(&text, Some(&primary), &hdus).unwrap();
    text
}
