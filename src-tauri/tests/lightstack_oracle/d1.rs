use std::f64::consts::PI;
use std::path::Path;

use astroburst_lib::core::synth::seed::{derive_seed, splitmix64, SeedPurpose};
use libm::{atan2, cos, exp, j1, log, pow, sin};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SEED: u64 = 20_261_030;
const FIELD: [f64; 2] = [53.1625, -27.7914];
const REFERENCE: &str = "F444W";
const CUBE_SCALE: f64 = 0.04;
const CROP_ARCSEC: f64 = 1.8;
const TRUNCATION_ARCSEC: f64 = 0.4;
const SUBSAMPLING: usize = 5;
const APERTURE_FWHM: f64 = 2.0;
const SNR_FLOOR: f64 = 300.0;
const NAN_BORDER: usize = 2;
const BORDER_DQ: i32 = 513;
const HOLE_DQ: i32 = 1;
const FLAG_DQ: i32 = 4;
const DEG: f64 = PI / 180.0;
const C_ANGSTROM_PER_S: f64 = 2.99792458e18;
const PHOTFLAM: f64 = 7.0723e-20;
const PHOTPLAM: f64 = 8045.53;
const BLOCK: usize = 2880;
const GENERATOR: &str = "src-tauri/tests/lightstack_oracle/d1.rs";
const REGENERATE: &str =
    "ASTROBURST_WRITE_LIGHTSTACK_FIXTURES=1 cargo test --test lightstack_oracle regenerate_d1_fixtures -- --ignored";

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    MjySr,
    JyPixel,
    ElectronsPerSecond,
}

#[derive(Clone, Copy)]
enum Profile {
    Gaussian,
    Moffat(f64),
    Airy,
}

struct Plane {
    dataset: &'static str,
    filter: &'static str,
    telescope: &'static str,
    instrument: &'static str,
    scale: f64,
    size: usize,
    rotation_deg: f64,
    crval_offset: [f64; 2],
    centre_pixel: [f64; 2],
    sip: bool,
    unit: Unit,
    profile: Profile,
    fwhm: f64,
    sigma0_jy: f64,
}

const PLANES: [Plane; 6] = [
    Plane {
        dataset: "HST",
        filter: "F814W",
        telescope: "HST",
        instrument: "ACS",
        scale: 0.04,
        size: 64,
        rotation_deg: 12.0,
        crval_offset: [0.25, -0.15],
        centre_pixel: [32.63, 32.23],
        sip: true,
        unit: Unit::ElectronsPerSecond,
        profile: Profile::Moffat(3.0),
        fwhm: 0.095,
        sigma0_jy: 8e-10,
    },
    Plane {
        dataset: "JWST",
        filter: "F150W",
        telescope: "JWST",
        instrument: "NIRCAM",
        scale: 0.02,
        size: 128,
        rotation_deg: 20.0,
        crval_offset: [0.0, 0.0],
        centre_pixel: [64.75, 64.1],
        sip: false,
        unit: Unit::MjySr,
        profile: Profile::Airy,
        fwhm: 0.05,
        sigma0_jy: 3e-10,
    },
    Plane {
        dataset: "JWST",
        filter: "F200W",
        telescope: "JWST",
        instrument: "NIRCAM",
        scale: 0.02,
        size: 128,
        rotation_deg: 20.0,
        crval_offset: [0.0, 0.0],
        centre_pixel: [64.75, 64.1],
        sip: false,
        unit: Unit::MjySr,
        profile: Profile::Airy,
        fwhm: 0.066,
        sigma0_jy: 3e-10,
    },
    Plane {
        dataset: "JWST",
        filter: "F277W",
        telescope: "JWST",
        instrument: "NIRCAM",
        scale: 0.04,
        size: 64,
        rotation_deg: 0.0,
        crval_offset: [0.0, 0.0],
        centre_pixel: [32.5, 32.5],
        sip: false,
        unit: Unit::JyPixel,
        profile: Profile::Gaussian,
        fwhm: 0.092,
        sigma0_jy: 6e-10,
    },
    Plane {
        dataset: "JWST",
        filter: "F356W",
        telescope: "JWST",
        instrument: "NIRCAM",
        scale: 0.04,
        size: 64,
        rotation_deg: 3.0,
        crval_offset: [0.0, 0.0],
        centre_pixel: [32.87, 32.29],
        sip: false,
        unit: Unit::JyPixel,
        profile: Profile::Moffat(2.5),
        fwhm: 0.116,
        sigma0_jy: 6e-10,
    },
    Plane {
        dataset: "JWST",
        filter: "F444W",
        telescope: "JWST",
        instrument: "NIRCAM",
        scale: 0.04,
        size: 64,
        rotation_deg: 0.0,
        crval_offset: [0.0, 0.0],
        centre_pixel: [32.5, 32.5],
        sip: false,
        unit: Unit::MjySr,
        profile: Profile::Airy,
        fwhm: 0.145,
        sigma0_jy: 6e-10,
    },
];

const SIP_A: [(i32, i32, f64); 3] = [(2, 0, 2.4e-4), (1, 1, -1.6e-4), (0, 2, 1.1e-4)];
const SIP_B: [(i32, i32, f64); 3] = [(2, 0, -1.3e-4), (1, 1, 2.1e-4), (0, 2, 1.7e-4)];

struct Star {
    id: &'static str,
    offset: [f64; 2],
    flux: f64,
}

const STARS: [Star; 5] = [
    Star {
        id: "s1",
        offset: [0.52, 0.50],
        flux: 2.0e-5,
    },
    Star {
        id: "s2",
        offset: [-0.50, 0.52],
        flux: 8.0e-6,
    },
    Star {
        id: "s3",
        offset: [0.50, -0.53],
        flux: 1.2e-5,
    },
    Star {
        id: "s4",
        offset: [-0.53, -0.49],
        flux: 1.0e-5,
    },
    Star {
        id: "s5",
        offset: [0.73, 0.04],
        flux: 3.0e-5,
    },
];
const STAR_SED: [f64; 6] = [0.80, 1.00, 1.00, 0.85, 0.70, 0.60];

struct Clump {
    offset: [f64; 2],
    sigma: f64,
    flux: f64,
}

const CLUMPS: [Clump; 4] = [
    Clump {
        offset: [0.14, 0.09],
        sigma: 0.025,
        flux: 5.0e-6,
    },
    Clump {
        offset: [-0.11, 0.12],
        sigma: 0.030,
        flux: 3.5e-6,
    },
    Clump {
        offset: [0.05, -0.17],
        sigma: 0.020,
        flux: 4.0e-6,
    },
    Clump {
        offset: [-0.19, -0.05],
        sigma: 0.035,
        flux: 2.5e-6,
    },
];
const CLUMP_SED: [f64; 6] = [1.40, 1.10, 1.00, 0.80, 0.62, 0.50];
const GALAXY_OFFSET: [f64; 2] = [0.04, -0.03];
const DISC_FLUX: f64 = 3.0e-5;
const DISC_SED: [f64; 6] = [0.35, 0.65, 0.80, 1.00, 1.10, 1.15];
const DISC_SCALE: f64 = 0.08;
const DISC_AXIS_RATIO: f64 = 0.55;
const DISC_PA_DEG: f64 = 35.0;
const DISC_TRUNCATION: f64 = 0.40;

struct Hole {
    offset: [f64; 2],
    width: usize,
    height: usize,
}

