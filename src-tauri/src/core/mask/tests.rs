use std::collections::BTreeSet;
use std::f64::consts::SQRT_2;
use std::ops::Range;
use std::path::Path;

use super::*;
use crate::core::history::id::Fnv1a;
use crate::core::imaging::contour::gaussian_blur_nan;
use crate::core::imaging::luminance::rgb_to_luminance;
use crate::core::imaging::region::paint_exclusions;
use crate::core::imaging::star_mask::generate_star_mask;
use crate::core::imaging::stats::{compute_image_stats, is_valid_pixel};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::fits::writer::write_fits_mono;

const SIDE: usize = 256;

const GOLDEN_RECIPES: [&str; 6] = [
    r#"{"kind":"luminance","auto_stf":true}"#,
    r#"{"kind":"range","lo":0.25,"hi":0.75,"fuzz":0.125}"#,
    r#"{"kind":"stars","config":{"growth_factor":2.5,"softness":4.0,"min_fwhm":1.5,"max_fwhm":30.0,"max_eccentricity":0.85,"luminance_protect":false,"luminance_ceiling":0.85},"sigma":5.0}"#,
    r#"{"kind":"regions","shapes":[{"shape":"circle","x":20.0,"y":20.0,"r":10.0},{"shape":"polygon","points":[[1.0,1.0],[9.0,1.0],[5.0,8.0]]}],"exclude_shapes":[{"shape":"box","x":22.0,"y":22.0,"width":4.0,"height":4.0,"angle":0.0}],"source_wcs":{"crval":[150.5,2.25],"crpix":[1024.5,1024.5],"cd":[[-0.0625,0.0],[0.0,0.0625]],"naxis":[2048,2048]}}"#,
    r#"{"kind":"file","path":"C:/data/jw02739_segm.fits#hdu=1","mark":"range","lo":3.0,"hi":3.0}"#,
    r#"{"kind":"file","path":"/data/m42_mask.fits","mark":"nonnan"}"#,
];

const GOLDEN_MODS: &str = r#"{"feather_px":3.0,"invert":true}"#;

const PROBIT_0_9: f64 = 1.281_551_565_544_600_5;

struct Lcg(u64);

