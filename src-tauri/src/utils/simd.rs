#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use ndarray::Array2;

#[inline(always)]
fn fast_asinh_scalar(x: f32) -> f32 {
    let abs_x = x.abs();
    if abs_x < 0.5 {
        let x2 = x * x;
        x * (1.0 - x2 * (1.0 / 6.0 - x2 * 3.0 / 40.0))
    } else if abs_x < 4.0 {
        let x2 = abs_x * abs_x;
        let num = abs_x * (1.0 + x2 * 0.1667);
        let den = 1.0 + x2 * 0.2058;
        let sign = if x >= 0.0 { 1.0 } else { -1.0 };
        sign * (num / den + (x2 + 1.0).sqrt().ln())
    } else {
        let sign = if x >= 0.0 { 1.0 } else { -1.0 };
        sign * (2.0 * abs_x).ln()
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn fast_log_avx2(x: __m256) -> __m256 {
    let ln2 = _mm256_set1_ps(std::f32::consts::LN_2);
    let magic = _mm256_set1_epi32(0x3F800000u32 as i32);
    let exp_mask = _mm256_set1_epi32(0x7F800000u32 as i32);

    let xi = _mm256_castps_si256(x);

    let exp_bits = _mm256_and_si256(xi, exp_mask);
    let exp_shifted = _mm256_srli_epi32(exp_bits, 23);
    let exp_f = _mm256_cvtepi32_ps(_mm256_sub_epi32(exp_shifted, _mm256_set1_epi32(127)));

    let mantissa_bits = _mm256_or_si256(_mm256_andnot_si256(exp_mask, xi), magic);
    let m = _mm256_castsi256_ps(mantissa_bits);

    let c0 = _mm256_set1_ps(-1.7417939);
    let c1 = _mm256_set1_ps(2.8212026);
    let c2 = _mm256_set1_ps(-1.4699568);
    let c3 = _mm256_set1_ps(0.44717955);

    let mut p = _mm256_add_ps(_mm256_mul_ps(c3, m), c2);
    p = _mm256_add_ps(_mm256_mul_ps(p, m), c1);
    p = _mm256_add_ps(_mm256_mul_ps(p, m), c0);

    let log2_x = _mm256_add_ps(exp_f, p);
    _mm256_mul_ps(log2_x, ln2)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn fast_sqrt_avx2(x: __m256) -> __m256 {
    _mm256_sqrt_ps(x)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn fast_asinh_avx2(x: __m256) -> __m256 {
    let one = _mm256_set1_ps(1.0);
    let abs_mask = _mm256_castsi256_ps(_mm256_set1_epi32(0x7FFFFFFF));
    let sign_mask = _mm256_castsi256_ps(_mm256_set1_epi32(0x80000000u32 as i32));

    let abs_x = _mm256_and_ps(x, abs_mask);
    let sign = _mm256_and_ps(x, sign_mask);

    let x2 = _mm256_mul_ps(abs_x, abs_x);
    let x2p1 = _mm256_add_ps(x2, one);
    let sqrt_x2p1 = fast_sqrt_avx2(x2p1);
    let inner = _mm256_add_ps(abs_x, sqrt_x2p1);
    let result = fast_log_avx2(inner);

    _mm256_or_ps(result, sign)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn normalize_chunk_avx2(
    input: &[f32],
    output: &mut [f32],
    median: f32,
    inv_sigma_alpha: f32,
    low: f32,
    high: f32,
) {
    let v_median = _mm256_set1_ps(median);
    let v_isa = _mm256_set1_ps(inv_sigma_alpha);
    let v_low = _mm256_set1_ps(low);
    let v_high = _mm256_set1_ps(high);

    let chunks = input.len() / 8;
    let remainder = input.len() % 8;

    for i in 0..chunks {
        let offset = i * 8;
        let data = _mm256_loadu_ps(input.as_ptr().add(offset));
        let is_finite = _mm256_cmp_ps(data, data, _CMP_EQ_OQ);
        let clamped = _mm256_min_ps(_mm256_max_ps(data, v_low), v_high);
        let centered = _mm256_sub_ps(clamped, v_median);
        let scaled = _mm256_mul_ps(centered, v_isa);
        let result = fast_asinh_avx2(scaled);
        let masked = _mm256_and_ps(result, is_finite);
        _mm256_storeu_ps(output.as_mut_ptr().add(offset), masked);
    }

    let base = chunks * 8;
    for i in 0..remainder {
        let v = input[base + i];
        if !v.is_finite() {
            output[base + i] = 0.0;
        } else {
            let clamped = v.clamp(low, high);
            let scaled = inv_sigma_alpha * (clamped - median);
            output[base + i] = fast_asinh_scalar(scaled);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn sum_finite_avx2(data: &[f32]) -> (f64, u32) {
    let chunks = data.len() / 8;
    let remainder = data.len() % 8;

    let mut v_sum_lo = _mm256_setzero_ps();
    let mut v_sum_hi = _mm256_setzero_ps();
    let mut v_count = _mm256_set1_epi32(0);
    let v_zero = _mm256_setzero_ps();

    for i in 0..chunks {
        let offset = i * 8;
        let data_v = _mm256_loadu_ps(data.as_ptr().add(offset));
        let is_finite = _mm256_cmp_ps(data_v, data_v, _CMP_EQ_OQ);
        let is_nonzero = _mm256_cmp_ps(data_v, v_zero, _CMP_NEQ_OQ);
        let mask = _mm256_and_ps(is_finite, is_nonzero);
        let mask_i = _mm256_castps_si256(mask);
        let masked = _mm256_and_ps(data_v, mask);

        if i % 2 == 0 {
            v_sum_lo = _mm256_add_ps(v_sum_lo, masked);
        } else {
            v_sum_hi = _mm256_add_ps(v_sum_hi, masked);
        }

        let ones = _mm256_and_si256(mask_i, _mm256_set1_epi32(1));
        v_count = _mm256_add_epi32(v_count, ones);
    }

    let v_sum = _mm256_add_ps(v_sum_lo, v_sum_hi);

    let mut sum_arr = [0.0f32; 8];
    let mut count_arr = [0i32; 8];
    _mm256_storeu_ps(sum_arr.as_mut_ptr(), v_sum);
    _mm256_storeu_si256(count_arr.as_mut_ptr() as *mut __m256i, v_count);

    let mut total_sum = 0.0f64;
    let mut total_count = 0u32;
    for i in 0..8 {
        total_sum += sum_arr[i] as f64;
        total_count += count_arr[i] as u32;
    }

    let base = chunks * 8;
    for i in 0..remainder {
        let v = data[base + i];
        if v.is_finite() && v != 0.0 {
            total_sum += v as f64;
            total_count += 1;
        }
    }

    (total_sum, total_count)
}

pub fn asinh_normalize_simd(data: &Array2<f32>) -> Array2<f32> {
    let mut finite: Vec<f32> = data.iter().filter(|v| v.is_finite()).copied().collect();

    if finite.is_empty() {
        return data.clone();
    }

    let n = finite.len();
    let mid = n / 2;
    finite.select_nth_unstable_by(mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let median = finite[mid];

    let mut deviations: Vec<f32> = finite.iter().map(|v| (v - median).abs()).collect();
    let dev_mid = deviations.len() / 2;
    deviations.select_nth_unstable_by(dev_mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let sigma = (deviations[dev_mid] * 1.4826).max(1e-10);

    finite.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let low = finite[(n as f64 * 0.01) as usize];
    let high = finite[((n as f64 * 0.999) as usize).min(n - 1)];

    let alpha: f32 = 10.0;
    let inv_sigma_alpha = alpha / sigma;

    let (rows, cols) = data.dim();
    let input_slice = data.as_slice().expect("Array2 must be contiguous");
    let mut output = vec![0.0f32; rows * cols];

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                normalize_chunk_avx2(
                    input_slice,
                    &mut output,
                    median,
                    inv_sigma_alpha,
                    low,
                    high,
                );
            }
            return Array2::from_shape_vec((rows, cols), output).unwrap();
        }
    }

    for (i, &v) in input_slice.iter().enumerate() {
        if !v.is_finite() {
            output[i] = 0.0;
        } else {
            let clamped = v.clamp(low, high);
            let scaled = inv_sigma_alpha * (clamped - median);
            output[i] = fast_asinh_scalar(scaled);
        }
    }

    Array2::from_shape_vec((rows, cols), output).unwrap()
}

pub fn collapse_mean_simd(cube: &ndarray::Array3<f32>) -> Array2<f32> {
    let (depth, rows, cols) = cube.dim();
    let mut result = Array2::<f32>::zeros((rows, cols));

    #[cfg(target_arch = "x86_64")]
    let use_avx2 = is_x86_feature_detected!("avx2");
    #[cfg(not(target_arch = "x86_64"))]
    let use_avx2 = false;

    for y in 0..rows {
        for x in 0..cols {
            let mut col = Vec::with_capacity(depth);
            for z in 0..depth {
                col.push(cube[[z, y, x]]);
            }

            let (sum, count) = if use_avx2 {
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    sum_finite_avx2(&col)
                }
                #[cfg(not(target_arch = "x86_64"))]
                sum_finite_scalar(&col)
            } else {
                sum_finite_scalar(&col)
            };

            result[[y, x]] = if count > 0 {
                (sum / count as f64) as f32
            } else {
                0.0
            };
        }
    }

    result
}

fn sum_finite_scalar(data: &[f32]) -> (f64, u32) {
    let mut sum = 0.0f64;
    let mut count = 0u32;
    for &v in data {
        if v.is_finite() && v != 0.0 {
            sum += v as f64;
            count += 1;
        }
    }
    (sum, count)
}

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

    for i in 0..chunks {
        let offset = i * 8;
        let v = _mm256_loadu_ps(data.as_ptr().add(offset));
        let is_finite = _mm256_cmp_ps(v, v, _CMP_EQ_OQ);
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