const HOLES: [Hole; 2] = [
    Hole {
        offset: [-0.04, 0.02],
        width: 3,
        height: 2,
    },
    Hole {
        offset: [0.0, 0.95],
        width: 2,
        height: 2,
    },
];
const FLAGGED: [[f64; 2]; 2] = [[0.0, -0.95], [-0.95, 0.0]];

fn project(origin: [f64; 2], sky: [f64; 2]) -> [f64; 2] {
    let (sin0, cos0) = (sin(origin[1] * DEG), cos(origin[1] * DEG));
    let (sin_dec, cos_dec) = (sin(sky[1] * DEG), cos(sky[1] * DEG));
    let delta = (sky[0] - origin[0]) * DEG;
    let cos_c = sin_dec * sin0 + cos_dec * cos0 * cos(delta);
    [
        cos_dec * sin(delta) / cos_c / DEG,
        (cos0 * sin_dec - sin0 * cos_dec * cos(delta)) / cos_c / DEG,
    ]
}

fn deproject(origin: [f64; 2], plane: [f64; 2]) -> [f64; 2] {
    let (xi, eta) = (plane[0] * DEG, plane[1] * DEG);
    let (sin0, cos0) = (sin(origin[1] * DEG), cos(origin[1] * DEG));
    let den = cos0 - eta * sin0;
    [
        origin[0] + atan2(xi, den) / DEG,
        atan2(sin0 + eta * cos0, (xi * xi + den * den).sqrt()) / DEG,
    ]
}

fn offset_sky(origin: [f64; 2], offset_arcsec: [f64; 2]) -> [f64; 2] {
    deproject(
        origin,
        [offset_arcsec[0] / 3600.0, offset_arcsec[1] / 3600.0],
    )
}

fn solve(matrix: [[f64; 2]; 2], target: [f64; 2]) -> [f64; 2] {
    let det = matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
    [
        (matrix[1][1] * target[0] - matrix[0][1] * target[1]) / det,
        (matrix[0][0] * target[1] - matrix[1][0] * target[0]) / det,
    ]
}

fn power(base: f64, exponent: i32) -> f64 {
    (0..exponent).fold(1.0, |product, _| product * base)
}

fn sip_terms(terms: &[(i32, i32, f64)], u: f64, v: f64) -> [f64; 3] {
    terms.iter().fold([0.0; 3], |[value, du, dv], &(p, q, c)| {
        [
            value + c * power(u, p) * power(v, q),
            du + if p > 0 {
                c * p as f64 * power(u, p - 1) * power(v, q)
            } else {
                0.0
            },
            dv + if q > 0 {
                c * q as f64 * power(u, p) * power(v, q - 1)
            } else {
                0.0
            },
        ]
    })
}

struct Wcs {
    crpix: [f64; 2],
    crval: [f64; 2],
    cd: [[f64; 2]; 2],
    sip: bool,
}

impl Wcs {
    fn of(plane: &Plane) -> Wcs {
        let degrees = plane.scale / 3600.0;
        let (s, c) = (sin(plane.rotation_deg * DEG), cos(plane.rotation_deg * DEG));
        let positive_zero = |value: f64| if value == 0.0 { 0.0 } else { value };
        let cd = [
            [positive_zero(-degrees * c), positive_zero(-degrees * s)],
            [positive_zero(-degrees * s), positive_zero(degrees * c)],
        ];
        let crval = offset_sky(FIELD, plane.crval_offset);
        let [u, v] = solve(cd, project(crval, FIELD));
        Wcs {
            crpix: [plane.centre_pixel[0] - u, plane.centre_pixel[1] - v],
            crval,
            cd,
            sip: plane.sip,
        }
    }

    fn pixel_to_world(&self, x: f64, y: f64) -> [f64; 2] {
        let (mut u, mut v) = (x + 1.0 - self.crpix[0], y + 1.0 - self.crpix[1]);
        if self.sip {
            let (du, dv) = (sip_terms(&SIP_A, u, v)[0], sip_terms(&SIP_B, u, v)[0]);
            u += du;
            v += dv;
        }
        deproject(
            self.crval,
            [
                self.cd[0][0] * u + self.cd[0][1] * v,
                self.cd[1][0] * u + self.cd[1][1] * v,
            ],
        )
    }

    fn world_to_pixel(&self, sky: [f64; 2]) -> [f64; 2] {
        let [target_u, target_v] = solve(self.cd, project(self.crval, sky));
        let (mut u, mut v) = (target_u, target_v);
        if self.sip {
            for _ in 0..60 {
                let next = [
                    target_u - sip_terms(&SIP_A, u, v)[0],
                    target_v - sip_terms(&SIP_B, u, v)[0],
                ];
                u = next[0];
                v = next[1];
            }
        }
        [u + self.crpix[0] - 1.0, v + self.crpix[1] - 1.0]
    }

    fn relative_pixel_area(&self, x: f64, y: f64) -> f64 {
        if !self.sip {
            return 1.0;
        }
        let (u, v) = (x + 1.0 - self.crpix[0], y + 1.0 - self.crpix[1]);
        let [_, fu, fv] = sip_terms(&SIP_A, u, v);
        let [_, gu, gv] = sip_terms(&SIP_B, u, v);
        ((1.0 + fu) * (1.0 + gv) - fv * gu).abs()
    }

    fn pixel_area_arcsec2(&self, x: f64, y: f64) -> f64 {
        let linear =
            (self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0]).abs() * 3600.0 * 3600.0;
        linear * self.relative_pixel_area(x, y)
    }
}

fn gaussian_fwhm_per_sigma() -> f64 {
    2.0 * (2.0 * log(2.0)).sqrt()
}

