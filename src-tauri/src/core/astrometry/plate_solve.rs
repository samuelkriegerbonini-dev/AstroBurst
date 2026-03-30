use std::collections::HashMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveResult {
    pub success: bool,
    pub ra_center: f64,
    pub dec_center: f64,
    pub orientation: f64,
    pub pixel_scale: f64,
    pub field_w_arcmin: f64,
    pub field_h_arcmin: f64,
    pub index_name: String,
    pub stars_used: usize,
    pub wcs_headers: HashMap<String, String>,
    #[serde(default)]
    pub annotations: Vec<FieldAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldAnnotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub names: Vec<String>,
    pub pixelx: f64,
    pub pixely: f64,
    pub radius: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveConfig {
    pub api_url: String,
    pub api_key: String,
    pub ra_hint: Option<f64>,
    pub dec_hint: Option<f64>,
    pub radius_hint: Option<f64>,
    pub scale_low: Option<f64>,
    pub scale_high: Option<f64>,
    pub max_stars: Option<usize>,
}

impl Default for SolveConfig {
    fn default() -> Self {
        Self {
            api_url: "https://nova.astrometry.net".into(),
            api_key: String::new(),
            ra_hint: None,
            dec_hint: None,
            radius_hint: Some(10.0),
            scale_low: None,
            scale_high: None,
            max_stars: Some(100),
        }
    }
}

pub fn solve_offline_placeholder() -> Result<SolveResult> {
    bail!(
        "Offline plate solving not available. \
         Use astrometry.net API by enabling the 'astrometry-net' feature, \
         or provide an image with WCS headers."
    )
}
