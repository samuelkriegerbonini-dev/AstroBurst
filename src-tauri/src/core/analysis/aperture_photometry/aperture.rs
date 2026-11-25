use ndarray::Array2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AperturePixel {
    pub y: usize,
    pub x: usize,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ApertureSum {
    pub sum: f64,
    pub weight: f64,
    pub n_finite: u32,
    pub n_masked: u32,
    pub n_nonfinite: u32,
}

const HALF_PIXEL_DIAGONAL: f64 = std::f64::consts::FRAC_1_SQRT_2;
const HALF_PIXEL: f64 = 0.5;
pub const WHOLE_PIXEL_SUBSAMPLES: u8 = 1;

fn area_under_arc(r: f64, x: f64) -> f64 {
    let x = x.clamp(-r, r);
    0.5 * (x * (r * r - x * x).max(0.0).sqrt() + r * r * (x / r).asin())
}

fn area_under_capped_arc(r: f64, x0: f64, x1: f64, cap: f64) -> f64 {
    let (from, to) = (x0.max(-r), x1.min(r));
    if from >= to || cap <= 0.0 {
        return 0.0;
    }
    let arc = |a: f64, b: f64| if b > a { area_under_arc(r, b) - area_under_arc(r, a) } else { 0.0 };
    if cap >= r {
        return arc(from, to);
    }
    let flat = (r * r - cap * cap).sqrt();
    let capped_width = (to.min(flat) - from.max(-flat)).max(0.0);
    arc(from, to.min(-flat)) + cap * capped_width + arc(from.max(flat), to)
}

fn exact_circle_fraction(dx: f64, dy: f64, r: f64) -> f64 {
    if !(r > 0.0) {
        return 0.0;
    }
    let (x0, x1) = (dx - HALF_PIXEL, dx + HALF_PIXEL);
    let signed_area_below = |y: f64| y.signum() * area_under_capped_arc(r, x0, x1, y.abs());
    (signed_area_below(dy + HALF_PIXEL) - signed_area_below(dy - HALF_PIXEL)).clamp(0.0, 1.0)
}

fn circle_fraction(px: f64, py: f64, cx: f64, cy: f64, r: f64, whole_pixels: bool) -> f64 {
    let (dx, dy) = (px - cx, py - cy);
    let d = dx.hypot(dy);
    if d + HALF_PIXEL_DIAGONAL <= r {
        return 1.0;
    }
    if d - HALF_PIXEL_DIAGONAL > r {
        return 0.0;
    }
    if whole_pixels {
        return if dx * dx + dy * dy <= r * r { 1.0 } else { 0.0 };
    }
    exact_circle_fraction(dx, dy, r)
}

fn pixel_span(centre: f64, reach: f64, len: usize) -> Option<(usize, usize)> {
    if len == 0 || !centre.is_finite() || !reach.is_finite() || reach < 0.0 {
        return None;
    }
    let lo = (centre - reach - 0.5).ceil().max(0.0);
    let hi = (centre + reach + 0.5).floor().min((len - 1) as f64);
    if lo > hi {
        return None;
    }
    Some((lo as usize, hi as usize))
}

fn collect_pixels(
    rows: usize,
    cols: usize,
    cx: f64,
    cy: f64,
    reach: f64,
    fraction: impl Fn(f64, f64) -> f64,
) -> Vec<AperturePixel> {
    let (Some((y0, y1)), Some((x0, x1))) =
        (pixel_span(cy, reach, rows), pixel_span(cx, reach, cols))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let weight = fraction(x as f64, y as f64);
            if weight > 0.0 {
                out.push(AperturePixel {
                    y,
                    x,
                    weight: weight as f32,
                });
            }
        }
    }
    out
}

pub fn circular_aperture(
    rows: usize,
    cols: usize,
    cx: f64,
    cy: f64,
    r: f64,
    subsamples: u8,
) -> Vec<AperturePixel> {
    let whole_pixels = subsamples <= WHOLE_PIXEL_SUBSAMPLES;
    collect_pixels(rows, cols, cx, cy, r, |px, py| circle_fraction(px, py, cx, cy, r, whole_pixels))
}

