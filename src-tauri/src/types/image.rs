#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageStats {
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub mad: f64,
    pub sigma: f64,
    pub mean: f64,
    pub valid_count: u64,
}

impl Default for ImageStats {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 0.0,
            median: 0.0,
            mad: 0.0,
            sigma: 0.0,
            mean: 0.0,
            valid_count: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Histogram {
    pub bins: Vec<u32>,
    pub bin_edges: Vec<f64>,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct StfParams {
    pub shadow: f64,
    pub midtone: f64,
    pub highlight: f64,
}

impl Default for StfParams {
    fn default() -> Self {
        Self {
            shadow: 0.0,
            midtone: 0.5,
            highlight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct AutoStfConfig {
    pub target_bg: f64,
    pub shadow_k: f64,
}

impl Default for AutoStfConfig {
    fn default() -> Self {
        Self {
            target_bg: 0.25,
            shadow_k: -2.8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScnrMethod {
    AverageNeutral,
    MaximumNeutral,
}

impl Default for ScnrMethod {
    fn default() -> Self {
        Self::AverageNeutral
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ScnrConfig {
    pub method: ScnrMethod,
    pub amount: f32,
    pub preserve_luminance: bool,
}

impl Default for ScnrConfig {
    fn default() -> Self {
        Self {
            method: ScnrMethod::AverageNeutral,
            amount: 1.0,
            preserve_luminance: false,
        }
    }
}