fn airy_fwhm_per_lambda_over_d() -> f64 {
    let excess = |x: f64| {
        let amplitude = 2.0 * j1(x) / x;
        amplitude * amplitude - 0.5
    };
    let (mut low, mut high) = (1.0f64, 2.0f64);
    for _ in 0..200 {
        let middle = 0.5 * (low + high);
        if excess(middle) > 0.0 {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / PI
}

fn moffat_alpha(fwhm: f64, beta: f64) -> f64 {
    fwhm / (2.0 * (pow(2.0, 1.0 / beta) - 1.0).sqrt())
}

struct Shape {
    profile: Profile,
    width: f64,
}

impl Shape {
    fn of(plane: &Plane) -> Shape {
        let width = match plane.profile {
            Profile::Gaussian => plane.fwhm / gaussian_fwhm_per_sigma(),
            Profile::Moffat(beta) => moffat_alpha(plane.fwhm, beta),
            Profile::Airy => plane.fwhm / airy_fwhm_per_lambda_over_d(),
        };
        Shape {
            profile: plane.profile,
            width,
        }
    }

    fn value(&self, radius: f64) -> f64 {
        let scaled = radius / self.width;
        match self.profile {
            Profile::Gaussian => exp(-0.5 * scaled * scaled),
            Profile::Moffat(beta) => pow(1.0 + scaled * scaled, -beta),
            Profile::Airy => {
                let x = PI * scaled;
                if x < 1e-9 {
                    1.0
                } else {
                    let amplitude = 2.0 * j1(x) / x;
                    amplitude * amplitude
                }
            }
        }
    }
}

struct Stamp {
    x0: i64,
    y0: i64,
    side: usize,
    values: Vec<f64>,
}

impl Stamp {
    fn bounds(&self) -> [i64; 4] {
        [
            self.x0,
            self.x0 + self.side as i64,
            self.y0,
            self.y0 + self.side as i64,
        ]
    }
}

fn half_width(scale: f64) -> usize {
    (TRUNCATION_ARCSEC / scale).round() as usize
}

fn subsample(index: usize) -> f64 {
    (index as f64 + 0.5) / SUBSAMPLING as f64 - 0.5
}

fn point_stamp(shape: &Shape, scale: f64, centre: [f64; 2]) -> Stamp {
    let half = half_width(scale) as i64;
    let (cx, cy) = (
        (centre[0] + 0.5).floor() as i64,
        (centre[1] + 0.5).floor() as i64,
    );
    let side = (2 * half + 1) as usize;
    let mut values = Vec::with_capacity(side * side);
    for py in cy - half..=cy + half {
        for px in cx - half..=cx + half {
            let mut sum = 0.0;
            for b in 0..SUBSAMPLING {
                for a in 0..SUBSAMPLING {
                    let dx = px as f64 + subsample(a) - centre[0];
                    let dy = py as f64 + subsample(b) - centre[1];
                    sum += shape.value(scale * (dx * dx + dy * dy).sqrt());
                }
            }
            values.push(sum);
        }
    }
    let total: f64 = values.iter().sum();
    values.iter_mut().for_each(|value| *value /= total);
    Stamp {
        x0: cx - half,
        y0: cy - half,
        side,
        values,
    }
}

fn centred_psf(shape: &Shape, scale: f64) -> Stamp {
    let half = half_width(scale) as f64;
    point_stamp(shape, scale, [half, half])
}

fn disc_peak(index: usize) -> f64 {
    let edge = DISC_TRUNCATION / DISC_SCALE;
    let enclosed = 1.0 - (1.0 + edge) * exp(-edge);
    DISC_FLUX * DISC_SED[index] / (2.0 * PI * DISC_AXIS_RATIO * DISC_SCALE * DISC_SCALE * enclosed)
}

fn galaxy_brightness(index: usize, peak: f64, offset: [f64; 2]) -> f64 {
    let (s, c) = (sin(DISC_PA_DEG * DEG), cos(DISC_PA_DEG * DEG));
    let major = offset[0] * s + offset[1] * c;
    let minor = (offset[0] * c - offset[1] * s) / DISC_AXIS_RATIO;
    let radius = (major * major + minor * minor).sqrt();
    let disc = if radius <= DISC_TRUNCATION {
        peak * exp(-radius / DISC_SCALE)
    } else {
        0.0
    };
    CLUMPS.iter().fold(disc, |sum, clump| {
        let (dx, dy) = (offset[0] - clump.offset[0], offset[1] - clump.offset[1]);
        let variance = clump.sigma * clump.sigma;
        sum + clump.flux * CLUMP_SED[index] / (2.0 * PI * variance)
            * exp(-(dx * dx + dy * dy) / (2.0 * variance))
    })
}

fn galaxy_stamp(index: usize, wcs: &Wcs, scale: f64, centre_sky: [f64; 2]) -> Stamp {
    let [gx, gy] = wcs.world_to_pixel(centre_sky);
    let half = ((DISC_TRUNCATION + 0.05) / scale).ceil() as i64 + 2;
    let (cx, cy) = ((gx + 0.5).floor() as i64, (gy + 0.5).floor() as i64);
    let peak = disc_peak(index);
    let side = (2 * half + 1) as usize;
    let weight = 1.0 / (SUBSAMPLING * SUBSAMPLING) as f64;
    let mut values = Vec::with_capacity(side * side);
    for py in cy - half..=cy + half {
        for px in cx - half..=cx + half {
            let mut sum = 0.0;
            for b in 0..SUBSAMPLING {
                for a in 0..SUBSAMPLING {
                    let (x, y) = (px as f64 + subsample(a), py as f64 + subsample(b));
                    let [xi, eta] = project(centre_sky, wcs.pixel_to_world(x, y));
                    sum += galaxy_brightness(index, peak, [xi * 3600.0, eta * 3600.0])
                        * wcs.pixel_area_arcsec2(x, y);
                }
            }
            values.push(sum * weight);
        }
    }
    Stamp {
        x0: cx - half,
        y0: cy - half,
        side,
        values,
    }
}

fn pixel_index(size: usize, x: i64, y: i64) -> usize {
    assert!(
        (0..size as i64).contains(&x) && (0..size as i64).contains(&y),
        "pixel ({x}, {y}) is outside a {size} px plane"
    );
    y as usize * size + x as usize
}

fn add_stamp(model: &mut [f64], size: usize, stamp: &Stamp, flux: f64) {
    for j in 0..stamp.side {
        for i in 0..stamp.side {
            model[pixel_index(size, stamp.x0 + i as i64, stamp.y0 + j as i64)] +=
                flux * stamp.values[j * stamp.side + i];
        }
    }
}

fn add_convolved(model: &mut [f64], size: usize, source: &Stamp, kernel: &Stamp) {
    let half = (kernel.side / 2) as i64;
    for j in 0..source.side {
        for i in 0..source.side {
            let value = source.values[j * source.side + i];
            for kj in 0..kernel.side {
                for ki in 0..kernel.side {
                    let x = source.x0 + i as i64 + ki as i64 - half;
                    let y = source.y0 + j as i64 + kj as i64 - half;
                    model[pixel_index(size, x, y)] += value * kernel.values[kj * kernel.side + ki];
                }
            }
        }
    }
}

pub struct Rendered {
    pub model_jy: Vec<f64>,
    galaxy_jy: f64,
    galaxy_centre: [f64; 2],
    stars: Vec<([f64; 2], [i64; 4])>,
}

pub fn world_to_pixel(index: usize, sky: [f64; 2]) -> [f64; 2] {
    Wcs::of(&PLANES[index]).world_to_pixel(sky)
}

pub fn pixel_to_world(index: usize, x: f64, y: f64) -> [f64; 2] {
    Wcs::of(&PLANES[index]).pixel_to_world(x, y)
}

pub fn render(index: usize) -> Rendered {
    render_scene(index, index, index)
}

pub fn render_scene(sed: usize, grid: usize, psf: usize) -> Rendered {
    let plane = &PLANES[grid];
    let wcs = Wcs::of(plane);
    let shape = Shape::of(&PLANES[psf]);
    let size = plane.size;
    let mut model_jy = vec![0.0; size * size];
    let mut stars = Vec::new();
    for star in &STARS {
        let position = wcs.world_to_pixel(offset_sky(FIELD, star.offset));
        let stamp = point_stamp(&shape, plane.scale, position);
        add_stamp(&mut model_jy, size, &stamp, star.flux * STAR_SED[sed]);
        stars.push((position, stamp.bounds()));
    }
    let centre_sky = offset_sky(FIELD, GALAXY_OFFSET);
    let mut galaxy = vec![0.0; size * size];
    add_convolved(
        &mut galaxy,
        size,
        &galaxy_stamp(sed, &wcs, plane.scale, centre_sky),
        &centred_psf(&shape, plane.scale),
    );
    let relative_area: Vec<f64> = (0..size * size)
        .map(|pixel| wcs.relative_pixel_area((pixel % size) as f64, (pixel / size) as f64))
        .collect();
    model_jy
        .iter_mut()
        .zip(&galaxy)
        .zip(&relative_area)
        .for_each(|((value, light), area)| *value = (*value + light) / area);
    Rendered {
        model_jy,
        galaxy_jy: galaxy
            .iter()
            .zip(&relative_area)
            .map(|(light, area)| light / area)
            .sum(),
        galaxy_centre: wcs.world_to_pixel(centre_sky),
        stars,
    }
}

const STAR_FIELD_SIZE: usize = 180;
const STAR_FIELD_NOISE: usize = 16;
const STAR_FIELD_SKY_JY: f64 = 5.0e-9;

pub struct FieldStar {
    pub name: &'static str,
    pub position: [f64; 2],
    pub flux_jy: f64,
}

const FIELD_STARS: [FieldStar; 10] = [
    FieldStar {
        name: "f1",
        position: [30.21, 30.67],
        flux_jy: 2.0e-5,
    },
    FieldStar {
        name: "f2",
        position: [80.48, 29.86],
        flux_jy: 1.1e-5,
    },
    FieldStar {
        name: "f3",
        position: [129.73, 30.32],
        flux_jy: 1.6e-5,
    },
    FieldStar {
        name: "f4",
        position: [29.94, 80.12],
        flux_jy: 0.9e-5,
    },
    FieldStar {
        name: "f5",
        position: [80.36, 80.55],
        flux_jy: 3.0e-5,
    },
    FieldStar {
        name: "f6",
        position: [130.09, 79.71],
        flux_jy: 1.3e-5,
    },
    FieldStar {
        name: "f7",
        position: [30.58, 129.88],
        flux_jy: 1.8e-5,
    },
    FieldStar {
        name: "f8",
        position: [79.67, 130.44],
        flux_jy: 1.0e-5,
    },
    FieldStar {
        name: "f9",
        position: [130.42, 130.03],
        flux_jy: 2.4e-5,
    },
    FieldStar {
        name: "edge",
        position: [169.0, 80.0],
        flux_jy: 2.0e-5,
    },
];
const FIELD_HOLES: [[i64; 4]; 2] = [[80, 82, 80, 82], [143, 145, 143, 145]];

pub struct StarField {
    pub size: usize,
    pub pixel_scale: f64,
    pub sigma_jy: f64,
    pub sci: Vec<f32>,
    pub stars: &'static [FieldStar],
    pub truth_side: usize,
    pub truth_psf: Vec<f64>,
}

struct FieldLayout {
    cols: usize,
    rows: usize,
    stars: &'static [FieldStar],
    holes: &'static [[i64; 4]],
    noise: usize,
}

const STAR_FIELD: FieldLayout = FieldLayout {
    cols: STAR_FIELD_SIZE,
    rows: STAR_FIELD_SIZE,
    stars: &FIELD_STARS,
    holes: &FIELD_HOLES,
    noise: STAR_FIELD_NOISE,
};

const ORACLE_FIELD_STARS: [FieldStar; 6] = [
    FieldStar {
        name: "s1",
        position: [18.21, 18.67],
        flux_jy: 2.0e-5,
    },
    FieldStar {
        name: "s2",
        position: [46.48, 17.86],
        flux_jy: 1.1e-5,
    },
    FieldStar {
        name: "s3",
        position: [74.73, 18.32],
        flux_jy: 1.6e-5,
    },
    FieldStar {
        name: "s4",
        position: [17.94, 46.12],
        flux_jy: 0.9e-5,
    },
    FieldStar {
        name: "s5",
        position: [46.36, 46.55],
        flux_jy: 3.0e-5,
    },
    FieldStar {
        name: "s6",
        position: [74.09, 45.71],
        flux_jy: 1.3e-5,
    },
];
const ORACLE_FIELD_HOLES: [[i64; 4]; 1] = [[87, 89, 59, 61]];
const ORACLE_FIELD: FieldLayout = FieldLayout {
    cols: 96,
    rows: 67,
    stars: &ORACLE_FIELD_STARS,
    holes: &ORACLE_FIELD_HOLES,
    noise: 17,
};
const ORACLE_FIELD_LEFT_OUT: &str = "s6";
const ORACLE_FIELD_LEFT_OUT_REASON: &str = "its LIGHTSTACK cutout holds the NaN hole in the background border; recenter_cutouts spreads the NaN over the whole cutout and build_empirical_psf (LIGHTSTACK 0.4.0) raises ValueError: Invalid normalization flux for star 's6': 0.0, so the region file leaves the star out as a user would";
const ORACLE_FIELD_BUNIT: &str = "Jy";
const ORACLE_FIELD_REGION_HEADER: &str = "# Region file format: DS9 version 4.1";
const ORACLE_FIELD_REGION_GLOBAL: &str = "global color=green dashlist=8 3 width=1 font=\"helvetica 10 normal roman\" select=1 highlite=1 dash=0 fixed=0 edit=1 move=1 delete=1 include=1 source=1";
const ORACLE_FIELD_RECIPE: &str = "recipe_d1_stars.json";
const ORACLE_FIELD_REGENERATE: &str =
    "ASTROBURST_WRITE_LIGHTSTACK_FIXTURES=1 cargo test --test lightstack_oracle regenerate_d1_star_field_fixture -- --ignored";
const ORACLE_CUTOUT: usize = 31;
const ORACLE_BORDER: usize = 5;
const ORACLE_RADIUS_ARCSEC: f64 = 0.46;
const ORACLE_PSF_SIZE: usize = 21;
const ORACLE_BOX_PX: usize = 9;

fn render_field(plane: &Plane, layout: &FieldLayout) -> Vec<f32> {
    let shape = Shape::of(plane);
    let (cols, rows) = (layout.cols, layout.rows);
    let mut model_jy = vec![STAR_FIELD_SKY_JY; cols * rows];
    for star in layout.stars {
        let stamp = point_stamp(&shape, plane.scale, star.position);
        for j in 0..stamp.side {
            for i in 0..stamp.side {
                let (x, y) = (stamp.x0 + i as i64, stamp.y0 + j as i64);
                assert!(
                    (0..cols as i64).contains(&x) && (0..rows as i64).contains(&y),
                    "star {} leaves the {cols} x {rows} px field",
                    star.name
                );
                model_jy[y as usize * cols + x as usize] +=
                    star.flux_jy * stamp.values[j * stamp.side + i];
            }
        }
    }
    let mut normal = Normal::new(layout.noise);
    let mut sci: Vec<f32> = model_jy
        .iter()
        .map(|value| (value + plane.sigma0_jy * normal.next()) as f32)
        .collect();
    for y in 0..rows {
        for x in 0..cols {
            let on_border = x < NAN_BORDER
                || y < NAN_BORDER
                || x >= cols - NAN_BORDER
                || y >= rows - NAN_BORDER;
            let in_hole = layout.holes.iter().any(|hole| {
                (hole[0]..hole[1]).contains(&(x as i64)) && (hole[2]..hole[3]).contains(&(y as i64))
            });
            if on_border || in_hole {
                sci[y * cols + x] = f32::NAN;
            }
        }
    }
    sci
}

pub fn star_field(index: usize) -> StarField {
    let plane = &PLANES[index];
    let truth = centred_psf(&Shape::of(plane), plane.scale);
    StarField {
        size: STAR_FIELD_SIZE,
        pixel_scale: plane.scale,
        sigma_jy: plane.sigma0_jy,
        sci: render_field(plane, &STAR_FIELD),
        stars: &FIELD_STARS,
        truth_side: truth.side,
        truth_psf: truth.values,
    }
}

pub fn generate_star_field(dir: &Path) -> Value {
    let plane = PLANES
        .iter()
        .find(|plane| plane.filter == REFERENCE)
        .unwrap();
    let layout = &ORACLE_FIELD;
    let (cols, rows) = (layout.cols, layout.rows);
    let wcs = Wcs {
        crpix: [(cols as f64 + 1.0) / 2.0, (rows as f64 + 1.0) / 2.0],
        crval: FIELD,
        cd: Wcs::of(plane).cd,
        sip: false,
    };
    let image = format!("d1_stars_{}_{}.fits", plane.dataset, plane.filter);
    let region = format!("stars_{}.reg", plane.filter.to_lowercase());
    let mut bytes = header(vec![
        logical("SIMPLE", true),
        integer("BITPIX", 8),
        integer("NAXIS", 0),
        logical("EXTEND", true),
        text("ORIGIN", "AstroBurst D1 synthetic"),
        text("TELESCOP", plane.telescope),
        text("INSTRUME", plane.instrument),
        text("FILTER", plane.filter),
        text("PUPIL", "CLEAR"),
        integer("D1SEED", SEED as i64),
        history("D1 star field for the LIGHTSTACK empirical PSF oracle"),
        history("Truth in manifest.json"),
    ]);
    let mut sci_cards = vec![text("BUNIT", ORACLE_FIELD_BUNIT)];
    sci_cards.extend(wcs_cards(&wcs));
    bytes.extend(header(extension("SCI", -32, [cols, rows], sci_cards)));
    bytes.extend(data(
        render_field(plane, layout)
            .iter()
            .map(|value| value.to_be_bytes()),
    ));
    let boxes: String = layout
        .stars
        .iter()
        .filter(|star| star.name != ORACLE_FIELD_LEFT_OUT)
        .map(|star| {
            format!(
                "box({:.4},{:.4},{ORACLE_BOX_PX},{ORACLE_BOX_PX},0) # text={{{}}}\n",
                star.position[0] + 1.0,
                star.position[1] + 1.0,
                star.name
            )
        })
        .collect();
    let recipe = format!(
        "{{\n  \"recipe_version\": 1,\n  \"name\": \"d1_stars\",\n  \"inputs\": [\n    {{\"path\": \"{image}\", \"dataset\": \"{}\", \"filter\": \"{}\"}}\n  ],\n  \"steps\": [\n    {{\"step\": \"psfs\", \"source\": \"empirical\", \"dataset\": \"{}\", \"filters\": [\"{}\"], \"cutout_size\": {ORACLE_CUTOUT}, \"background_border\": {ORACLE_BORDER}, \"normalization_radius\": {ORACLE_RADIUS_ARCSEC:?}, \"normalization_radius_unit\": \"arcsec\", \"psf_size\": {ORACLE_PSF_SIZE}}}\n  ]\n}}\n",
        plane.dataset, plane.filter, plane.dataset, plane.filter
    );
    let shape = Shape::of(plane);
    let truth = json!({
        "seed": SEED,
        "noise_stream": layout.noise,
        "dataset": plane.dataset,
        "filter": plane.filter,
        "file": image,
        "extension": "SCI",
        "bunit": ORACLE_FIELD_BUNIT,
        "shape": [rows, cols],
        "pixel_scale_arcsec": plane.scale,
        "wcs": {"ctype": ["RA---TAN", "DEC--TAN"], "crpix": wcs.crpix, "crval_deg": wcs.crval, "cd_deg": wcs.cd},
        "sky_jy": STAR_FIELD_SKY_JY,
        "sigma_jy": plane.sigma0_jy,
        "nan_border_px": NAN_BORDER,
        "holes": layout.holes.iter().map(|hole| json!({"x": [hole[0], hole[1]], "y": [hole[2], hole[3]]})).collect::<Vec<_>>(),
        "psf": {
            "model": model_name(plane.profile),
            "fwhm_arcsec": plane.fwhm,
            "lambda_over_d_arcsec": shape.width,
            "truncation_arcsec": TRUNCATION_ARCSEC,
            "subsampling": SUBSAMPLING,
            "side": centred_psf(&shape, plane.scale).side,
        },
        "positions": "0-based pixel centres; the region file lists them 1-based, as DS9 physical coordinates",
        "region_file": region,
        "stars": layout.stars.iter().map(|star| json!({"name": star.name, "x": star.position[0], "y": star.position[1], "flux_jy": star.flux_jy, "in_region_file": star.name != ORACLE_FIELD_LEFT_OUT})).collect::<Vec<_>>(),
        "left_out": {"star": ORACLE_FIELD_LEFT_OUT, "reason": ORACLE_FIELD_LEFT_OUT_REASON},
    });
    let files = vec![
        (image, bytes),
        (
            region,
            format!(
                "{ORACLE_FIELD_REGION_HEADER}\n{ORACLE_FIELD_REGION_GLOBAL}\nphysical\n{boxes}"
            )
            .into_bytes(),
        ),
        (ORACLE_FIELD_RECIPE.to_owned(), recipe.into_bytes()),
    ];
    write_fixture(
        dir,
        Written { files, truth },
        "d1_star_field",
        ORACLE_FIELD_REGENERATE,
    )
}

struct Normal {
    seed: u64,
    counter: u64,
    spare: Option<f64>,
}

impl Normal {
    fn new(index: usize) -> Normal {
        Normal {
            seed: derive_seed(SEED, SeedPurpose::Noise, index as u32),
            counter: 0,
            spare: None,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.counter += 1;
        ((splitmix64(self.seed.wrapping_add(self.counter)) >> 11) as f64 + 0.5)
            / 9_007_199_254_740_992.0
    }

    fn next(&mut self) -> f64 {
        if let Some(value) = self.spare.take() {
            return value;
        }
        let radius = (-2.0 * log(self.uniform())).sqrt();
        let angle = 2.0 * PI * self.uniform();
        self.spare = Some(radius * sin(angle));
        radius * cos(angle)
    }
}

fn sigma_jy(plane: &Plane, x: usize) -> f64 {
    plane.sigma0_jy * (0.85 + 0.30 * x as f64 / (plane.size - 1) as f64)
}

fn pixar_sr(scale: f64) -> f64 {
    let side = scale * PI / 648_000.0;
    side * side
}

fn photfnu() -> f64 {
    PHOTFLAM * PHOTPLAM * PHOTPLAM / C_ANGSTROM_PER_S / 1e-23
}

fn jy_per_unit(plane: &Plane) -> f64 {
    match plane.unit {
        Unit::MjySr => 1e6 * pixar_sr(plane.scale),
        Unit::JyPixel => 1.0,
        Unit::ElectronsPerSecond => photfnu(),
    }
}

fn bunit(unit: Unit) -> &'static str {
    match unit {
        Unit::MjySr => "MJy/sr",
        Unit::JyPixel => "Jy",
        Unit::ElectronsPerSecond => "ELECTRONS/S",
    }
}

fn fits_real(value: f64) -> String {
    let text = format!("{value:?}").replace('e', "E");
    match text.split_once('E') {
        Some((mantissa, exponent)) if !mantissa.contains('.') => format!("{mantissa}.0E{exponent}"),
        _ => text,
    }
}

fn card(text: String) -> String {
    assert!(
        text.len() <= 80 && text.is_ascii(),
        "bad FITS card {text:?}"
    );
    format!("{text:<80}")
}

fn logical(key: &str, value: bool) -> String {
    card(format!("{key:<8}= {:>20}", if value { "T" } else { "F" }))
}

fn integer(key: &str, value: i64) -> String {
    card(format!("{key:<8}= {value:>20}"))
}

fn real(key: &str, value: f64) -> String {
    card(format!("{key:<8}= {:>20}", fits_real(value)))
}

fn text(key: &str, value: &str) -> String {
    card(format!("{key:<8}= '{:<8}'", value.replace('\'', "''")))
}

fn history(value: &str) -> String {
    card(format!("HISTORY {value}"))
}

fn pad(bytes: &mut Vec<u8>, fill: u8) {
    let rest = bytes.len() % BLOCK;
    if rest != 0 {
        bytes.resize(bytes.len() + BLOCK - rest, fill);
    }
}

fn header(cards: Vec<String>) -> Vec<u8> {
    let mut bytes: Vec<u8> = cards.concat().into_bytes();
    bytes.extend(card("END".to_owned()).into_bytes());
    pad(&mut bytes, b' ');
    bytes
}

fn data<const N: usize>(values: impl Iterator<Item = [u8; N]>) -> Vec<u8> {
    let mut bytes: Vec<u8> = values.flatten().collect();
    pad(&mut bytes, 0);
    bytes
}

fn wcs_cards(wcs: &Wcs) -> Vec<String> {
    let suffix = if wcs.sip { "-SIP" } else { "" };
    let mut cards = vec![
        integer("WCSAXES", 2),
        text("CTYPE1", &format!("RA---TAN{suffix}")),
        text("CTYPE2", &format!("DEC--TAN{suffix}")),
        text("CUNIT1", "deg"),
        text("CUNIT2", "deg"),
        real("CRPIX1", wcs.crpix[0]),
        real("CRPIX2", wcs.crpix[1]),
        real("CRVAL1", wcs.crval[0]),
        real("CRVAL2", wcs.crval[1]),
        real("CD1_1", wcs.cd[0][0]),
        real("CD1_2", wcs.cd[0][1]),
        real("CD2_1", wcs.cd[1][0]),
        real("CD2_2", wcs.cd[1][1]),
        text("RADESYS", "ICRS"),
    ];
    if wcs.sip {
        for (prefix, terms) in [("A", &SIP_A), ("B", &SIP_B)] {
            cards.push(integer(&format!("{prefix}_ORDER"), 2));
            cards.extend(
                terms
                    .iter()
                    .map(|&(p, q, c)| real(&format!("{prefix}_{p}_{q}"), c)),
            );
        }
    }
    cards
}

fn unit_cards(plane: &Plane) -> Vec<String> {
    match plane.unit {
        Unit::MjySr => vec![
            real("PIXAR_SR", pixar_sr(plane.scale)),
            real("PIXAR_A2", plane.scale * plane.scale),
        ],
        Unit::JyPixel => Vec::new(),
        Unit::ElectronsPerSecond => vec![
            real("PHOTFLAM", PHOTFLAM),
            real("PHOTPLAM", PHOTPLAM),
            real("PHOTFNU", photfnu()),
        ],
    }
}

fn extension(name: &str, bitpix: i64, [cols, rows]: [usize; 2], extra: Vec<String>) -> Vec<String> {
    let mut cards = vec![
        text("XTENSION", "IMAGE"),
        integer("BITPIX", bitpix),
        integer("NAXIS", 2),
        integer("NAXIS1", cols as i64),
        integer("NAXIS2", rows as i64),
        integer("PCOUNT", 0),
        integer("GCOUNT", 1),
        text("EXTNAME", name),
        integer("EXTVER", 1),
    ];
    cards.extend(extra);
    cards
}

fn plane_file(plane: &Plane) -> String {
    format!("d1_{}_{}.fits", plane.dataset, plane.filter)
}

fn psf_file(plane: &Plane, scale: f64) -> String {
    format!(
        "psf/d1_psf_{}_{}_{}mas.fits",
        plane.dataset,
        plane.filter,
        (scale * 1000.0).round() as i64
    )
}

fn psf_scales(plane: &Plane) -> Vec<f64> {
    if plane.scale == CUBE_SCALE {
        vec![CUBE_SCALE]
    } else {
        vec![plane.scale, CUBE_SCALE]
    }
}

fn model_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Gaussian => "gaussian",
        Profile::Moffat(_) => "moffat",
        Profile::Airy => "airy",
    }
}