pub fn annulus(
    rows: usize,
    cols: usize,
    cx: f64,
    cy: f64,
    r_in: f64,
    r_out: f64,
    subsamples: u8,
) -> Vec<AperturePixel> {
    if !r_in.is_finite() || r_in >= r_out {
        return Vec::new();
    }
    let whole_pixels = subsamples <= WHOLE_PIXEL_SUBSAMPLES;
    collect_pixels(rows, cols, cx, cy, r_out, |px, py| {
        circle_fraction(px, py, cx, cy, r_out, whole_pixels) - circle_fraction(px, py, cx, cy, r_in, whole_pixels)
    })
}

pub fn total_weight(pixels: &[AperturePixel]) -> f64 {
    pixels.iter().map(|p| p.weight as f64).sum()
}

pub fn weighted_sum(
    image: &Array2<f32>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
) -> ApertureSum {
    let mask = excluded.filter(|m| m.dim() == image.dim());
    let mut acc = ApertureSum::default();
    for p in pixels {
        let Some(&value) = image.get((p.y, p.x)) else {
            continue;
        };
        if mask.is_some_and(|m| m[[p.y, p.x]] != 0) {
            acc.n_masked += 1;
        } else if !value.is_finite() {
            acc.n_nonfinite += 1;
        } else {
            let weight = p.weight as f64;
            acc.sum += weight * value as f64;
            acc.weight += weight;
            acc.n_finite += 1;
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::RegionShape;
    use std::collections::HashMap;

    fn index_image(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32)
    }

    fn brute_centre_in_circle(
        rows: usize,
        cols: usize,
        cx: f64,
        cy: f64,
        r: f64,
    ) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                if dx * dx + dy * dy <= r * r {
                    out.push((y, x));
                }
            }
        }
        out
    }

    fn exhaustive_circle(
        rows: usize,
        cols: usize,
        cx: f64,
        cy: f64,
        r: f64,
        subsamples: u8,
    ) -> Vec<AperturePixel> {
        let mut out = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                let weight = if subsamples <= 1 {
                    f64::from(u8::from(dx * dx + dy * dy <= r * r))
                } else {
                    exact_circle_fraction(dx, dy, r)
                };
                if weight > 0.0 {
                    out.push(AperturePixel { y, x, weight: weight as f32 });
                }
            }
        }
        out
    }

    fn indices(pixels: &[AperturePixel]) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = pixels.iter().map(|p| (p.y, p.x)).collect();
        v.sort();
        v
    }

    fn weight_map(pixels: &[AperturePixel]) -> HashMap<(usize, usize), f64> {
        pixels
            .iter()
            .map(|p| ((p.y, p.x), p.weight as f64))
            .collect()
    }

    fn photutils_exact_fixture() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("aperture")
            .join("photutils_exact_weights.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn a_circle_inside_the_image_weighs_its_area() {
        for (cx, cy, r) in [(32.0, 32.0, 2.0), (32.3, 31.6, 3.0), (20.71, 40.13, 5.4), (31.5, 31.5, 10.0)] {
            let total = total_weight(&circular_aperture(64, 64, cx, cy, r, 5));
            let area = std::f64::consts::PI * r * r;
            assert!((total - area).abs() < 1e-6, "centre ({cx}, {cy}) radius {r}: weight {total} against the area {area}");
        }
    }

    #[test]
    fn weights_are_the_exact_overlap_of_photutils() {
        let fixture = photutils_exact_fixture();
        assert_eq!(fixture["generator"], "scripts/oracle/aperture_exact_goldens.py");
        assert_eq!(fixture["versions"]["photutils"], "3.1.1");
        let cases = fixture["weights"].as_array().unwrap();
        assert_eq!(cases.len(), 12);
        for case in cases {
            let number = |key: &str| case[key].as_f64().unwrap();
            let (rows, cols) = (number("rows") as usize, number("cols") as usize);
            let (x, y, r) = (number("x"), number("y"), number("r"));
            let mine = weight_map(&circular_aperture(rows, cols, x, y, r, 5));
            let theirs: HashMap<(usize, usize), f64> = case["pixels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| ((entry[0].as_u64().unwrap() as usize, entry[1].as_u64().unwrap() as usize), entry[2].as_f64().unwrap()))
                .collect();
            let mut worst = 0.0f64;
            for key in mine.keys().chain(theirs.keys()) {
                let gap = (mine.get(key).copied().unwrap_or(0.0) - theirs.get(key).copied().unwrap_or(0.0)).abs();
                worst = worst.max(gap);
            }
            assert!(worst < 1e-6, "({x}, {y}) radius {r}: a weight is {worst} away from photutils");
            let total: f64 = mine.values().sum();
            assert!((total - number("total")).abs() < 1e-5, "({x}, {y}) radius {r}: total {total} against {}", number("total"));
        }
        for case in fixture["annuli"].as_array().unwrap() {
            let number = |key: &str| case[key].as_f64().unwrap();
            let ring = annulus(number("rows") as usize, number("cols") as usize, number("x"), number("y"), number("r_in"), number("r_out"), 5);
            assert!((total_weight(&ring) - number("total")).abs() < 1e-5, "annulus total {} against {}", total_weight(&ring), number("total"));
        }
    }

    #[test]
    fn subsamples_one_reproduces_centre_in_circle_rule() {
        let px = circular_aperture(32, 32, 10.0, 10.0, 5.0, 1);
        assert_eq!(px.len(), 81);
        assert!(px.iter().all(|p| p.weight == 1.0));
        assert_eq!(
            indices(&px),
            brute_centre_in_circle(32, 32, 10.0, 10.0, 5.0)
        );

        let px = circular_aperture(32, 32, 10.0, 10.0, 2.0, 1);
        assert_eq!(px.len(), 13);

        let px = circular_aperture(40, 40, 17.3, 21.8, 3.7, 1);
        assert_eq!(
            indices(&px),
            brute_centre_in_circle(40, 40, 17.3, 21.8, 3.7)
        );
        assert!(px.iter().all(|p| p.weight == 1.0));
    }

    #[test]
    fn subsamples_one_circle_matches_region_circle_pixel_set_in_index_space_no_ds9_offset() {
        let arr = index_image(32, 32);
        let region = RegionShape::Circle {
            x: 10.0,
            y: 10.0,
            r: 5.0,
        };
        let mv = region.masked_values(&arr, None);
        assert_eq!(mv.n_inside, 81);
        let mut region_ids: Vec<u32> = mv.values.iter().map(|v| *v as u32).collect();
        region_ids.sort();

        let px = circular_aperture(32, 32, 10.0, 10.0, 5.0, 1);
        let mut helper_ids: Vec<u32> = px.iter().map(|p| (p.y * 32 + p.x) as u32).collect();
        helper_ids.sort();
        assert_eq!(helper_ids, region_ids);

        let shifted = region.to_ds9_image().masked_values(&arr, None);
        let mut shifted_ids: Vec<u32> = shifted.values.iter().map(|v| *v as u32).collect();
        shifted_ids.sort();
        assert_ne!(shifted_ids, helper_ids);
    }

    #[test]
    fn annulus_subsamples_one_matches_region_annulus_pixel_set() {
        let arr = index_image(32, 32);
        let region = RegionShape::Annulus {
            x: 10.0,
            y: 10.0,
            r_inner: 2.0,
            r_outer: 5.0,
        };
        let mv = region.masked_values(&arr, None);
        assert_eq!(mv.n_inside, 68);
        let mut region_ids: Vec<u32> = mv.values.iter().map(|v| *v as u32).collect();
        region_ids.sort();

        let px = annulus(32, 32, 10.0, 10.0, 2.0, 5.0, 1);
        let mut helper_ids: Vec<u32> = px.iter().map(|p| (p.y * 32 + p.x) as u32).collect();
        helper_ids.sort();
        assert_eq!(helper_ids, region_ids);
        assert!(px.iter().all(|p| p.weight == 1.0));
    }

    #[test]
    fn fully_contained_circle_weight_sums_to_area_within_one_percent() {
        let area = std::f64::consts::PI * 100.0;
        for (cx, cy) in [(20.0, 20.0), (20.3, 19.6), (25.5, 30.5)] {
            let px = circular_aperture(64, 64, cx, cy, 10.0, 5);
            let total = total_weight(&px);
            let rel = (total - area).abs() / area;
            assert!(
                rel < 0.01,
                "centre ({cx},{cy}) total {total} vs {area} ({rel:.4})"
            );
        }
    }

    #[test]
    fn small_aperture_weight_sums_closer_to_area_than_centre_rule() {
        let r = 3.0;
        let area = std::f64::consts::PI * r * r;
        let fine = total_weight(&circular_aperture(64, 64, 32.3, 31.6, r, 5));
        let coarse = total_weight(&circular_aperture(64, 64, 32.3, 31.6, r, 1));
        assert!(
            (fine - area).abs() < (coarse - area).abs(),
            "fine {fine} coarse {coarse} area {area}"
        );
        assert!((fine - area).abs() / area < 0.02, "fine {fine} vs {area}");
    }

    #[test]
    fn annulus_weight_equals_difference_of_circles() {
        for subsamples in [1u8, 3, 5] {
            for (cx, cy) in [(20.0, 20.0), (21.4, 18.7)] {
                let outer = circular_aperture(64, 64, cx, cy, 9.0, subsamples);
                let inner = circular_aperture(64, 64, cx, cy, 4.0, subsamples);
                let ann = annulus(64, 64, cx, cy, 4.0, 9.0, subsamples);
                let expected = total_weight(&outer) - total_weight(&inner);
                let got = total_weight(&ann);
                assert!(
                    (got - expected).abs() < 1e-6,
                    "s={subsamples} ({cx},{cy}) {got} vs {expected}"
                );

                let inner_map = weight_map(&inner);
                let ann_map = weight_map(&ann);
                for p in &outer {
                    let key = (p.y, p.x);
                    let diff = p.weight as f64 - inner_map.get(&key).copied().unwrap_or(0.0);
                    let got = ann_map.get(&key).copied().unwrap_or(0.0);
                    assert!((got - diff).abs() < 1e-6, "pixel {key:?} {got} vs {diff}");
                }
                assert!(ann_map
                    .keys()
                    .all(|k| outer.iter().any(|p| (p.y, p.x) == *k)));
            }
        }
    }

    #[test]
    fn annulus_with_inner_radius_not_below_outer_is_empty() {
        assert!(annulus(32, 32, 10.0, 10.0, 5.0, 5.0, 5).is_empty());
        assert!(annulus(32, 32, 10.0, 10.0, 6.0, 5.0, 3).is_empty());
    }

    #[test]
    fn aperture_is_clipped_to_image_bounds() {
        let px = circular_aperture(32, 32, 1.0, 1.0, 5.0, 1);
        assert_eq!(indices(&px), brute_centre_in_circle(32, 32, 1.0, 1.0, 5.0));
        assert!(px.iter().all(|p| p.x <= 6 && p.y <= 6));

        let px = circular_aperture(32, 32, 30.6, 0.4, 4.0, 5);
        assert!(!px.is_empty());
        assert!(px.iter().all(|p| p.x < 32 && p.y < 32));
        let full = total_weight(&circular_aperture(64, 64, 30.6, 20.4, 4.0, 5));
        assert!(total_weight(&px) < full);

        assert!(circular_aperture(32, 32, -20.0, -20.0, 5.0, 5).is_empty());
        assert!(circular_aperture(32, 32, 100.0, 10.0, 5.0, 5).is_empty());
        assert!(circular_aperture(0, 0, 0.0, 0.0, 5.0, 5).is_empty());
        assert!(circular_aperture(32, 32, 10.0, 10.0, -1.0, 5).is_empty());
        assert!(circular_aperture(32, 32, f64::NAN, 10.0, 3.0, 5).is_empty());
    }

    #[test]
    fn weights_are_strictly_positive_and_at_most_one() {
        for subsamples in [1u8, 2, 5, 8] {
            let px = circular_aperture(48, 48, 23.7, 24.2, 6.3, subsamples);
            assert!(!px.is_empty());
            assert!(px.iter().all(|p| p.weight > 0.0 && p.weight <= 1.0));
        }
    }

    #[test]
    fn the_inside_and_outside_shortcuts_match_the_overlap_computed_for_every_pixel() {
        let cases = [
            (10.0, 10.0, 5.0, 5u8),
            (17.3, 21.8, 3.7, 4),
            (0.5, 0.5, 2.0, 3),
            (30.0, 12.25, 8.0, 7),
            (15.0, 15.0, 0.4, 5),
            (15.5, 15.5, 1.0, 2),
        ];
        for (cx, cy, r, s) in cases {
            let fast = weight_map(&circular_aperture(32, 32, cx, cy, r, s));
            let slow = weight_map(&exhaustive_circle(32, 32, cx, cy, r, s));
            for key in fast.keys().chain(slow.keys()) {
                let gap = (fast.get(key).copied().unwrap_or(0.0) - slow.get(key).copied().unwrap_or(0.0)).abs();
                assert!(gap < 1e-6, "case ({cx},{cy},{r},{s}) pixel {key:?}: {gap}");
            }
        }
    }

    #[test]
    fn fractional_centre_shifts_boundary_weights_toward_the_centre() {
        let px = circular_aperture(32, 32, 10.25, 10.0, 3.0, 4);
        let map = weight_map(&px);
        let right = map.get(&(10, 13)).copied().unwrap_or(0.0);
        let left = map.get(&(10, 7)).copied().unwrap_or(0.0);
        assert!(right > left, "right {right} left {left}");
        assert!(right > 0.0);
    }

    #[test]
    fn subsamples_zero_behaves_as_one() {
        let zero = circular_aperture(32, 32, 10.3, 9.8, 4.0, 0);
        let one = circular_aperture(32, 32, 10.3, 9.8, 4.0, 1);
        assert_eq!(zero, one);
    }

    #[test]
    fn weighted_sum_on_flat_image_equals_level_times_total_weight() {
        let img = Array2::from_elem((32, 32), 2.5f32);
        let px = circular_aperture(32, 32, 15.2, 16.7, 4.0, 5);
        let s = weighted_sum(&img, &px, None);
        let total = total_weight(&px);
        assert!((s.sum - 2.5 * total).abs() < 1e-9);
        assert!((s.weight - total).abs() < 1e-12);
        assert_eq!(s.n_finite as usize, px.len());
        assert_eq!(s.n_masked, 0);
        assert_eq!(s.n_nonfinite, 0);
    }

    #[test]
    fn weighted_sum_counts_masked_and_nonfinite_pixels() {
        let mut img = index_image(32, 32);
        img[[16, 16]] = f32::NAN;
        let mut mask = Array2::<u8>::zeros((32, 32));
        mask[[16, 17]] = 1;
        let px = circular_aperture(32, 32, 16.0, 16.0, 3.0, 5);
        let s = weighted_sum(&img, &px, Some(&mask));
        assert_eq!(s.n_nonfinite, 1);
        assert_eq!(s.n_masked, 1);
        assert_eq!(s.n_finite as usize, px.len() - 2);

        let mut expected_sum = 0.0f64;
        let mut expected_weight = 0.0f64;
        for p in &px {
            if (p.y, p.x) == (16, 16) || (p.y, p.x) == (16, 17) {
                continue;
            }
            expected_sum += p.weight as f64 * img[[p.y, p.x]] as f64;
            expected_weight += p.weight as f64;
        }
        assert!((s.sum - expected_sum).abs() < 1e-6);
        assert!((s.weight - expected_weight).abs() < 1e-9);

        let wrong = Array2::<u8>::ones((8, 8));
        let ignored = weighted_sum(&img, &px, Some(&wrong));
        assert_eq!(ignored.n_masked, 0);
        assert_eq!(ignored.n_nonfinite, 1);
        assert_eq!(ignored.n_finite as usize, px.len() - 1);

        let masked_nan = weighted_sum(&img, &px, Some(&Array2::<u8>::ones((32, 32))));
        assert_eq!(masked_nan.n_masked as usize, px.len());
        assert_eq!(masked_nan.n_nonfinite, 0);
        assert_eq!(masked_nan.n_finite, 0);
        assert_eq!(masked_nan.weight, 0.0);
    }

    #[test]
    fn weighted_sum_skips_pixels_outside_the_image() {
        let img = Array2::from_elem((8, 8), 1.0f32);
        let px = [
            AperturePixel {
                y: 2,
                x: 2,
                weight: 1.0,
            },
            AperturePixel {
                y: 9,
                x: 2,
                weight: 1.0,
            },
            AperturePixel {
                y: 2,
                x: 40,
                weight: 0.5,
            },
        ];
        let s = weighted_sum(&img, &px, None);
        assert_eq!(s.n_finite, 1);
        assert_eq!(s.weight, 1.0);
        assert_eq!(s.sum, 1.0);
    }
}
