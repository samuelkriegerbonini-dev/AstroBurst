use std::time::Instant;

use ndarray::Array2;

use crate::core::analysis::star_detection::detect_stars;
use crate::core::imaging::contour::gaussian_blur_nan;
use crate::core::mask::feather;

const BENCH_DIMS: [(usize, usize); 2] = [(4000, 6000), (4177, 7065)];
const SIGMAS: [f64; 2] = [5.0, 20.0];
const BOX_DIMS: (usize, usize) = BENCH_DIMS[1];
const BOX_SIGMAS: [f32; 2] = [40.0, 100.0];
const STARS_PER_MEGAPIXEL: f64 = 40.0;
const STAR_SIGMA_PX: f32 = 1.6;
const STAR_REACH_SIGMAS: f32 = 4.0;
const BACKGROUND: f32 = 0.1;
const NOISE_AMPLITUDE: f32 = 0.01;

struct Lcg(u64);

impl Lcg {
    fn uniform(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn megapixels((rows, cols): (usize, usize)) -> f64 {
    (rows * cols) as f64 / 1e6
}

fn star_field((rows, cols): (usize, usize)) -> Array2<f32> {
    let mut rng = Lcg(0x6d61_736b);
    let mut field =
        Array2::from_shape_fn((rows, cols), |_| BACKGROUND + NOISE_AMPLITUDE * (2.0 * rng.uniform() - 1.0));
    let reach = (STAR_REACH_SIGMAS * STAR_SIGMA_PX).ceil() as usize;
    let stars = (STARS_PER_MEGAPIXEL * megapixels((rows, cols))) as usize;
    for _ in 0..stars {
        let cy = reach + (rng.uniform() * (rows - 2 * reach) as f32) as usize;
        let cx = reach + (rng.uniform() * (cols - 2 * reach) as f32) as usize;
        let peak = 0.2 + 0.6 * rng.uniform();
        for y in cy - reach..=cy + reach {
            for x in cx - reach..=cx + reach {
                let d2 = (y as f32 - cy as f32).powi(2) + (x as f32 - cx as f32).powi(2);
                field[[y, x]] += peak * (-d2 / (2.0 * STAR_SIGMA_PX * STAR_SIGMA_PX)).exp();
            }
        }
    }
    field
}

fn ellipse_mask((rows, cols): (usize, usize)) -> Array2<f32> {
    let (cy, cx) = (rows as f64 / 2.0, cols as f64 / 2.0);
    let (ry, rx) = (rows as f64 / 3.0, cols as f64 / 3.0);
    Array2::from_shape_fn((rows, cols), |(y, x)| {
        let (u, v) = ((x as f64 - cx) / rx, (y as f64 - cy) / ry);
        if u * u + v * v <= 1.0 {
            1.0
        } else {
            0.0
        }
    })
}

#[test]
#[ignore = "timing bench: cargo test --release --lib bench_mask_inputs -- --ignored --nocapture"]
fn detect_stars_timing_at_6000x4000_and_7065x4177() {
    for dims in BENCH_DIMS {
        let field = star_field(dims);
        for sigma in SIGMAS {
            let started = Instant::now();
            let detection = detect_stars(&field, sigma);
            let elapsed = started.elapsed().as_secs_f64() * 1e3;
            println!(
                "detect_stars {}x{} ({:.1} MP) sigma {sigma}: {} stars in {elapsed:.0} ms ({})",
                dims.1,
                dims.0,
                megapixels(dims),
                detection.stars.len(),
                profile()
            );
            assert!(!detection.stars.is_empty(), "no star found at sigma {sigma}");
        }
    }
}

#[test]
#[ignore = "timing bench: cargo test --release --lib bench_mask_inputs -- --ignored --nocapture"]
fn gaussian_blur_nan_timing_at_sigma_5_and_20() {
    for dims in BENCH_DIMS {
        let mask = ellipse_mask(dims);
        for sigma in SIGMAS {
            let started = Instant::now();
            let blurred = gaussian_blur_nan(&mask, sigma);
            let elapsed = started.elapsed().as_secs_f64() * 1e3;
            println!(
                "gaussian_blur_nan {}x{} ({:.1} MP) sigma {sigma}: {elapsed:.0} ms ({})",
                dims.1,
                dims.0,
                megapixels(dims),
                profile()
            );
            assert_eq!(blurred.dim(), dims);
            assert!(blurred.iter().all(|v| (0.0..=1.0 + 1e-5).contains(v)));
        }
    }
}

#[test]
#[ignore = "timing bench: cargo test --release --lib bench_mask_inputs -- --ignored --nocapture"]
fn box_feather_timing_at_7065x4177() {
    let mask = ellipse_mask(BOX_DIMS);
    for sigma in BOX_SIGMAS {
        let started = Instant::now();
        let feathered = feather(&mask, sigma);
        let elapsed = started.elapsed().as_secs_f64() * 1e3;
        println!(
            "box feather {}x{} ({:.1} MP) sigma {sigma}: {elapsed:.0} ms ({})",
            BOX_DIMS.1,
            BOX_DIMS.0,
            megapixels(BOX_DIMS),
            profile()
        );
        assert_eq!(feathered.dim(), BOX_DIMS);
        assert!(feathered.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(feathered.iter().any(|&v| v > 0.0 && v < 1.0), "sigma {sigma}: the feather softens the ellipse edge");
    }
}