fn psf_bytes(plane: &Plane, shape: &Shape, scale: f64) -> (Vec<u8>, usize) {
    let stamp = centred_psf(shape, scale);
    let mut cards = vec![
        logical("SIMPLE", true),
        integer("BITPIX", -64),
        integer("NAXIS", 2),
        integer("NAXIS1", stamp.side as i64),
        integer("NAXIS2", stamp.side as i64),
        integer("WCSAXES", 2),
        text("CTYPE1", "RA---TAN"),
        text("CTYPE2", "DEC--TAN"),
        text("CUNIT1", "deg"),
        text("CUNIT2", "deg"),
        real("CRVAL1", 0.0),
        real("CRVAL2", 0.0),
        real("CRPIX1", (stamp.side as f64 + 1.0) / 2.0),
        real("CRPIX2", (stamp.side as f64 + 1.0) / 2.0),
        real("CD1_1", -scale / 3600.0),
        real("CD1_2", 0.0),
        real("CD2_1", 0.0),
        real("CD2_2", scale / 3600.0),
        text("FILTER", plane.filter),
        text("DATASET", plane.dataset),
        real("PIXSCALE", scale),
        text("PSFMODEL", &model_name(plane.profile).to_uppercase()),
        real("PSFFWHM", plane.fwhm),
    ];
    if let Profile::Moffat(beta) = plane.profile {
        cards.push(real("PSFBETA", beta));
    }
    cards.push(real("PSFTRUNC", TRUNCATION_ARCSEC));
    cards.push(integer("PSFNSUB", SUBSAMPLING as i64));
    cards.push(history(
        "D1 truth PSF: analytic profile averaged over pixels, truncated, sum 1",
    ));
    let mut bytes = header(cards);
    bytes.extend(data(stamp.values.iter().map(|value| value.to_be_bytes())));
    (bytes, stamp.side)
}

