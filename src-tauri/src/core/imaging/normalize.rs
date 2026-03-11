use ndarray::Array2;
use crate::math::simd::asinh_normalize_simd;

pub fn robust_asinh_preview(data: &Array2<f32>) -> Array2<f32> {
    asinh_normalize_simd(data)
}

#[deprecated(note = "use robust_asinh_preview or core::imaging::stretch::arcsinh_stretch")]
pub fn asinh_normalize(data: &Array2<f32>) -> Array2<f32> {
    asinh_normalize_simd(data)
}
