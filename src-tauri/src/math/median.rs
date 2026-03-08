use std::cmp::Ordering;

#[inline]
pub fn f32_cmp(a: &f32, b: &f32) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

#[inline]
pub fn f64_cmp(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

pub fn exact_median_mut(data: &mut [f32]) -> f64 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let mid = n / 2;
    data.select_nth_unstable_by(mid, f32_cmp);
    if n % 2 == 0 {
        let right = data[mid] as f64;
        let left = data[..mid]
            .iter()
            .copied()
            .fold(f32::MIN, f32::max) as f64;
        (left + right) / 2.0
    } else {
        data[mid] as f64
    }
}

pub fn median_f32_mut(data: &mut [f32]) -> f32 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let mid = n / 2;
    data.select_nth_unstable_by(mid, f32_cmp);
    if n % 2 == 0 {
        let right = data[mid];
        let left = data[..mid]
            .iter()
            .copied()
            .fold(f32::MIN, f32::max);
        (left + right) / 2.0
    } else {
        data[mid]
    }
}

pub fn exact_mad_mut(data: &mut [f32], median: f32) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    for v in data.iter_mut() {
        *v = (*v - median).abs();
    }
    median_f32_mut(data)
}

pub fn exact_median_f64(data: &[f64]) -> f64 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let mut buf: Vec<f64> = data.to_vec();
    let mid = n / 2;
    buf.select_nth_unstable_by(mid, f64_cmp);
    if n % 2 == 0 {
        let right = buf[mid];
        let left = buf[..mid]
            .iter()
            .copied()
            .fold(f64::MIN, f64::max);
        (left + right) / 2.0
    } else {
        buf[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_odd() {
        let mut vals = vec![5.0f32, 1.0, 3.0, 2.0, 4.0];
        let m = exact_median_mut(&mut vals);
        assert!((m - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_even() {
        let mut vals = vec![1.0f32, 2.0, 3.0, 4.0];
        let m = exact_median_mut(&mut vals);
        assert!((m - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_empty() {
        let mut vals: Vec<f32> = vec![];
        assert_eq!(exact_median_mut(&mut vals), 0.0);
    }

    #[test]
    fn test_f64_odd() {
        let vals = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        let m = exact_median_f64(&vals);
        assert!((m - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_median_f32_mut() {
        let mut vals = vec![5.0f32, 1.0, 3.0, 2.0, 4.0];
        let m = median_f32_mut(&mut vals);
        assert!((m - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_exact_mad_mut() {
        let mut vals = vec![1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mad = exact_mad_mut(&mut vals, 3.0);
        assert!((mad - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_f32_cmp_nan() {
        assert_eq!(f32_cmp(&f32::NAN, &1.0), Ordering::Equal);
    }
}
