#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

pub fn find_minmax_simd(data: &[f32]) -> (f32, f32) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { find_minmax_avx2(data) };
        }
    }

    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for &v in data {
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    (min, max)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn find_minmax_avx2(data: &[f32]) -> (f32, f32) {
    let chunks = data.len() / 8;
    let remainder = data.len() % 8;

    let mut v_min = _mm256_set1_ps(f32::MAX);
    let mut v_max = _mm256_set1_ps(f32::MIN);
    let abs_mask = _mm256_castsi256_ps(_mm256_set1_epi32(0x7FFFFFFF));
    let v_inf = _mm256_set1_ps(f32::INFINITY);

    for i in 0..chunks {
        let offset = i * 8;
        let v = _mm256_loadu_ps(data.as_ptr().add(offset));
        let is_finite = _mm256_cmp_ps(_mm256_and_ps(v, abs_mask), v_inf, _CMP_LT_OQ);
        let masked_for_min = _mm256_blendv_ps(_mm256_set1_ps(f32::MAX), v, is_finite);
        let masked_for_max = _mm256_blendv_ps(_mm256_set1_ps(f32::MIN), v, is_finite);
        v_min = _mm256_min_ps(v_min, masked_for_min);
        v_max = _mm256_max_ps(v_max, masked_for_max);
    }

    let mut min_arr = [0.0f32; 8];
    let mut max_arr = [0.0f32; 8];
    _mm256_storeu_ps(min_arr.as_mut_ptr(), v_min);
    _mm256_storeu_ps(max_arr.as_mut_ptr(), v_max);

    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for i in 0..8 {
        min = min.min(min_arr[i]);
        max = max.max(max_arr[i]);
    }

    let base = chunks * 8;
    for i in 0..remainder {
        let v = data[base + i];
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }

    (min, max)
}
