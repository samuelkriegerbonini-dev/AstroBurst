use ndarray::Array2;
use rayon::prelude::*;

pub fn area_downsample(img: &Array2<f32>, out_rows: usize, out_cols: usize) -> Array2<f32> {
    let (in_rows, in_cols) = img.dim();

    if in_rows == out_rows && in_cols == out_cols {
        return img.clone();
    }

    let scale_y = in_rows as f64 / out_rows as f64;
    let scale_x = in_cols as f64 / out_cols as f64;
    let src = img.as_slice().expect("contiguous");

    let mut buf = vec![0.0f32; out_rows * out_cols];
    buf.par_chunks_mut(out_cols).enumerate().for_each(|(oy, row)| {
        let y0 = (oy as f64 * scale_y).floor() as usize;
        let y1 = (((oy + 1) as f64 * scale_y).ceil() as usize).min(in_rows);
        for ox in 0..out_cols {
            let x0 = (ox as f64 * scale_x).floor() as usize;
            let x1 = (((ox + 1) as f64 * scale_x).ceil() as usize).min(in_cols);

            let mut sum = 0.0f64;
            let mut count = 0u32;

            for y in y0..y1 {
                let base = y * in_cols;
                for x in x0..x1 {
                    let v = src[base + x];
                    if v.is_finite() {
                        sum += v as f64;
                        count += 1;
                    }
                }
            }

            row[ox] = if count > 0 { (sum / count as f64) as f32 } else { 0.0 };
        }
    });

    Array2::from_shape_vec((out_rows, out_cols), buf).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity() {
        let img = Array2::from_shape_fn((64, 64), |(y, x)| (y * 64 + x) as f32);
        let result = area_downsample(&img, 64, 64);
        assert_eq!(result.dim(), (64, 64));
        assert!((result[[0, 0]] - img[[0, 0]]).abs() < 1e-6);
    }

    #[test]
    fn test_halve() {
        let img = Array2::from_shape_fn((4, 4), |(y, x)| (y * 4 + x) as f32);
        let result = area_downsample(&img, 2, 2);
        assert_eq!(result.dim(), (2, 2));
        let expected_00 = (0.0 + 1.0 + 4.0 + 5.0) / 4.0;
        assert!((result[[0, 0]] - expected_00).abs() < 1e-4);
    }
}
