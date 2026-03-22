use super::image::{ImageStats, ScnrConfig, StfParams};
use super::stacking::DrizzleConfig;

#[derive(Debug, Clone)]
pub enum WhiteBalance {
    Auto,
    Manual(f64, f64, f64),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMethod {
    PhaseCorrelation,
    Affine,
}

impl Default for AlignMethod {
    fn default() -> Self {
        Self::PhaseCorrelation
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelStats {
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub mean: f64,
}

impl From<&ImageStats> for ChannelStats {
    fn from(st: &ImageStats) -> Self {
        Self { min: st.min, max: st.max, median: st.median, mean: st.mean }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DimensionCrop {
    pub original_r: Option<[usize; 2]>,
    pub original_g: Option<[usize; 2]>,
    pub original_b: Option<[usize; 2]>,
    pub cropped_to: [usize; 2],
}

#[derive(Debug, Clone)]
pub struct RgbComposeConfig {
    pub white_balance: WhiteBalance,
    pub auto_stretch: bool,
    pub stf_r: Option<StfParams>,
    pub stf_g: Option<StfParams>,
    pub stf_b: Option<StfParams>,
    pub linked_stf: bool,
    pub align: bool,
    pub align_method: AlignMethod,
    pub scnr: Option<ScnrConfig>,
    pub dimension_tolerance: usize,
}

impl Default for RgbComposeConfig {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::Auto,
            auto_stretch: true,
            stf_r: None,
            stf_g: None,
            stf_b: None,
            linked_stf: false,
            align: true,
            align_method: AlignMethod::PhaseCorrelation,
            scnr: None,
            dimension_tolerance: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RgbComposeResult {
    pub png_path: String,
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub offset_g: (f64, f64),
    pub offset_b: (f64, f64),
    pub width: usize,
    pub height: usize,
    pub scnr_applied: bool,
    pub dimension_crop: Option<DimensionCrop>,
}

#[derive(Debug, Clone)]
pub struct DrizzleRgbConfig {
    pub drizzle: DrizzleConfig,
    pub white_balance: WhiteBalance,
    pub auto_stretch: bool,
    pub linked_stf: bool,
    pub scnr: Option<ScnrConfig>,
}

impl Default for DrizzleRgbConfig {
    fn default() -> Self {
        Self {
            drizzle: DrizzleConfig::default(),
            white_balance: WhiteBalance::Auto,
            auto_stretch: true,
            linked_stf: false,
            scnr: None,
        }
    }
}

#[derive(Debug)]
pub struct DrizzleRgbResult {
    pub png_path: String,
    pub fits_path: Option<String>,
    pub input_dims: (usize, usize),
    pub output_dims: (usize, usize),
    pub scale: f64,
    pub frame_count_r: usize,
    pub frame_count_g: usize,
    pub frame_count_b: usize,
    pub rejected_pixels: u64,
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub scnr_applied: bool,
}