fn disjoint(a: [i64; 4], b: [i64; 4]) -> bool {
    a[1] <= b[0] || b[1] <= a[0] || a[3] <= b[2] || b[3] <= a[2]
}

fn within_interior(rect: [i64; 4], size: usize) -> bool {
    let (low, high) = (NAN_BORDER as i64, (size - NAN_BORDER) as i64);
    rect[0] >= low && rect[1] <= high && rect[2] >= low && rect[3] <= high
}

struct Written {
    files: Vec<(String, Vec<u8>)>,
    truth: Value,
}

fn write_plane(index: usize, written: &mut Vec<(String, Vec<u8>)>) -> Value {
    let plane = &PLANES[index];
    let wcs = Wcs::of(plane);
    let shape = Shape::of(plane);
    let size = plane.size;
    let rendered = render(index);
    let factor = jy_per_unit(plane);
    let star_boxes: Vec<[i64; 4]> = rendered.stars.iter().map(|(_, bounds)| *bounds).collect();
    for bounds in &star_boxes {
        assert!(
            within_interior(*bounds, size),
            "{}: a star box {bounds:?} leaves the interior",
            plane.filter
        );
    }
    let holes: Vec<[i64; 4]> = HOLES
        .iter()
        .map(|hole| {
            let [x, y] = wcs.world_to_pixel(offset_sky(FIELD, hole.offset));
            let x0 = (x + 0.5).floor() as i64 - (hole.width as i64 - 1) / 2;
            let y0 = (y + 0.5).floor() as i64 - (hole.height as i64 - 1) / 2;
            [x0, x0 + hole.width as i64, y0, y0 + hole.height as i64]
        })
        .collect();
    let flagged: Vec<[i64; 2]> = FLAGGED
        .iter()
        .map(|offset| {
            let [x, y] = wcs.world_to_pixel(offset_sky(FIELD, *offset));
            [(x + 0.5).floor() as i64, (y + 0.5).floor() as i64]
        })
        .collect();
    let marks: Vec<[i64; 4]> = holes
        .iter()
        .copied()
        .chain(flagged.iter().map(|[x, y]| [*x, x + 1, *y, y + 1]))
        .collect();
    for (number, mark) in marks.iter().enumerate() {
        assert!(
            within_interior(*mark, size),
            "{}: mark {mark:?} leaves the interior",
            plane.filter
        );
        assert!(
            star_boxes.iter().all(|bounds| disjoint(*mark, *bounds)),
            "{}: mark {mark:?} touches a star",
            plane.filter
        );
        assert!(
            marks[number + 1..]
                .iter()
                .all(|other| disjoint(*mark, *other)),
            "{}: marks overlap",
            plane.filter
        );
    }
    let mut normal = Normal::new(index);
    let (mut sci, mut err, mut dq) = (
        vec![0f32; size * size],
        vec![0f32; size * size],
        vec![0i32; size * size],
    );
    for y in 0..size {
        for x in 0..size {
            let pixel = y * size + x;
            let sigma = sigma_jy(plane, x) / factor;
            sci[pixel] = (rendered.model_jy[pixel] / factor + sigma * normal.next()) as f32;
            err[pixel] = sigma as f32;
        }
    }
    let inside = |rect: &[i64; 4], x: usize, y: usize| {
        (rect[0]..rect[1]).contains(&(x as i64)) && (rect[2]..rect[3]).contains(&(y as i64))
    };
    for y in 0..size {
        for x in 0..size {
            let pixel = y * size + x;
            let on_border = x < NAN_BORDER
                || y < NAN_BORDER
                || x >= size - NAN_BORDER
                || y >= size - NAN_BORDER;
            if on_border || holes.iter().any(|hole| inside(hole, x, y)) {
                sci[pixel] = f32::NAN;
                err[pixel] = f32::NAN;
                dq[pixel] = if on_border { BORDER_DQ } else { HOLE_DQ };
            } else if flagged
                .iter()
                .any(|[fx, fy]| *fx == x as i64 && *fy == y as i64)
            {
                dq[pixel] = FLAG_DQ;
            }
        }
    }
    let mut primary = vec![
        logical("SIMPLE", true),
        integer("BITPIX", 8),
        integer("NAXIS", 0),
        logical("EXTEND", true),
        text("ORIGIN", "AstroBurst D1 synthetic"),
        text("TELESCOP", plane.telescope),
        text("INSTRUME", plane.instrument),
        text("FILTER", plane.filter),
    ];
    if plane.telescope == "JWST" {
        primary.push(text("PUPIL", "CLEAR"));
    }
    primary.push(integer("D1SEED", SEED as i64));
    primary.push(history(
        "Synthetic D1 plane for the LIGHTSTACK oracle; truth in manifest.json",
    ));
    let mut bytes = header(primary);
    let mut sci_cards = vec![text("BUNIT", bunit(plane.unit))];
    sci_cards.extend(wcs_cards(&wcs));
    sci_cards.extend(unit_cards(plane));
    bytes.extend(header(extension("SCI", -32, [size, size], sci_cards)));
    bytes.extend(data(sci.iter().map(|value| value.to_be_bytes())));
    let mut err_cards = vec![text("BUNIT", bunit(plane.unit))];
    err_cards.extend(wcs_cards(&wcs));
    bytes.extend(header(extension("ERR", -32, [size, size], err_cards)));
    bytes.extend(data(err.iter().map(|value| value.to_be_bytes())));
    bytes.extend(header(extension("DQ", 32, [size, size], wcs_cards(&wcs))));
    bytes.extend(data(dq.iter().map(|value| value.to_be_bytes())));
    written.push((plane_file(plane), bytes));
    let mut psf_files = Vec::new();
    for scale in psf_scales(plane) {
        let (psf, side) = psf_bytes(plane, &shape, scale);
        written.push((psf_file(plane, scale), psf));
        psf_files.push(
            json!({"path": psf_file(plane, scale), "pixel_scale_arcsec": scale, "side": side}),
        );
    }
    let radius_px = |scale: f64| APERTURE_FWHM * plane.fwhm / scale;
    let stars: Vec<Value> = STARS
        .iter()
        .zip(&rendered.stars)
        .map(|(star, ([x, y], _))| {
            let radius = radius_px(plane.scale);
            let alone = point_stamp(&shape, plane.scale, [*x, *y]);
            let (mut model, mut own, mut variance) = (0.0, 0.0, 0.0);
            for py in 0..size {
                for px in 0..size {
                    let (dx, dy) = (px as f64 - x, py as f64 - y);
                    if dx * dx + dy * dy <= radius * radius {
                        let sigma = sigma_jy(plane, px);
                        let (i, j) = (px as i64 - alone.x0, py as i64 - alone.y0);
                        if (0..alone.side as i64).contains(&i)
                            && (0..alone.side as i64).contains(&j)
                        {
                            own += star.flux
                                * STAR_SED[index]
                                * alone.values[j as usize * alone.side + i as usize]
                                / wcs.relative_pixel_area(px as f64, py as f64);
                        }
                        model += rendered.model_jy[py * size + px];
                        variance += sigma * sigma;
                    }
                }
            }
            assert!(
                model / variance.sqrt() > SNR_FLOOR,
                "{} {}: S/N {}",
                plane.filter,
                star.id,
                model / variance.sqrt()
            );
            json!({
                "id": star.id,
                "x": x,
                "y": y,
                "flux_jy": star.flux * STAR_SED[index],
                "aperture_radius_px": radius,
                "aperture_model_jy": model,
                "aperture_star_jy": own,
                "aperture_sigma_jy": variance.sqrt(),
            })
        })
        .collect();
    let hole_truth: Vec<Value> = holes
        .iter()
        .map(|hole| {
            let mut model = 0.0;
            for y in hole[2]..hole[3] {
                for x in hole[0]..hole[1] {
                    model += rendered.model_jy[pixel_index(size, x, y)];
                }
            }
            json!({"x": [hole[0], hole[1]], "y": [hole[2], hole[3]], "dq": HOLE_DQ, "model_jy": model})
        })
        .collect();
    let width = match plane.profile {
        Profile::Gaussian => json!({"sigma_arcsec": shape.width}),
        Profile::Moffat(beta) => json!({"alpha_arcsec": shape.width, "beta": beta}),
        Profile::Airy => json!({"lambda_over_d_arcsec": shape.width}),
    };
    let mut psf =
        json!({"model": model_name(plane.profile), "fwhm_arcsec": plane.fwhm, "files": psf_files});
    psf.as_object_mut()
        .unwrap()
        .extend(width.as_object().unwrap().clone());
    let unit_truth = match plane.unit {
        Unit::MjySr => {
            json!({"PIXAR_SR": pixar_sr(plane.scale), "PIXAR_A2": plane.scale * plane.scale})
        }
        Unit::JyPixel => json!({}),
        Unit::ElectronsPerSecond => {
            json!({"PHOTFLAM": PHOTFLAM, "PHOTPLAM": PHOTPLAM, "PHOTFNU": photfnu()})
        }
    };
    json!({
        "filter": plane.filter,
        "dataset": plane.dataset,
        "telescope": plane.telescope,
        "instrument": plane.instrument,
        "file": plane_file(plane),
        "extensions": ["SCI", "ERR", "DQ"],
        "shape": [size, size],
        "pixel_scale_arcsec": plane.scale,
        "wcs": {
            "ctype": [format!("RA---TAN{}", if plane.sip { "-SIP" } else { "" }), format!("DEC--TAN{}", if plane.sip { "-SIP" } else { "" })],
            "crpix": wcs.crpix,
            "crval_deg": wcs.crval,
            "cd_deg": wcs.cd,
            "rotation_deg": plane.rotation_deg,
            "sip": if plane.sip { json!({"a": SIP_A.iter().map(|&(p, q, c)| json!([p, q, c])).collect::<Vec<_>>(), "b": SIP_B.iter().map(|&(p, q, c)| json!([p, q, c])).collect::<Vec<_>>()}) } else { Value::Null },
        },
        "unit": {"bunit": bunit(plane.unit), "jy_per_unit": factor, "cards": unit_truth},
        "psf": psf,
        "noise": {"sigma0_jy": plane.sigma0_jy, "sigma_jy": "sigma0_jy * (0.85 + 0.30 * x / (nx - 1)), x the 0-based column"},
        "nan_border_px": NAN_BORDER,
        "border_dq": BORDER_DQ,
        "holes": hole_truth,
        "flagged": flagged.iter().map(|[x, y]| json!({"x": x, "y": y, "dq": FLAG_DQ})).collect::<Vec<_>>(),
        "stars": stars,
        "galaxy": {
            "centre_px": rendered.galaxy_centre,
            "disc_flux_jy": DISC_FLUX * DISC_SED[index],
            "clump_flux_jy": CLUMPS.iter().map(|clump| clump.flux * CLUMP_SED[index]).collect::<Vec<_>>(),
            "model_jy": rendered.galaxy_jy,
        },
        "model_jy": rendered.model_jy.iter().sum::<f64>(),
    })
}