impl Lcg {
    fn uniform(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn gaussian(&mut self) -> f32 {
        let s: f32 = (0..4).map(|_| self.uniform()).sum();
        (s - 2.0) * 3.0f32.sqrt()
    }
}

fn star_field(seed: u64, scale: f32) -> Array2<f32> {
    let mut rng = Lcg(seed);
    let mut frame = Array2::from_shape_fn((SIDE, SIDE), |(y, x)| {
        0.2 + 0.0004 * x as f32 + 0.0002 * y as f32 + 0.01 * rng.gaussian()
    });
    let stars: Vec<(f32, f32, f32)> = (0..24)
        .map(|_| (40.0 + 176.0 * rng.uniform(), 40.0 + 176.0 * rng.uniform(), 0.3 + 0.6 * rng.uniform()))
        .collect();
    for ((y, x), v) in frame.indexed_iter_mut() {
        for &(cy, cx, peak) in &stars {
            let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
            *v += peak * (-d2 / (2.0 * 1.6 * 1.6)).exp();
        }
    }
    frame.mapv_inplace(|v| v * scale);
    frame
}

fn normalised(values: &Array2<f32>) -> Array2<f32> {
    let valid: Vec<f32> = values.iter().copied().filter(|&v| is_valid_pixel(v)).collect();
    let lo = valid.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = valid.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = hi - lo;
    if !range.is_finite() || range < 1e-10 {
        return Array2::zeros(values.dim());
    }
    let inv = 1.0 / range;
    values.mapv(|v| if is_valid_pixel(v) { ((v - lo) * inv).clamp(0.0, 1.0) } else { 0.0 })
}

fn assert_bitwise(actual: &Array2<f32>, expected: &Array2<f32>, what: &str) {
    assert_eq!(actual.dim(), expected.dim(), "{what}");
    let differing = actual.iter().zip(expected.iter()).filter(|(a, e)| a.to_bits() != e.to_bits()).count();
    assert_eq!(differing, 0, "{what}: {differing} pixels differ");
}

fn raster(recipe: &MaskRecipe, planes: &[&Array2<f32>]) -> Array2<f32> {
    rasterize(recipe, &MaskMods::default(), planes).unwrap_or_else(|e| panic!("{}: {e}", recipe.kind()))
}

fn circle(x: f64, y: f64, r: f64) -> RegionShape {
    RegionShape::Circle { x, y, r }
}

fn square(x: f64, y: f64, side: f64) -> RegionShape {
    RegionShape::Box { x, y, width: side, height: side, angle: 0.0 }
}

fn regions(shapes: Vec<RegionShape>, exclude_shapes: Vec<RegionShape>) -> MaskRecipe {
    MaskRecipe::Regions { shapes, exclude_shapes, source_wcs: None }
}

fn file_recipe(path: &str, mark: MarkMode, lo: Option<f32>, hi: Option<f32>) -> MaskRecipe {
    MaskRecipe::File { path: path.to_string(), mark, lo, hi }
}

fn write_mask_file(dir: &Path, name: &str, plane: &Array2<f32>) -> String {
    let path = dir.join(name).to_string_lossy().into_owned();
    write_fits_mono(&path, plane, None).unwrap();
    path
}

fn values_of(mask: &Array2<f32>) -> Vec<f32> {
    mask.iter().copied().collect()
}

fn rectangle(dims: (usize, usize), rows: Range<usize>, cols: Range<usize>) -> Array2<f32> {
    Array2::from_shape_fn(dims, |(y, x)| if rows.contains(&y) && cols.contains(&x) { 1.0 } else { 0.0 })
}

fn rising_edges(line: Vec<f32>) -> [Vec<f32>; 2] {
    let half = line.len() / 2;
    let mut far_side = line[half..].to_vec();
    far_side.reverse();
    [line[..half].to_vec(), far_side]
}

fn crossing(rising: &[f32], level: f32) -> f64 {
    let above = rising.iter().position(|&m| m >= level).unwrap_or_else(|| panic!("the edge never reaches {level}"));
    assert!(above > 0, "the edge starts above {level}");
    let (lo, hi) = (rising[above - 1], rising[above]);
    (above - 1) as f64 + f64::from(level - lo) / f64::from(hi - lo)
}

fn width_10_90(rising: &[f32]) -> f64 {
    crossing(rising, 0.9) - crossing(rising, 0.1)
}

fn ulps_apart(a: f32, b: f32) -> u64 {
    let ordered = |v: f32| {
        let magnitude = i64::from(v.to_bits() & 0x7fff_ffff);
        if v.is_sign_negative() {
            -magnitude
        } else {
            magnitude
        }
    };
    ordered(a).abs_diff(ordered(b))
}

fn blend_pairs() -> (Array2<f32>, Array2<f32>) {
    let mut rng = Lcg(13);
    let wide = |u: f32, e: f32| (2.0 * u - 1.0) * 2f32.powi((e * 252.0) as i32 - 126);
    let mut pairs = vec![
        (f32::MAX, -f32::MAX),
        (f32::MAX, f32::MAX),
        (-f32::MAX, 1.0),
        (f32::MIN_POSITIVE, -f32::MIN_POSITIVE),
        (1e-45, 0.0),
        (0.0, -0.0),
        (-1.0, 1.000_000_1),
        (1.0, 1.000_000_1),
        (3.2e4, -3.2e4),
        (0.1, 0.1),
    ];
    while pairs.len() < 64 * 64 {
        let (a, b) = (rng.uniform(), rng.uniform());
        let pair = match pairs.len() % 5 {
            0 => (a, b),
            1 => (6.4e4 * a - 3.2e4, 6.4e4 * b - 3.2e4),
            2 => (a, -a * (1.0 + 1e-6 * b)),
            3 => (1e3 * a, 1e3 * a),
            _ => (wide(a, b), wide(b, a)),
        };
        pairs.push(pair);
    }
    let plane = |pick: fn(&(f32, f32)) -> f32| Array2::from_shape_vec((64, 64), pairs.iter().map(pick).collect()).unwrap();
    (plane(|p| p.0), plane(|p| p.1))
}

#[test]
fn recipe_json_round_trips_and_hashes_stably() {
    let mods: MaskMods = serde_json::from_str(GOLDEN_MODS).unwrap();
    assert_eq!(mods, MaskMods { feather_px: 3.0, invert: true });
    let mut hashes = BTreeSet::new();
    for golden in GOLDEN_RECIPES {
        let recipe: MaskRecipe = serde_json::from_str(golden).unwrap_or_else(|e| panic!("{golden}: {e}"));
        assert_eq!(serde_json::to_string(&recipe).unwrap(), golden, "did not re-serialise byte-identically");
        let tree: serde_json::Value = serde_json::from_str(golden).unwrap();
        assert_eq!(tree["kind"], recipe.kind());

        let hash = recipe_hash(&recipe, &mods);
        let canonical = format!("[{golden},{GOLDEN_MODS}]");
        assert_eq!(hash, Fnv1a::default().update(canonical.as_bytes()).finish(), "{golden}");
        let respelled: MaskRecipe = serde_json::from_str(&serde_json::to_string_pretty(&tree).unwrap()).unwrap();
        assert_eq!(recipe_hash(&respelled, &mods), hash, "{golden}: key order or whitespace changed the hash");
        assert_ne!(recipe_hash(&recipe, &MaskMods { feather_px: 3.5, ..mods }), hash, "{golden}: the hash ignores the feather");
        assert_ne!(recipe_hash(&recipe, &MaskMods { invert: false, ..mods }), hash, "{golden}: the hash ignores invert");
        assert!(hashes.insert(hash), "{golden}: two recipes share a hash");
    }

    let defaults = [
        (r#"{"kind":"luminance"}"#, r#"{"kind":"luminance","auto_stf":false}"#),
        (r#"{"kind":"range","lo":1.0,"hi":2.0}"#, r#"{"kind":"range","lo":1.0,"hi":2.0,"fuzz":0.0}"#),
        (
            r#"{"kind":"stars"}"#,
            r#"{"kind":"stars","config":{"growth_factor":2.5,"softness":4.0,"min_fwhm":1.5,"max_fwhm":30.0,"max_eccentricity":0.85,"luminance_protect":false,"luminance_ceiling":0.85},"sigma":5.0}"#,
        ),
        (
            r#"{"kind":"stars","config":{"growth_factor":3.0},"sigma":8.0}"#,
            r#"{"kind":"stars","config":{"growth_factor":3.0,"softness":4.0,"min_fwhm":1.5,"max_fwhm":30.0,"max_eccentricity":0.85,"luminance_protect":false,"luminance_ceiling":0.85},"sigma":8.0}"#,
        ),
        (r#"{"kind":"regions","shapes":[]}"#, r#"{"kind":"regions","shapes":[],"exclude_shapes":[]}"#),
        (r#"{"kind":"file","path":"/x.fits"}"#, r#"{"kind":"file","path":"/x.fits","mark":"nonzero"}"#),
    ];
    for (sparse, full) in defaults {
        let recipe: MaskRecipe = serde_json::from_str(sparse).unwrap_or_else(|e| panic!("{sparse}: {e}"));
        assert_eq!(serde_json::to_string(&recipe).unwrap(), full);
    }
    assert_eq!(serde_json::from_str::<MaskMods>("{}").unwrap(), MaskMods::default());
    assert_eq!(serde_json::to_string(&MaskMods::default()).unwrap(), r#"{"feather_px":0.0,"invert":false}"#);

    for (refused, named) in [
        (r#"{"kind":"luminance","autoStf":true}"#, "`autoStf`"),
        (r#"{"kind":"stars","config":{"detection_sigma":8.0}}"#, "`detection_sigma`"),
        (r#"{"kind":"range","lo":1.0}"#, "`hi`"),
        (r#"{"kind":"blur","sigma":2.0}"#, "`blur`"),
        (r#"{"kind":"file","path":"/x.fits","mark":"nonnull"}"#, "`nonnull`"),
    ] {
        let err = serde_json::from_str::<MaskRecipe>(refused).expect_err(refused).to_string();
        assert!(err.contains(named), "{refused}: {err}");
    }
    let err = serde_json::from_str::<MaskMods>(r#"{"featherPx":2.0}"#).expect_err("camelCase mods").to_string();
    assert!(err.contains("`featherPx`"), "{err}");

    for (json, source) in [
        (r#""op_input""#, MaskSource::OpInput),
        (r#"{"node":"n0002"}"#, MaskSource::Node("n0002".to_string())),
        (r#""composite""#, MaskSource::Composite),
    ] {
        assert_eq!(serde_json::to_string(&source).unwrap(), json);
        assert_eq!(serde_json::from_str::<MaskSource>(json).unwrap(), source);
    }
    assert_eq!(MaskSource::default(), MaskSource::OpInput);
    let marks: Vec<String> = [MarkMode::NonZero, MarkMode::Zero, MarkMode::Nan, MarkMode::NonNan, MarkMode::Range]
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect();
    assert_eq!(marks, [r#""nonzero""#, r#""zero""#, r#""nan""#, r#""nonnan""#, r#""range""#]);
    assert_eq!(MarkMode::default(), MarkMode::NonZero);
}

#[test]
fn luminance_mask_equals_normalised_rgb_to_luminance() {
    let mut r = star_field(1, 900.0);
    let mut g = star_field(2, 1400.0);
    let mut b = star_field(3, 500.0);
    g[[3, 4]] = f32::NAN;
    for plane in [&mut r, &mut g, &mut b] {
        plane[[5, 6]] = 0.0;
    }
    let luminance = rgb_to_luminance(&r, &g, &b);

    let plain = raster(&MaskRecipe::Luminance { auto_stf: false }, &[&r, &g, &b]);
    assert_bitwise(&plain, &normalised(&luminance), "rgb luminance");
    assert_eq!(plain[[3, 4]], 0.0, "a NaN in one channel is outside the mask");
    assert_eq!(plain[[5, 6]], 0.0, "padding is outside the mask");
    assert!(plain.iter().all(|m| (0.0..=1.0).contains(m)));
    assert!(plain.iter().copied().fold(0.0f32, f32::max) > 0.99, "the brightest pixel reaches the top of the mask");

    let mono = raster(&MaskRecipe::Luminance { auto_stf: false }, &[&r]);
    assert_bitwise(&mono, &normalised(&r), "one plane");

    let stats = compute_image_stats(&luminance);
    let expected = apply_stf_f32(&luminance, &auto_stf(&stats, &AutoStfConfig::default()), &stats);
    let stretched = raster(&MaskRecipe::Luminance { auto_stf: true }, &[&r, &g, &b]);
    assert_bitwise(&stretched, &expected, "auto-STF luminance");
    assert!(stretched.iter().all(|m| (0.0..=1.0).contains(m)));
    let mean = |m: &Array2<f32>| m.iter().map(|&v| f64::from(v)).sum::<f64>() / m.len() as f64;
    assert!(mean(&stretched) > mean(&plain), "the auto STF lifts a linear background");
}

#[test]
fn range_mask_is_one_inside_lo_hi_and_fuzzy_outside() {
    let values = [0.0, 10.0, 20.0, 25.0, 30.0, 45.0, 60.0, 70.0, 75.0, 80.0, 100.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    let plane = Array2::from_shape_vec((1, values.len()), values.to_vec()).unwrap();

    let fuzzy = raster(&MaskRecipe::Range { lo: 30.0, hi: 60.0, fuzz: 20.0 }, &[&plane]);
    assert_eq!(values_of(&fuzzy), [0.0, 0.0, 0.5, 0.75, 1.0, 1.0, 1.0, 0.5, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let hard = raster(&MaskRecipe::Range { lo: 30.0, hi: 60.0, fuzz: 0.0 }, &[&plane]);
    assert_eq!(values_of(&hard), [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

    let (r, g, b) = (star_field(4, 900.0), star_field(5, 1400.0), star_field(6, 500.0));
    let recipe = MaskRecipe::Range { lo: 400.0, hi: 800.0, fuzz: 50.0 };
    let rgb = raster(&recipe, &[&r, &g, &b]);
    assert_bitwise(&rgb, &raster(&recipe, &[&rgb_to_luminance(&r, &g, &b)]), "three planes select on the luminance");
    assert!(rgb.iter().any(|&m| m == 1.0) && rgb.iter().any(|&m| m == 0.0));

    for (lo, hi, fuzz) in [(60.0, 30.0, 0.0), (30.0, 60.0, -1.0), (f32::NAN, 60.0, 0.0), (30.0, f32::INFINITY, 0.0)] {
        let err = rasterize(&MaskRecipe::Range { lo, hi, fuzz }, &MaskMods::default(), &[&plane])
            .expect_err("a malformed range")
            .to_string();
        assert!(err.contains("range"), "{lo} {hi} {fuzz}: {err}");
    }
}

#[test]
fn stars_mask_coverage_equals_generate_star_mask() {
    let frame = star_field(7, 1000.0);
    let config = StarMaskConfig::default();
    let at = |sigma: f64| StarMaskConfig { detection_sigma: sigma, ..config.clone() };
    let recipe = MaskRecipe::Stars { config: config.clone(), sigma: 5.0 };

    let rasterized = rasterize_with_stars(&recipe, &MaskMods::default(), &[&frame]).unwrap();
    let expected = generate_star_mask(&normalised(&frame), &at(5.0)).unwrap();
    assert_bitwise(&rasterized.mask, &expected.mask, "one plane");
    assert_eq!(rasterized.stars, Some(expected.stars_masked));
    assert!(expected.stars_masked > 0);
    assert_eq!(coverage(&rasterized.mask), expected.coverage_fraction);
    assert!(expected.coverage_fraction > 0.0 && expected.coverage_fraction < 1.0);

    let (r, g, b) = (star_field(8, 900.0), star_field(9, 1400.0), star_field(10, 500.0));
    let rgb = rasterize_with_stars(&recipe, &MaskMods::default(), &[&r, &g, &b]).unwrap();
    let expected_rgb = generate_star_mask(&normalised(&rgb_to_luminance(&r, &g, &b)), &at(5.0)).unwrap();
    assert_bitwise(&rgb.mask, &expected_rgb.mask, "three planes");
    assert_eq!(rgb.stars, Some(expected_rgb.stars_masked));
    assert_eq!(coverage(&rgb.mask), expected_rgb.coverage_fraction);

    let config_threshold = MaskRecipe::Stars { config: at(80.0), sigma: 5.0 };
    assert_bitwise(&raster(&config_threshold, &[&frame]), &expected.mask, "sigma, not config.detection_sigma, is the threshold");
    let strict = MaskRecipe::Stars { config: config.clone(), sigma: 60.0 };
    assert_bitwise(&raster(&strict, &[&frame]), &generate_star_mask(&normalised(&frame), &at(60.0)).unwrap().mask, "sigma 60");

    let luminance = rasterize_with_stars(&MaskRecipe::Luminance { auto_stf: false }, &MaskMods::default(), &[&frame]).unwrap();
    assert_eq!(luminance.stars, None);
    for sigma in [0.0, -2.0, f32::NAN] {
        let err = rasterize(&MaskRecipe::Stars { config: config.clone(), sigma }, &MaskMods::default(), &[&frame])
            .expect_err("a threshold that is not positive")
            .to_string();
        assert!(err.contains("sigma"), "{sigma}: {err}");
    }
}

#[test]
fn regions_mask_painted_count_equals_paint_exclusions_and_subtracts_excludes() {
    let dims = (64, 80);
    let plane = Array2::<f32>::ones(dims);
    let shapes = vec![
        circle(20.0, 20.0, 10.0),
        RegionShape::Box { x: 55.0, y: 40.0, width: 20.0, height: 12.0, angle: 30.0 },
        RegionShape::Polygon { points: vec![[60.0, 2.0], [78.0, 4.0], [70.0, 18.0]] },
        RegionShape::Point { x: 5.0, y: 60.0 },
        circle(75.0, 60.0, 9.0),
    ];
    let excludes = vec![circle(22.0, 22.0, 4.0), square(55.0, 40.0, 6.0), square(5.0, 50.0, 6.0)];
    let mut included = Array2::<u8>::zeros(dims);
    let painted = paint_exclusions(&mut included, &shapes);
    let mut excluded = Array2::<u8>::zeros(dims);
    paint_exclusions(&mut excluded, &excludes);
    let overlap = included.iter().zip(excluded.iter()).filter(|(i, e)| **i != 0 && **e != 0).count() as u64;
    assert!(overlap > 0 && painted.painted > overlap);

    let recipe = regions(shapes.clone(), excludes);
    let mask = raster(&recipe, &[&plane]);
    assert!(mask.iter().all(|&m| m == 0.0 || m == 1.0));
    assert_eq!(mask.iter().filter(|&&m| m == 1.0).count() as u64, painted.painted - overlap);
    for ((ix, &m), (&i, &e)) in mask.indexed_iter().zip(included.iter().zip(excluded.iter())) {
        assert_eq!(m == 1.0, i != 0 && e == 0, "pixel {ix:?}");
    }
    let union = raster(&regions(shapes, vec![]), &[&plane]);
    assert_eq!(union.iter().filter(|&&m| m == 1.0).count() as u64, painted.painted);
    assert_bitwise(&raster(&recipe, &[&plane, &plane, &plane]), &mask, "regions on three planes");

    let bad = regions(vec![circle(10.0, 10.0, 3.0), circle(10.0, 10.0, -1.0)], vec![]);
    let err = rasterize(&bad, &MaskMods::default(), &[&plane]).expect_err("a negative radius").to_string();
    assert!(err.contains("shape 2 (circle)") && err.contains("r must be > 0"), "{err}");
    let bad_exclude = regions(vec![circle(10.0, 10.0, 3.0)], vec![square(1.0, 1.0, 0.0)]);
    let err = rasterize(&bad_exclude, &MaskMods::default(), &[&plane]).expect_err("an empty box").to_string();
    assert!(err.contains("exclude shape 1 (box)"), "{err}");
}

#[test]
fn file_mask_mark_modes() {
    let dir = tempfile::tempdir().unwrap();
    let values = [0.0, -0.0, 1.0, 2.0, 2.5, 3.0, 3.5, -4.0, f32::NAN, f32::INFINITY, 7.0, 0.0];
    let path = write_mask_file(dir.path(), "labels.fits", &Array2::from_shape_vec((3, 4), values.to_vec()).unwrap());
    let image = Array2::<f32>::zeros((3, 4));
    let marked = |mark: MarkMode, lo: Option<f32>, hi: Option<f32>| values_of(&raster(&file_recipe(&path, mark, lo, hi), &[&image]));

    assert_eq!(marked(MarkMode::NonZero, None, None), [0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0]);
    assert_eq!(marked(MarkMode::Zero, None, None), [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(marked(MarkMode::Nan, None, None), [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    assert_eq!(marked(MarkMode::NonNan, None, None), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0]);
    assert_eq!(marked(MarkMode::Range, Some(2.0), Some(3.0)), [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(marked(MarkMode::Range, Some(3.0), Some(3.0)), [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(marked(MarkMode::Zero, Some(9.0), None), marked(MarkMode::Zero, None, None), "lo and hi only matter for range");

    for (lo, hi) in [(None, None), (Some(2.0), None), (Some(4.0), Some(3.0)), (Some(f32::NAN), Some(3.0))] {
        let err = rasterize(&file_recipe(&path, MarkMode::Range, lo, hi), &MaskMods::default(), &[&image])
            .expect_err("a malformed mark range")
            .to_string();
        assert!(err.contains("mark range"), "{lo:?} {hi:?}: {err}");
    }
    let missing = dir.path().join("missing.fits").to_string_lossy().into_owned();
    let err = rasterize(&file_recipe(&missing, MarkMode::NonZero, None, None), &MaskMods::default(), &[&image])
        .expect_err("a missing mask file")
        .to_string();
    assert!(err.contains(&missing), "{err}");
}

#[test]
fn invert_plus_mask_is_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let frame = star_field(11, 1000.0);
    let file = write_mask_file(dir.path(), "bright.fits", &frame.mapv(|v| if v > 250.0 { v } else { 0.0 }));
    let recipes = [
        MaskRecipe::Luminance { auto_stf: false },
        MaskRecipe::Luminance { auto_stf: true },
        MaskRecipe::Range { lo: 230.0, hi: 400.0, fuzz: 37.0 },
        MaskRecipe::Stars { config: StarMaskConfig::default(), sigma: 5.0 },
        regions(vec![circle(100.0, 90.0, 40.0)], vec![circle(100.0, 90.0, 10.0)]),
        file_recipe(&file, MarkMode::NonZero, None, None),
    ];
    let inverted = MaskMods { invert: true, ..MaskMods::default() };
    for recipe in &recipes {
        let mask = raster(recipe, &[&frame]);
        let inverse = rasterize(recipe, &inverted, &[&frame]).unwrap();
        assert!(mask.iter().chain(inverse.iter()).all(|m| (0.0..=1.0).contains(m)), "{}", recipe.kind());
        let off = mask.iter().zip(inverse.iter()).filter(|(m, i)| **m + **i != 1.0).count();
        assert_eq!(off, 0, "{}: m + invert(m) differs from 1 on {off} pixels", recipe.kind());
    }
    let fractional = raster(&recipes[0], &[&frame]);
    assert!(fractional.iter().any(|&m| m > 0.0 && m < 1.0), "the luminance case covers fractional values");
}

#[test]
fn mask_dims_must_match_image() {
    let err = check_dims("m01", (2058, 2058), (2048, 2048)).expect_err("a foreign mask").to_string();
    assert_eq!(err, "mask m01 is 2058×2058, image is 2048×2048");
    let err = check_dims("m02", (10, 20), (20, 10)).expect_err("a transposed mask").to_string();
    assert_eq!(err, "mask m02 is 20×10, image is 10×20", "the sizes read width × height");
    assert!(check_dims("m03", (2048, 2048), (2048, 2048)).is_ok());

    let dir = tempfile::tempdir().unwrap();
    let path = write_mask_file(dir.path(), "wide.fits", &Array2::<f32>::ones((10, 12)));
    let image = Array2::<f32>::zeros((10, 10));
    let foreign = raster(&file_recipe(&path, MarkMode::NonZero, None, None), &[&image]);
    assert_eq!(foreign.dim(), (10, 12), "a file mask keeps the size of its file");
    let err = check_dims("m01", foreign.dim(), image.dim()).expect_err("12×10 on 10×10").to_string();
    assert_eq!(err, "mask m01 is 12×10, image is 10×10");
    for recipe in [
        MaskRecipe::Luminance { auto_stf: false },
        MaskRecipe::Range { lo: 0.0, hi: 1.0, fuzz: 0.0 },
        regions(vec![circle(4.0, 4.0, 3.0)], vec![]),
    ] {
        assert_eq!(raster(&recipe, &[&image]).dim(), image.dim(), "{}", recipe.kind());
    }

    let narrow = Array2::<f32>::zeros((10, 11));
    let recipe = MaskRecipe::Luminance { auto_stf: false };
    let err = rasterize(&recipe, &MaskMods::default(), &[&image, &image, &narrow]).expect_err("planes of two sizes").to_string();
    assert!(err.contains("one size"), "{err}");
    let err = rasterize(&recipe, &MaskMods::default(), &[&image, &image]).expect_err("two planes").to_string();
    assert!(err.contains("one or three planes; got 2"), "{err}");
    let err = rasterize(&recipe, &MaskMods::default(), &[]).expect_err("no plane").to_string();
    assert!(err.contains("got 0"), "{err}");
}

#[test]
fn feather_must_be_a_finite_radius_of_zero_or_more() {
    let frame = star_field(12, 1.0);
    for feather_px in [-1.0, f32::NAN, f32::INFINITY] {
        let mods = MaskMods { feather_px, invert: false };
        let err = rasterize(&MaskRecipe::Luminance { auto_stf: false }, &mods, &[&frame]).expect_err("a bad feather").to_string();
        assert!(err.contains("feather"), "{feather_px}: {err}");
    }
}

#[test]
fn label_image_assigns_shape_index_and_zero_outside_or_excluded() {
    let dims = (40, 60);
    let first = circle(20.0, 20.0, 10.0);
    let second = circle(32.0, 20.0, 10.0);
    let hole = square(36.0, 20.0, 4.0);
    let recipe = regions(vec![first.clone(), second.clone()], vec![hole.clone()]);
    let labels = label_image(&recipe, dims).unwrap();
    assert_eq!(labels.dim(), dims);
    assert_eq!(labels.iter().copied().collect::<BTreeSet<i32>>(), BTreeSet::from([0, 1, 2]));
    for ((y, x), &label) in labels.indexed_iter() {
        let (px, py) = (x as f64, y as f64);
        let expected = if hole.contains(px, py) {
            0
        } else if second.contains(px, py) {
            2
        } else if first.contains(px, py) {
            1
        } else {
            0
        };
        assert_eq!(label, expected, "pixel ({x}, {y})");
    }
    assert!(labels.indexed_iter().any(|((y, x), &l)| l == 2 && first.contains(x as f64, y as f64)), "the overlap takes the later index");
    assert!(labels.indexed_iter().any(|((y, x), _)| hole.contains(x as f64, y as f64)), "the excluded box covers pixels");
    let mask = raster(&recipe, &[&Array2::<f32>::zeros(dims)]);
    assert!(labels.iter().zip(mask.iter()).all(|(&l, &m)| (l != 0) == (m == 1.0)), "the label and float masks cover the same pixels");

    let after_a_point = regions(vec![RegionShape::Point { x: 5.0, y: 5.0 }, first], vec![]);
    let indices: BTreeSet<i32> = label_image(&after_a_point, dims).unwrap().iter().copied().collect();
    assert_eq!(indices, BTreeSet::from([0, 2]), "a shape keeps its index in the recipe after a shape that paints nothing");

    let err = label_image(&MaskRecipe::Luminance { auto_stf: false }, dims).expect_err("a luminance mask").to_string();
    assert_eq!(err, "only a regions mask has a label image; this mask is luminance");
    let err = label_image(&regions(vec![circle(1.0, 1.0, 0.0)], vec![]), dims).expect_err("a zero radius").to_string();
    assert!(err.contains("shape 1 (circle)"), "{err}");
}

#[test]
fn gaussian_feather_10_90_edge_width_is_2563_sigma_within_10pct() {
    let side = 512;
    let plane = Array2::<f32>::zeros((side, side));
    let square = regions(vec![RegionShape::Box { x: 255.5, y: 255.5, width: 256.0, height: 256.0, angle: 0.0 }], vec![]);
    let hard = raster(&square, &[&plane]);
    assert_bitwise(&hard, &rectangle((side, side), 128..384, 128..384), "the square covers rows and columns 128..384");
    let expected = 2.0 * PROBIT_0_9;
    for sigma in [3.0f32, 10.0, 20.0] {
        let feathered = feather(&hard, sigma);
        let profiles = [rising_edges(feathered.row(side / 2).to_vec()), rising_edges(feathered.column(side / 2).to_vec())];
        for (edge, rising) in ["left", "right", "top", "bottom"].into_iter().zip(profiles.iter().flatten()) {
            let width = width_10_90(rising) / f64::from(sigma);
            println!("gaussian feather sigma {sigma}, {edge} edge: 10-90 width {width:.4} sigma (expected {expected:.4})");
            assert!(
                (width / expected - 1.0).abs() <= 0.10,
                "sigma {sigma}, {edge} edge: 10-90 width {width:.4} sigma, expected {expected:.4} within 10 %"
            );
        }
        assert_bitwise(&feathered, &gaussian_blur_nan(&hard, f64::from(sigma)), &format!("sigma {sigma}: the contour.rs Gaussian"));
        assert_eq!(feathered[[0, 0]], 0.0, "sigma {sigma}: far outside the square stays exactly 0");
        assert_eq!(feathered[[side / 2, side / 2]], 1.0, "sigma {sigma}: deep inside the square stays exactly 1");

        let mods = MaskMods { feather_px: sigma, invert: false };
        let rasterized = rasterize(&square, &mods, &[&plane]).unwrap();
        assert_bitwise(&rasterized, &feathered.mapv(|m| m.clamp(0.0, 1.0)), &format!("sigma {sigma}: rasterize with feather_px"));
        let inverse = rasterize(&square, &MaskMods { invert: true, ..mods }, &[&plane]).unwrap();
        let off = rasterized.iter().zip(inverse.iter()).filter(|(m, i)| **m + **i != 1.0).count();
        assert_eq!(off, 0, "sigma {sigma}: m + invert(m) differs from 1 on {off} feathered pixels");
    }
}

#[test]
fn box_feather_above_20px_matches_gaussian_within_3pct_rms() {
    let dims = (1024, 1152);
    let (rows, cols) = (256..768, 320..832);
    let hard = rectangle(dims, rows.clone(), cols.clone());
    for sigma in [21.0f32, 40.0] {
        let s = f64::from(sigma);
        let feathered = feather(&hard, sigma);
        let blurred_step = |t: usize, inside: &Range<usize>| {
            let (t, from, to) = (t as f64, inside.start as f64 - 0.5, inside.end as f64 - 0.5);
            0.5 * (libm::erfc((t - to) / (s * SQRT_2)) - libm::erfc((t - from) / (s * SQRT_2)))
        };
        let down: Vec<f64> = (0..dims.0).map(|y| blurred_step(y, &rows)).collect();
        let across: Vec<f64> = (0..dims.1).map(|x| blurred_step(x, &cols)).collect();
        let (mut squares, mut band, mut worst) = (0.0f64, 0usize, 0.0f64);
        for ((y, x), &m) in feathered.indexed_iter() {
            let gaussian = down[y] * across[x];
            if (0.01..=0.99).contains(&gaussian) {
                let diff = f64::from(m) - gaussian;
                squares += diff * diff;
                band += 1;
                worst = worst.max(diff.abs());
            }
        }
        let rms = (squares / band as f64).sqrt();
        println!("box feather sigma {sigma}: rms {rms:.5}, max {worst:.5} against the Gaussian over {band} feather-band pixels");
        assert!(rms <= 0.03, "box feather sigma {sigma}: rms {rms:.5} against the Gaussian, expected <= 0.03");
    }

    let sigma = 40.0f32;
    let s = f64::from(sigma);
    let feathered = feather(&hard, sigma);
    let plane = Array2::<f32>::zeros(dims);
    let rectangle_region = regions(vec![RegionShape::Box { x: 575.5, y: 511.5, width: 512.0, height: 512.0, angle: 0.0 }], vec![]);
    assert_bitwise(&raster(&rectangle_region, &[&plane]), &hard, "the box region covers rows 256..768 and columns 320..832");
    let rasterized = rasterize(&rectangle_region, &MaskMods { feather_px: sigma, invert: false }, &[&plane]).unwrap();
    assert_bitwise(&rasterized, &feathered.mapv(|m| m.clamp(0.0, 1.0)), "rasterize with feather_px above 20 takes the box path");

    let radius = (((12.0 * s * s / 3.0 + 1.0).sqrt() - 1.0) / 2.0).round() as usize;
    assert_eq!(2 * radius + 1, 81, "sigma 40 takes a box of width 81");
    let reach = 3 * radius;
    let middle = feathered.row(dims.0 / 2);
    assert_eq!(middle[cols.start - reach - 1], 0.0, "three boxes reach {reach} px outside the edge, no further");
    assert!(middle[cols.start - reach] > 0.0, "three boxes reach {reach} px outside the edge");
    assert!(middle[cols.start + reach - 1] < 1.0, "the edge is felt {reach} px inside");
    assert_eq!(middle[cols.start + reach], 1.0, "beyond {reach} px inside the mask is exactly 1");
    assert!(feathered.iter().all(|m| (0.0..=1.0).contains(m)));

    let small = (64, 80);
    assert_bitwise(&feather(&Array2::ones(small), sigma), &Array2::ones(small), "a full mask stays 1 up to the borders");
    assert_bitwise(&feather(&Array2::zeros(small), sigma), &Array2::zeros(small), "an empty mask stays 0");

    let hole = [dims.0 / 2, dims.1 / 2];
    let mut holed = hard.clone();
    holed[hole] = f32::NAN;
    let mut with_hole = feather(&holed, sigma);
    assert!(with_hole[hole].is_nan(), "a NaN pixel stays NaN, as in gaussian_blur_nan");
    with_hole[hole] = feathered[hole];
    assert_bitwise(&with_hole, &feathered, "a NaN pixel is left out of its neighbours' boxes");
}

#[test]
fn blend_is_exact_at_0_and_1_and_midpoint_within_1_ulp() {
    let (base, processed) = blend_pairs();
    let uniform = |m: f32| Array2::from_elem(base.dim(), m);

    assert_bitwise(&blend(&base, &processed, &uniform(0.0)).unwrap(), &base, "m = 0 gives the base");
    assert_bitwise(&blend(&base, &processed, &uniform(1.0)).unwrap(), &processed, "m = 1 gives the processed value");
    let midpoint = blend(&base, &processed, &uniform(0.5)).unwrap();
    Zip::from(&midpoint).and(&base).and(&processed).for_each(|&out, &b, &p| {
        let exact = ((f64::from(b) + f64::from(p)) / 2.0) as f32;
        assert!(ulps_apart(out, exact) <= 1, "base {b:e}, processed {p:e}: midpoint {out:e}, exact {exact:e}");
    });

    let mut rng = Lcg(14);
    let mixed = Array2::from_shape_fn(base.dim(), |_| match (rng.uniform() * 3.0) as u32 {
        0 => 0.0,
        1 => 1.0,
        _ => rng.uniform(),
    });
    let out = blend(&base, &processed, &mixed).unwrap();
    Zip::from(&out).and(&base).and(&processed).and(&mixed).for_each(|&o, &b, &p, &m| {
        if m == 0.0 {
            assert_eq!(o.to_bits(), b.to_bits(), "m 0: base {b:e}");
        } else if m == 1.0 {
            assert_eq!(o.to_bits(), p.to_bits(), "m 1: processed {p:e}");
        } else {
            assert!(b.min(p) <= o && o <= b.max(p), "m {m}: {o:e} is outside [{b:e}, {p:e}]");
        }
    });

    for m in [-0.5, -0.0, f32::NAN] {
        assert_bitwise(&blend(&base, &processed, &uniform(m)).unwrap(), &base, &format!("m {m} keeps the base"));
    }
    for m in [1.5, f32::INFINITY] {
        assert_bitwise(&blend(&base, &processed, &uniform(m)).unwrap(), &processed, &format!("m {m} takes the processed value"));
    }

    let narrow = Array2::<f32>::zeros((64, 63));
    let err = blend(&base, &narrow, &mixed).expect_err("a processed plane of another size").to_string();
    assert_eq!(err, "a blend needs base, processed and mask of one size; got base 64×64, processed 63×64, mask 64×64");
    let err = blend(&base, &processed, &narrow.t().to_owned()).expect_err("a mask of another size").to_string();
    assert_eq!(err, "a blend needs base, processed and mask of one size; got base 64×64, processed 64×64, mask 64×63");
}

#[test]
fn non_finite_pixels_follow_the_unmasked_result() {
    let non_finite = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    let payload_nan = f32::from_bits(0x7fc0_0001);
    let outputs = [payload_nan, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, 0.75, -3.5e4];
    let cases: Vec<(f32, f32)> = non_finite
        .iter()
        .flat_map(|&b| outputs.iter().map(move |&p| (b, p)))
        .chain([0.25f32, -2.0, 3.2e4].iter().flat_map(|&b| [payload_nan, f32::INFINITY, f32::NEG_INFINITY].map(|p| (b, p))))
        .collect();
    let row = |pick: fn(&(f32, f32)) -> f32| Array2::from_shape_vec((1, cases.len()), cases.iter().map(pick).collect()).unwrap();
    let (base, processed) = (row(|c| c.0), row(|c| c.1));

    for m in [0.0f32, 1e-7, 0.25, 0.5, 0.999_999_9, 1.0] {
        let out = blend(&base, &processed, &Array2::from_elem(base.dim(), m)).unwrap();
        Zip::from(&out).and(&base).and(&processed).for_each(|&o, &b, &p| {
            if b.is_finite() && m == 0.0 {
                assert_eq!(o.to_bits(), b.to_bits(), "m 0 keeps a finite base {b} next to a processed {p}");
            } else {
                assert_eq!(o.to_bits(), p.to_bits(), "m {m}, base {b}: the unmasked result {p} was not kept, got {o}");
            }
        });
    }
}

#[test]
fn mask_spec_reads_the_recipe_and_its_mods_from_one_object() {
    let spec: MaskSpec = serde_json::from_str(r#"{"kind":"luminance","feather_px":3,"invert":true}"#).unwrap();
    assert_eq!(spec.recipe.kind(), "luminance");
    assert_eq!(spec.mods, MaskMods { feather_px: 3.0, invert: true });

    let mods: serde_json::Value = serde_json::from_str(GOLDEN_MODS).unwrap();
    for recipe in GOLDEN_RECIPES {
        let plain: MaskSpec = serde_json::from_str(recipe).unwrap_or_else(|e| panic!("{recipe}: {e}"));
        assert_eq!(plain.mods, MaskMods::default(), "{recipe}");
        let mut merged: serde_json::Value = serde_json::from_str(recipe).unwrap();
        merged.as_object_mut().unwrap().extend(mods.as_object().unwrap().clone());
        let spec: MaskSpec = serde_json::from_value(merged).unwrap_or_else(|e| panic!("{recipe} with mods: {e}"));
        let separate: MaskRecipe = serde_json::from_str(recipe).unwrap();
        let separate_mods: MaskMods = serde_json::from_str(GOLDEN_MODS).unwrap();
        assert_eq!(recipe_hash(&spec.recipe, &spec.mods), recipe_hash(&separate, &separate_mods), "{recipe}");
    }

    for (text, field) in [
        (r#"{"kind":"luminance","featherPx":2}"#, "featherPx"),
        (r#"{"kind":"stars","growth":2.5}"#, "growth"),
        (r#"{"kind":"range","lo":0.1,"hi":0.9,"source":{"node":"n0002"}}"#, "source"),
        (r#"{"feather_px":1}"#, "kind"),
    ] {
        let err = serde_json::from_str::<MaskSpec>(text).expect_err(text).to_string();
        assert!(err.contains(field), "{text}: {err}");
    }

    let plane = star_field(5, 1.0);
    let spec: MaskSpec = serde_json::from_str(r#"{"kind":"range","lo":0.2,"hi":0.6,"fuzz":0.05,"invert":true}"#).unwrap();
    let expected = rasterize(&spec.recipe, &spec.mods, &[&plane]).unwrap();
    assert_bitwise(&spec.rasterize(&[&plane]).unwrap(), &expected, "MaskSpec::rasterize");
}
