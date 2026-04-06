use ndarray::Array2;
use rayon::prelude::*;

use crate::math::subpixel::quadratic_3pt;

pub fn compute_offset(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    search_radius: i32,
) -> (i32, i32) {
    let (rows, cols) = reference.dim();
    if target.dim() != (rows, cols) {
        return (0, 0);
    }

    let cy = rows / 2;
    let cx = cols / 2;
    let region = rows.min(cols).min(256) / 2;
    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);

    let shifts: Vec<(i32, i32)> = (-search_radius..=search_radius)
        .flat_map(|dy| (-search_radius..=search_radius).map(move |dx| (dy, dx)))
        .collect();

    let (best_dy, best_dx, _) = shifts
        .par_iter()
        .map(|&(dy, dx)| {
            let mut sum_prod = 0.0f64;
            let mut sum_r2 = 0.0f64;
            let mut sum_t2 = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 {
                    continue;
                }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 {
                        continue;
                    }
                    let r = reference[[y, x]] as f64;
                    let t = target[[ty as usize, tx as usize]] as f64;
                    if r.is_finite() && t.is_finite() {
                        sum_prod += r * t;
                        sum_r2 += r * r;
                        sum_t2 += t * t;
                        count += 1;
                    }
                }
            }

            let score = if count > 0 {
                let denom = (sum_r2 * sum_t2).sqrt();
                if denom > 1e-10 { sum_prod / denom } else { 0.0 }
            } else {
                f64::NEG_INFINITY
            };
            (dy, dx, score)
        })
        .reduce(
            || (0i32, 0i32, f64::NEG_INFINITY),
            |a, b| if b.2 > a.2 { b } else { a },
        );

    (best_dy, best_dx)
}

pub fn shift_image(image: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let mut shifted = Array2::<f32>::from_elem((rows, cols), f32::NAN);

    shifted
        .axis_iter_mut(ndarray::Axis(0))
        .into_par_iter()
        .enumerate()
        .for_each(|(y, mut row)| {
            let sy = y as i32 - dy;
            if sy < 0 || sy >= rows as i32 {
                return;
            }
            for x in 0..cols {
                let sx = x as i32 - dx;
                if sx < 0 || sx >= cols as i32 {
                    continue;
                }
                row[x] = image[[sy as usize, sx as usize]];
            }
        });

    shifted
}

pub fn compute_subpixel_offset(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    search_radius: i32,
) -> (f64, f64) {
    let (rows, cols) = reference.dim();
    if target.dim() != (rows, cols) {
        return (0.0, 0.0);
    }

    let cy = rows / 2;
    let cx = cols / 2;
    let region = rows.min(cols).min(256) / 2;
    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);

    let diameter = (2 * search_radius + 1) as usize;

    let shifts: Vec<(i32, i32)> = (-search_radius..=search_radius)
        .flat_map(|dy| (-search_radius..=search_radius).map(move |dx| (dy, dx)))
        .collect();

    let scores_flat: Vec<f64> = shifts
        .par_iter()
        .map(|&(dy, dx)| {
            let mut r_sum = 0.0f64;
            let mut t_sum = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 { continue; }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 { continue; }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && tv.is_finite() {
                        r_sum += rv;
                        t_sum += tv;
                        count += 1;
                    }
                }
            }

            if count < 10 {
                return f64::NEG_INFINITY;
            }

            let r_mean = r_sum / count as f64;
            let t_mean = t_sum / count as f64;
            let mut num = 0.0f64;
            let mut r_var = 0.0f64;
            let mut t_var = 0.0f64;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 { continue; }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 { continue; }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && tv.is_finite() {
                        let rd = rv - r_mean;
                        let td = tv - t_mean;
                        num += rd * td;
                        r_var += rd * rd;
                        t_var += td * td;
                    }
                }
            }

            if r_var > 0.0 && t_var > 0.0 {
                num / (r_var * t_var).sqrt()
            } else {
                f64::NEG_INFINITY
            }
        })
        .collect();

    let grid_idx = |dy: i32, dx: i32| -> usize {
        (dy + search_radius) as usize * diameter + (dx + search_radius) as usize
    };

    let mut best_score = f64::NEG_INFINITY;
    let mut by = 0i32;
    let mut bx = 0i32;
    for &(dy, dx) in &shifts {
        let s = scores_flat[grid_idx(dy, dx)];
        if s > best_score {
            best_score = s;
            by = dy;
            bx = dx;
        }
    }

    if best_score <= f64::NEG_INFINITY {
        return (by as f64, bx as f64);
    }

    let sub_dy = quadratic_peak_grid(
        &scores_flat, diameter, by, bx, search_radius, true,
    ).unwrap_or(by as f64);
    let sub_dx = quadratic_peak_grid(
        &scores_flat, diameter, by, bx, search_radius, false,
    ).unwrap_or(bx as f64);

    (sub_dy, sub_dx)
}

fn quadratic_peak_grid(
    grid: &[f64],
    diameter: usize,
    cy: i32,
    cx: i32,
    search_radius: i32,
    axis_y: bool,
) -> Option<f64> {
    let idx = |dy: i32, dx: i32| -> usize {
        (dy + search_radius) as usize * diameter + (dx + search_radius) as usize
    };

    let c_score = grid[idx(cy, cx)];
    if c_score.is_infinite() {
        return Some(if axis_y { cy as f64 } else { cx as f64 });
    }

    let (prev_score, next_score, center) = if axis_y {
        if cy <= -search_radius || cy >= search_radius {
            return Some(cy as f64);
        }
        (grid[idx(cy - 1, cx)], grid[idx(cy + 1, cx)], cy as f64)
    } else {
        if cx <= -search_radius || cx >= search_radius {
            return Some(cx as f64);
        }
        (grid[idx(cy, cx - 1)], grid[idx(cy, cx + 1)], cx as f64)
    };

    if prev_score.is_infinite() || next_score.is_infinite() {
        return Some(center);
    }

    let offset = quadratic_3pt(prev_score, c_score, next_score);
    Some(center + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_offset_no_shift() {
        let img = Array2::from_shape_vec(
            (64, 64),
            (0..4096)
                .map(|i| (i as f32).sin() * 100.0 + 500.0)
                .collect(),
        )
            .unwrap();

        let (dy, dx) = compute_offset(&img, &img, 10);
        assert_eq!(dy, 0);
        assert_eq!(dx, 0);
    }

    #[test]
    fn test_compute_offset_known_shift() {
        let base = Array2::from_shape_vec(
            (64, 64),
            (0..4096)
                .map(|i| {
                    let y = i / 64;
                    let x = i % 64;
                    ((y as f32 * 0.1).sin() * (x as f32 * 0.1).cos() * 1000.0) + 500.0
                })
                .collect(),
        )
            .unwrap();

        let shifted = shift_image(&base, 3, 5);
        let (dy, dx) = compute_offset(&base, &shifted, 10);
        assert_eq!(dy, 3);
        assert_eq!(dx, 5);
    }

    #[test]
    fn test_shift_image() {
        let img = Array2::from_shape_vec(
            (4, 4),
            vec![
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0,
                14.0, 15.0, 16.0,
            ],
        )
            .unwrap();

        let shifted = shift_image(&img, 1, 1);
        assert!((shifted[[1, 1]] - 1.0).abs() < 1e-6);
        assert!((shifted[[2, 2]] - 6.0).abs() < 1e-6);
        assert!(shifted[[0, 0]].is_nan());
    }
}