fn recipe() -> String {
    let inputs: Vec<String> = PLANES
        .iter()
        .map(|plane| {
            format!(
                "    {{\"path\": \"{}\", \"dataset\": \"{}\", \"filter\": \"{}\"}}",
                plane_file(plane),
                plane.dataset,
                plane.filter
            )
        })
        .collect();
    let psfs: Vec<String> = PLANES
        .iter()
        .map(|plane| {
            format!(
                "        {{\"filter\": \"{}\", \"path\": \"{}\"}}",
                plane.filter,
                psf_file(plane, plane.scale)
            )
        })
        .collect();
    let steps = [
        format!("{{\"step\": \"crop\", \"mode\": \"radec\", \"ra\": {:?}, \"dec\": {:?}, \"size_arcsec\": {CROP_ARCSEC:?}}}", FIELD[0], FIELD[1]),
        "{\"step\": \"units\", \"to\": \"Jy/pixel\"}".to_owned(),
        format!("{{\"step\": \"align\", \"reference\": \"{REFERENCE}\", \"method\": \"bilinear\", \"border_crop\": 1}}"),
        format!("{{\"step\": \"cube\", \"reference\": \"{REFERENCE}\", \"order\": \"wavelength\", \"output\": \"d1_cube.fits\"}}"),
        "{\"step\": \"edit\", \"action\": \"drop_invalid\", \"threshold\": 0.0, \"frac_valid\": 0.01}".to_owned(),
        format!(
            "{{\"step\": \"psfs\", \"source\": \"files\", \"reference\": \"{REFERENCE}\", \"pixel_scale_arcsec\": {CUBE_SCALE:?}, \"files\": [\n{}\n    ]}}",
            psfs.join(",\n")
        ),
        format!("{{\"step\": \"kernels\", \"reference\": \"{REFERENCE}\", \"method\": \"wiener\", \"eps\": 0.05, \"shape\": \"reference\"}}"),
        format!("{{\"step\": \"match\", \"reference\": \"{REFERENCE}\"}}"),
    ];
    format!(
        "{{\n  \"recipe_version\": 1,\n  \"name\": \"d1\",\n  \"inputs\": [\n{}\n  ],\n  \"steps\": [\n    {}\n  ]\n}}\n",
        inputs.join(",\n"),
        steps.join(",\n    ")
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn build() -> Written {
    let mut files = Vec::new();
    let planes: Vec<Value> = (0..PLANES.len())
        .map(|index| write_plane(index, &mut files))
        .collect();
    files.push(("recipe_d1.json".to_owned(), recipe().into_bytes()));
    let centre = offset_sky(FIELD, GALAXY_OFFSET);
    let truth = json!({
        "seed": SEED,
        "field_centre_deg": FIELD,
        "reference_filter": REFERENCE,
        "cube_pixel_scale_arcsec": CUBE_SCALE,
        "psf_truncation_arcsec": TRUNCATION_ARCSEC,
        "subsampling": SUBSAMPLING,
        "aperture_radius_fwhm": APERTURE_FWHM,
        "galaxy": {
            "centre_deg": centre,
            "offset_arcsec": GALAXY_OFFSET,
            "disc": {
                "profile": "exponential",
                "scale_length_arcsec": DISC_SCALE,
                "axis_ratio": DISC_AXIS_RATIO,
                "position_angle_deg": DISC_PA_DEG,
                "truncation_arcsec": DISC_TRUNCATION,
            },
            "clumps": CLUMPS.iter().map(|clump| json!({"offset_arcsec": clump.offset, "sigma_arcsec": clump.sigma})).collect::<Vec<_>>(),
        },
        "stars": STARS.iter().map(|star| json!({"id": star.id, "offset_arcsec": star.offset, "position_deg": offset_sky(FIELD, star.offset)})).collect::<Vec<_>>(),
        "planes": planes,
    });
    Written { files, truth }
}

pub fn generate(dir: &Path) -> Value {
    write_fixture(dir, build(), "d1_synthetic", REGENERATE)
}

fn write_fixture(dir: &Path, written: Written, kind: &str, regenerate: &str) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    for (name, bytes) in &written.files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        entries.push(json!({"path": name, "bytes": bytes.len(), "sha256": sha256_hex(bytes)}));
    }
    entries.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let manifest = json!({
        "manifest_version": 1,
        "kind": kind,
        "generator": GENERATOR,
        "regenerate": regenerate,
        "files": entries,
        "truth": written.truth,
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    )
    .unwrap();
    manifest
}
