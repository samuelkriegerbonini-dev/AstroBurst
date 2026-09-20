use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::types::compose::AlignMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RejectionMethod {
    None,
    #[default]
    SigmaClip,
    WinsorizedSigmaClip,
    LinearFitClip,
    PercentileClip,
    MinMax,
}

impl RejectionMethod {
    pub const SUPPORTED: &'static str =
        "none, sigma_clip, winsorized_sigma_clip, linear_fit_clip, percentile_clip, min_max";

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(RejectionMethod::None),
            "sigma_clip" | "sigma" => Ok(RejectionMethod::SigmaClip),
            "winsorized_sigma_clip" | "winsorized" => Ok(RejectionMethod::WinsorizedSigmaClip),
            "linear_fit_clip" | "linear_fit" => Ok(RejectionMethod::LinearFitClip),
            "percentile_clip" | "percentile" => Ok(RejectionMethod::PercentileClip),
            "min_max" | "minmax" => Ok(RejectionMethod::MinMax),
            other => Err(format!(
                "unknown rejection method '{other}' (supported: {})",
                Self::SUPPORTED
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RejectionMethod::None => "none",
            RejectionMethod::SigmaClip => "sigma_clip",
            RejectionMethod::WinsorizedSigmaClip => "winsorized_sigma_clip",
            RejectionMethod::LinearFitClip => "linear_fit_clip",
            RejectionMethod::PercentileClip => "percentile_clip",
            RejectionMethod::MinMax => "min_max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CombineMethod {
    #[default]
    Mean,
    Median,
    Min,
    Max,
}

impl CombineMethod {
    pub const SUPPORTED: &'static str = "mean, median, min, max";

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mean" | "average" => Ok(CombineMethod::Mean),
            "median" => Ok(CombineMethod::Median),
            "min" | "minimum" => Ok(CombineMethod::Min),
            "max" | "maximum" => Ok(CombineMethod::Max),
            other => Err(format!(
                "unknown combine method '{other}' (supported: {})",
                Self::SUPPORTED
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CombineMethod::Mean => "mean",
            CombineMethod::Median => "median",
            CombineMethod::Min => "min",
            CombineMethod::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NormalizationMethod {
    None,
    Additive,
    Multiplicative,
    #[default]
    AdditiveScaling,
    MultiplicativeScaling,
}

impl NormalizationMethod {
    pub const SUPPORTED: &'static str =
        "none, additive, multiplicative, additive_scaling, multiplicative_scaling";

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(NormalizationMethod::None),
            "additive" => Ok(NormalizationMethod::Additive),
            "multiplicative" => Ok(NormalizationMethod::Multiplicative),
            "additive_scaling" | "additive_with_scaling" => Ok(NormalizationMethod::AdditiveScaling),
            "multiplicative_scaling" | "multiplicative_with_scaling" => {
                Ok(NormalizationMethod::MultiplicativeScaling)
            }
            other => Err(format!(
                "unknown normalization method '{other}' (supported: {})",
                Self::SUPPORTED
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            NormalizationMethod::None => "none",
            NormalizationMethod::Additive => "additive",
            NormalizationMethod::Multiplicative => "multiplicative",
            NormalizationMethod::AdditiveScaling => "additive_scaling",
            NormalizationMethod::MultiplicativeScaling => "multiplicative_scaling",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RejectionNormalization {
    None,
    #[default]
    ScaleOffset,
}

impl RejectionNormalization {
    pub const SUPPORTED: &'static str = "none, scale_offset";

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(RejectionNormalization::None),
            "scale_offset" | "scale+offset" => Ok(RejectionNormalization::ScaleOffset),
            other => Err(format!(
                "unknown rejection normalization '{other}' (supported: {})",
                Self::SUPPORTED
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RejectionNormalization::None => "none",
            RejectionNormalization::ScaleOffset => "scale_offset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RejectionParams {
    pub rejection: RejectionMethod,
    pub combine: CombineMethod,
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub winsor_cutoff: f32,
    pub percentile_low: f32,
    pub percentile_high: f32,
    pub minmax_low: usize,
    pub minmax_high: usize,
}

impl Default for RejectionParams {
    fn default() -> Self {
        Self {
            rejection: RejectionMethod::SigmaClip,
            combine: CombineMethod::Mean,
            sigma_low: 4.0,
            sigma_high: 3.0,
            max_iterations: 5,
            winsor_cutoff: 5.0,
            percentile_low: 0.2,
            percentile_high: 0.1,
            minmax_low: 1,
            minmax_high: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StackConfig {
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub align: bool,
    pub align_method: AlignMethod,
    pub weights: Option<Vec<f64>>,
    pub rejection: RejectionMethod,
    pub combine: CombineMethod,
    pub normalization: NormalizationMethod,
    pub rejection_normalization: RejectionNormalization,
    pub winsor_cutoff: f32,
    pub percentile_low: f32,
    pub percentile_high: f32,
    pub minmax_low: usize,
    pub minmax_high: usize,
    pub rejection_maps: bool,
}

impl Default for StackConfig {
    fn default() -> Self {
        let rejection = RejectionParams::default();
        Self {
            sigma_low: rejection.sigma_low,
            sigma_high: rejection.sigma_high,
            max_iterations: rejection.max_iterations,
            align: true,
            align_method: AlignMethod::default(),
            weights: None,
            rejection: rejection.rejection,
            combine: rejection.combine,
            normalization: NormalizationMethod::AdditiveScaling,
            rejection_normalization: RejectionNormalization::ScaleOffset,
            winsor_cutoff: rejection.winsor_cutoff,
            percentile_low: rejection.percentile_low,
            percentile_high: rejection.percentile_high,
            minmax_low: rejection.minmax_low,
            minmax_high: rejection.minmax_high,
            rejection_maps: false,
        }
    }
}

impl StackConfig {
    pub fn rejection_params(&self) -> RejectionParams {
        RejectionParams {
            rejection: self.rejection,
            combine: self.combine,
            sigma_low: self.sigma_low,
            sigma_high: self.sigma_high,
            max_iterations: self.max_iterations,
            winsor_cutoff: self.winsor_cutoff,
            percentile_low: self.percentile_low,
            percentile_high: self.percentile_high,
            minmax_low: self.minmax_low,
            minmax_high: self.minmax_high,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StackResult {
    pub image: Array2<f32>,
    pub frame_count: usize,
    pub rejected_pixels: u64,
    pub offsets: Vec<(i32, i32)>,
    pub rejection_low: Option<Array2<u16>>,
    pub rejection_high: Option<Array2<u16>>,
    pub normalization_applied: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignmentMethod {
    PhaseCorrelation,
    Zncc,
}

impl Default for AlignmentMethod {
    fn default() -> Self {
        Self::PhaseCorrelation
    }
}

#[derive(Debug, Clone)]
pub struct DrizzleConfig {
    pub scale: f64,
    pub pixfrac: f64,
    pub kernel: DrizzleKernel,
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub sigma_iterations: usize,
    pub align: bool,
    pub alignment_method: AlignmentMethod,
    pub rejection: RejectionMethod,
}

impl Default for DrizzleConfig {
    fn default() -> Self {
        Self {
            scale: 2.0,
            pixfrac: 0.7,
            kernel: DrizzleKernel::Square,
            sigma_low: 3.0,
            sigma_high: 3.0,
            sigma_iterations: 5,
            align: true,
            alignment_method: AlignmentMethod::default(),
            rejection: RejectionMethod::SigmaClip,
        }
    }
}

impl DrizzleConfig {
    pub fn rejection_params(&self) -> RejectionParams {
        RejectionParams {
            rejection: self.rejection,
            combine: CombineMethod::Mean,
            sigma_low: self.sigma_low,
            sigma_high: self.sigma_high,
            max_iterations: self.sigma_iterations.max(1),
            ..RejectionParams::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrizzleKernel {
    Square,
    Gaussian,
    Lanczos3,
}

#[derive(Debug, Clone)]
pub struct DrizzleResult {
    pub image: Array2<f32>,
    pub weight_map: Array2<f32>,
    pub frame_count: usize,
    pub output_scale: f64,
    pub input_dims: (usize, usize),
    pub output_dims: (usize, usize),
    pub offsets: Vec<(f64, f64)>,
    pub rejected_pixels: u64,
}

#[derive(Debug, Clone)]
pub struct RLConfig {
    pub iterations: usize,
    pub psf_sigma: f64,
    pub psf_size: usize,
    pub regularization: f64,
    pub deringing: bool,
    pub deringing_threshold: f32,
}

impl Default for RLConfig {
    fn default() -> Self {
        Self {
            iterations: 20,
            psf_sigma: 2.0,
            psf_size: 15,
            regularization: 0.001,
            deringing: true,
            deringing_threshold: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RLResult {
    pub image: Array2<f32>,
    pub iterations_run: usize,
    pub convergence: f64,
    pub elapsed_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_method_names_round_trip_and_reject_unknown() {
        for m in [
            RejectionMethod::None,
            RejectionMethod::SigmaClip,
            RejectionMethod::WinsorizedSigmaClip,
            RejectionMethod::LinearFitClip,
            RejectionMethod::PercentileClip,
            RejectionMethod::MinMax,
        ] {
            assert_eq!(RejectionMethod::from_name(m.name()), Ok(m));
            assert_eq!(serde_json::to_value(m).unwrap(), serde_json::json!(m.name()));
        }
        assert_eq!(RejectionMethod::from_name(" Winsorized_Sigma_Clip "), Ok(RejectionMethod::WinsorizedSigmaClip));
        assert!(RejectionMethod::from_name("bogus").unwrap_err().contains("bogus"));
    }

    #[test]
    fn combine_and_normalization_names_round_trip() {
        for m in [CombineMethod::Mean, CombineMethod::Median, CombineMethod::Min, CombineMethod::Max] {
            assert_eq!(CombineMethod::from_name(m.name()), Ok(m));
        }
        for m in [
            NormalizationMethod::None,
            NormalizationMethod::Additive,
            NormalizationMethod::Multiplicative,
            NormalizationMethod::AdditiveScaling,
            NormalizationMethod::MultiplicativeScaling,
        ] {
            assert_eq!(NormalizationMethod::from_name(m.name()), Ok(m));
        }
        for m in [RejectionNormalization::None, RejectionNormalization::ScaleOffset] {
            assert_eq!(RejectionNormalization::from_name(m.name()), Ok(m));
        }
        assert!(CombineMethod::from_name("mode").is_err());
        assert!(NormalizationMethod::from_name("robust").is_err());
        assert!(RejectionNormalization::from_name("scale").is_err());
    }

    #[test]
    fn stack_config_defaults_follow_pixinsight() {
        let c = StackConfig::default();
        assert_eq!(c.rejection, RejectionMethod::SigmaClip);
        assert_eq!(c.combine, CombineMethod::Mean);
        assert_eq!(c.normalization, NormalizationMethod::AdditiveScaling);
        assert_eq!(c.rejection_normalization, RejectionNormalization::ScaleOffset);
        assert_eq!(c.sigma_low, 4.0);
        assert_eq!(c.sigma_high, 3.0);
        assert_eq!(c.winsor_cutoff, 5.0);
        assert_eq!((c.percentile_low, c.percentile_high), (0.2, 0.1));
        assert_eq!((c.minmax_low, c.minmax_high), (1, 1));
        assert!(!c.rejection_maps);
        let p = c.rejection_params();
        assert_eq!(p, RejectionParams::default());
    }
}
