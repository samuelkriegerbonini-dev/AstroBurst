use ndarray::Array2;
use crate::utils::simd::asinh_normalize_simd;

pub fn asinh_normalize(data: &Array2<f32>) -> Array2<f32> {
    asinh_normalize_simd(data)
}
