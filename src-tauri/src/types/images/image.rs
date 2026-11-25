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
    #[serde(alias = "average")]
    AverageNeutral,
    #[serde(alias = "maximum")]
    MaximumNeutral,
}

impl Default for ScnrMethod {
    fn default() -> Self {
        Self::AverageNeutral
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone)]
pub struct IntPlane {
    pub bits: ndarray::Array2<u32>,
    pub signed: bool,
}

impl IntPlane {
    pub fn value_at(&self, y: usize, x: usize) -> i64 {
        let raw = self.bits[[y, x]];
        if self.signed {
            raw as i32 as i64
        } else {
            raw as i64
        }
    }

    pub fn byte_size(&self) -> usize {
        let (rows, cols) = self.bits.dim();
        rows.saturating_mul(cols).saturating_mul(std::mem::size_of::<u32>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_plane_value_at_respects_signedness() {
        let bits = ndarray::Array2::from_shape_vec((1, 2), vec![0xFFFF_FFFFu32, 0x8000_0001]).unwrap();
        let unsigned = IntPlane { bits: bits.clone(), signed: false };
        assert_eq!(unsigned.value_at(0, 0), 4294967295);
        assert_eq!(unsigned.value_at(0, 1), 2147483649);
        let signed = IntPlane { bits, signed: true };
        assert_eq!(signed.value_at(0, 0), -1);
        assert_eq!(signed.value_at(0, 1), -2147483647);
    }

    #[test]
    fn int_plane_byte_size_is_four_per_pixel() {
        let p = IntPlane { bits: ndarray::Array2::zeros((3, 5)), signed: false };
        assert_eq!(p.byte_size(), 60);
    }
}
