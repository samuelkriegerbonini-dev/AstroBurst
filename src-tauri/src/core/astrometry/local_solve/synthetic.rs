use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::{destination, CatalogProvider, Detection, FieldSize, RankingKey, SkyPair};
use crate::core::analysis::star_detection::DetectedStar;
use crate::core::astrometry::catalog::CatalogRow;
use crate::core::astrometry::solve_hints::{Pointing, ScaleRange, SolveHints};
use crate::core::astrometry::wcs::{angular_separation, pixel_edge_corners, WcsTransform};
use crate::core::registration::poly::TanParams;
use crate::core::synth::psf::{render_stars, GaussianPsf};
use crate::core::synth::star_field::Star;
use crate::types::header::HduHeader;

pub(crate) const ARCSEC_PER_DEG: f64 = 3600.0;
pub(crate) const HINT_SCALE_LOW: f64 = 0.92;
pub(crate) const HINT_SCALE_HIGH: f64 = 1.12;
pub(crate) const TIER1_CUT_STARS_PER_DEG2: f64 = 610.0;

const SQ_DEG_PER_SR: f64 = (180.0 / std::f64::consts::PI) * (180.0 / std::f64::consts::PI);
const MAG_SLOPE: f64 = 0.4;
const DETECTION_FWHM_PX: f64 = 3.0;
const DETECTION_ECCENTRICITY: f64 = 0.05;

pub(crate) struct SyntheticProvider {
    label: &'static str,
    rows: Vec<CatalogRow>,
    depth: f64,
    ranking: RankingKey,
}

impl SyntheticProvider {
    pub(crate) fn new(label: &'static str, mut rows: Vec<CatalogRow>, depth: f64) -> Self {
        rows.sort_by(|a, b| a.dec.total_cmp(&b.dec));
        Self {
            label,
            rows,
            depth,
            ranking: RankingKey::MAGNITUDE,
        }
    }

    pub(crate) fn with_ranking(self, ranking: RankingKey) -> Self {
        Self { ranking, ..self }
    }

    pub(crate) fn uniform(
        label: &'static str,
        centre: (f64, f64),
        radius_deg: f64,
        stars_per_deg2: f64,
        mags: (f64, f64),
        seed: u64,
    ) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let count = (stars_per_deg2 * cap_area_deg2(radius_deg)).round() as usize;
        let rows = (0..count)
            .map(|index| {
                let (ra, dec) = random_in_cap(&mut rng, centre, radius_deg);
                catalog_row(index, ra, dec, Some(power_law_mag(&mut rng, mags)))
            })
            .collect();
        Self::new(label, rows, mags.1)
    }

    pub(crate) fn rows(&self) -> &[CatalogRow] {
        &self.rows
    }
}

impl CatalogProvider for SyntheticProvider {
    fn cone(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        mag_limit: Option<f64>,
    ) -> Result<(Vec<CatalogRow>, &'static str), String> {
        if !(ra_deg.is_finite() && dec_deg.is_finite() && radius_deg.is_finite()) {
            return Err(format!(
                "cone ({ra_deg}, {dec_deg}) radius {radius_deg} is not finite"
            ));
        }
        let low = self
            .rows
            .partition_point(|row| row.dec < dec_deg - radius_deg);
        let high = self
            .rows
            .partition_point(|row| row.dec <= dec_deg + radius_deg);
        let mut rows: Vec<CatalogRow> = self.rows[low..high]
            .iter()
            .filter(|row| angular_separation(ra_deg, dec_deg, row.ra, row.dec) <= radius_deg)
            .filter(|row| match (mag_limit, row.g) {
                (Some(limit), Some(mag)) => mag <= limit,
                (Some(_), None) => false,
                (None, _) => true,
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            a.g.unwrap_or(f64::INFINITY)
                .total_cmp(&b.g.unwrap_or(f64::INFINITY))
        });
        Ok((rows, self.label))
    }

    fn depth_mag(&self) -> f64 {
        self.depth
    }

    fn ranking_key(&self) -> RankingKey {
        self.ranking
    }
}

pub(crate) fn at_most(value: f64, limit: f64) -> bool {
    value <= limit
}

pub(crate) fn catalog_row(index: usize, ra: f64, dec: f64, mag: Option<f64>) -> CatalogRow {
    CatalogRow {
        id: format!("S{index}"),
        ra,
        dec,
        ra_epoch: ra,
        dec_epoch: dec,
        pm_ra_masyr: None,
        pm_dec_masyr: None,
        g: mag,
        bp: None,
        rp: None,
        bp_rp: None,
        parallax_mas: None,
        teff_k: None,
    }
}

pub(crate) struct ColouredField {
    pub(crate) infrared: Vec<CatalogRow>,
    pub(crate) optical: Vec<CatalogRow>,
}

pub(crate) struct ColourLocus {
    pub(crate) intrinsic_colour: (f64, f64),
    pub(crate) intrinsic_slope: f64,
    pub(crate) intrinsic_offset: f64,
    pub(crate) reddening: (f64, f64),
    pub(crate) reddening_slope: f64,
    pub(crate) scatter: f64,
}

pub(crate) fn coloured_rows(optical: &[CatalogRow], locus: &ColourLocus, seed: u64) -> ColouredField {
    let mut rng = StdRng::seed_from_u64(seed);
    let uniform =
        |rng: &mut StdRng, (low, high): (f64, f64)| low + (high - low) * rng.gen::<f64>();
    let infrared = optical
        .iter()
        .map(|row| {
            let intrinsic = uniform(&mut rng, locus.intrinsic_colour);
            let reddening = uniform(&mut rng, locus.reddening);
            let optical_minus_infrared = locus.intrinsic_offset
                + locus.intrinsic_slope * intrinsic
                + locus.reddening_slope * reddening
                + locus.scatter * gaussian(&mut rng);
            CatalogRow {
                g: row.g.map(|r| r - optical_minus_infrared),
                bp_rp: Some(intrinsic + reddening),
                ..row.clone()
            }
        })
        .collect();
    ColouredField {
        infrared,
        optical: optical.to_vec(),
    }
}

pub(crate) fn cap_area_deg2(radius_deg: f64) -> f64 {
    2.0 * std::f64::consts::PI * (1.0 - radius_deg.to_radians().cos()) * SQ_DEG_PER_SR
}

fn power_law_mag(rng: &mut StdRng, (bright, faint): (f64, f64)) -> f64 {
    let floor = 10f64.powf(-MAG_SLOPE * (faint - bright));
    let u = floor + (1.0 - floor) * rng.gen::<f64>();
    faint + u.log10() / MAG_SLOPE
}

pub(crate) fn random_in_cap(rng: &mut StdRng, centre: (f64, f64), radius_deg: f64) -> (f64, f64) {
    let cos_r = radius_deg.to_radians().cos();
    let z = cos_r + (1.0 - cos_r) * rng.gen::<f64>();
    let distance = z.clamp(-1.0, 1.0).acos().to_degrees();
    destination(centre, 360.0 * rng.gen::<f64>(), distance)
}

pub(crate) fn gaussian(rng: &mut StdRng) -> f64 {
    let u1 = 1.0 - rng.gen::<f64>();
    let u2 = rng.gen::<f64>();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

pub(crate) fn tan_cards(tan: &TanParams, width: usize, height: usize) -> Vec<(String, String)> {
    [
        ("NAXIS", "2".to_string()),
        ("NAXIS1", width.to_string()),
        ("NAXIS2", height.to_string()),
        ("WCSAXES", "2".to_string()),
        ("CTYPE1", "RA---TAN".to_string()),
        ("CTYPE2", "DEC--TAN".to_string()),
        ("CUNIT1", "deg".to_string()),
        ("CUNIT2", "deg".to_string()),
        ("CRPIX1", tan.crpix.0.to_string()),
        ("CRPIX2", tan.crpix.1.to_string()),
        ("CRVAL1", tan.crval.0.to_string()),
        ("CRVAL2", tan.crval.1.to_string()),
        ("CD1_1", tan.cd[0][0].to_string()),
        ("CD1_2", tan.cd[0][1].to_string()),
        ("CD2_1", tan.cd[1][0].to_string()),
        ("CD2_2", tan.cd[1][1].to_string()),
        ("LONPOLE", "180".to_string()),
        ("RADESYS", "ICRS".to_string()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}

pub(crate) fn wcs_from_cards(
    cards: &[(String, String)],
    width: usize,
    height: usize,
) -> WcsTransform {
    let mut header = HduHeader::empty();
    header.set("NAXIS", "2".to_string());
    header.set("NAXIS1", width.to_string());
    header.set("NAXIS2", height.to_string());
    for (key, value) in cards {
        header.set(key, value.clone());
    }
    WcsTransform::from_header(&header)
        .unwrap_or_else(|error| panic!("cards do not form a WCS: {error:#}\n{cards:?}"))
}

pub(crate) fn image_centre_crpix(width: usize, height: usize) -> (f64, f64) {
    ((width as f64 + 1.0) / 2.0, (height as f64 + 1.0) / 2.0)
}

pub(crate) struct TruthField {
    pub(crate) tan: TanParams,
    pub(crate) width: usize,
    pub(crate) height: usize,
    wcs: WcsTransform,
}

impl TruthField {
    pub(crate) fn new(
        centre: (f64, f64),
        scale_arcsec: f64,
        rotation_deg: f64,
        flipped: bool,
        width: usize,
        height: usize,
    ) -> Self {
        let s = scale_arcsec / ARCSEC_PER_DEG;
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let cd = if flipped {
            [[s * cos, s * sin], [-s * sin, s * cos]]
        } else {
            [[-s * cos, s * sin], [s * sin, s * cos]]
        };
        Self::from_tan(
            TanParams {
                crval: centre,
                crpix: image_centre_crpix(width, height),
                cd,
            },
            width,
            height,
        )
    }

    pub(crate) fn with_long_side(
        centre: (f64, f64),
        long_side_deg: f64,
        rotation_deg: f64,
        flipped: bool,
        width: usize,
        height: usize,
    ) -> Self {
        let scale = long_side_deg * ARCSEC_PER_DEG / width.max(height) as f64;
        Self::new(centre, scale, rotation_deg, flipped, width, height)
    }

    pub(crate) fn from_tan(tan: TanParams, width: usize, height: usize) -> Self {
        let wcs = wcs_from_cards(&tan_cards(&tan, width, height), width, height);
        Self {
            tan,
            width,
            height,
            wcs,
        }
    }

    pub(crate) fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub(crate) fn wcs(&self) -> &WcsTransform {
        &self.wcs
    }

    pub(crate) fn centre(&self) -> (f64, f64) {
        self.tan.crval
    }

    pub(crate) fn scale_arcsec(&self) -> f64 {
        let [[a, b], [c, d]] = self.tan.cd;
        (a * d - b * c).abs().sqrt() * ARCSEC_PER_DEG
    }

    pub(crate) fn field(&self) -> FieldSize {
        let scale_deg = self.scale_arcsec() / ARCSEC_PER_DEG;
        FieldSize {
            width_deg: self.width as f64 * scale_deg,
            height_deg: self.height as f64 * scale_deg,
        }
    }

    pub(crate) fn long_side_deg(&self) -> f64 {
        let field = self.field();
        field.width_deg.max(field.height_deg)
    }

    pub(crate) fn to_sky(&self, x: f64, y: f64) -> (f64, f64) {
        let sky = self.wcs.pixel_to_world(x, y);
        (sky.ra, sky.dec)
    }

    pub(crate) fn to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
        self.wcs.world_to_pixel(ra, dec)
    }

    pub(crate) fn inside(&self, x: f64, y: f64) -> bool {
        x >= -0.5 && y >= -0.5 && x <= self.width as f64 - 0.5 && y <= self.height as f64 - 0.5
    }

    pub(crate) fn density_for(&self, stars_in_frame: f64) -> f64 {
        let field = self.field();
        stars_in_frame / (field.width_deg * field.height_deg)
    }

    pub(crate) fn hints(&self, offset_fraction: f64, bearing_deg: f64) -> SolveHints {
        let scale = self.scale_arcsec();
        let (ra, dec) = destination(
            self.centre(),
            bearing_deg,
            offset_fraction * self.long_side_deg(),
        );
        SolveHints {
            centre: Some(Pointing {
                ra,
                dec,
                source: "synthetic".to_string(),
            }),
            scale: Some(ScaleRange {
                low_arcsec: scale * HINT_SCALE_LOW,
                high_arcsec: scale * HINT_SCALE_HIGH,
                source: "synthetic".to_string(),
            }),
            fov_deg: Some(self.long_side_deg()),
            ..SolveHints::default()
        }
    }

    pub(crate) fn sky_error_px(&self, other: &WcsTransform, x: f64, y: f64) -> f64 {
        let truth = self.wcs.pixel_to_world(x, y);
        let got = other.pixel_to_world(x, y);
        angular_separation(truth.ra, truth.dec, got.ra, got.dec) * ARCSEC_PER_DEG
            / self.scale_arcsec()
    }

    pub(crate) fn worst_corner_px(&self, cards: &[(String, String)]) -> f64 {
        let solved = wcs_from_cards(cards, self.width, self.height);
        pixel_edge_corners(self.width, self.height)
            .iter()
            .map(|&(x, y)| self.sky_error_px(&solved, x, y))
            .fold(0.0, |worst, error| {
                if error.is_nan() || error > worst {
                    error
                } else {
                    worst
                }
            })
    }
}

pub(crate) fn detection(x: f64, y: f64, flux: f64, fwhm: f64) -> Detection {
    Detection {
        star: DetectedStar {
            x,
            y,
            flux,
            fwhm,
            eccentricity: DETECTION_ECCENTRICITY,
            peak: flux / (fwhm * fwhm),
            npix: 9,
            snr: 50.0,
        },
        saturated: false,
    }
}

pub(crate) fn flux_for(mag: f64, depth_mag: f64, flux_at_depth: f64) -> f64 {
    flux_at_depth * 10f64.powf(-0.4 * (mag - depth_mag))
}

pub(crate) fn by_flux(mut detections: Vec<Detection>) -> Vec<Detection> {
    detections.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    detections
}

pub(crate) fn detections_from(
    truth: &TruthField,
    rows: &[CatalogRow],
    depth_mag: f64,
    jitter_px: f64,
    spurious: usize,
    seed: u64,
) -> Vec<Detection> {
    let mut rng = StdRng::seed_from_u64(seed);
    let flux_at_depth = 1000.0;
    let mut out: Vec<Detection> = rows
        .iter()
        .filter_map(|row| {
            let mag = row.g.filter(|mag| *mag <= depth_mag)?;
            let (x, y) = truth.to_pixel(row.ra, row.dec);
            truth.inside(x, y).then_some((x, y, mag))
        })
        .map(|(x, y, mag)| {
            detection(
                x + jitter_px * gaussian(&mut rng),
                y + jitter_px * gaussian(&mut rng),
                flux_for(mag, depth_mag, flux_at_depth),
                DETECTION_FWHM_PX,
            )
        })
        .collect();
    for _ in 0..spurious {
        let mag = depth_mag - 5.0 * rng.gen::<f64>();
        out.push(detection(
            rng.gen::<f64>() * (truth.width as f64 - 1.0),
            rng.gen::<f64>() * (truth.height as f64 - 1.0),
            flux_for(mag, depth_mag, flux_at_depth),
            DETECTION_FWHM_PX,
        ));
    }
    by_flux(out)
}

pub(crate) fn random_detections(
    count: usize,
    width: usize,
    height: usize,
    seed: u64,
) -> Vec<Detection> {
    let mut rng = StdRng::seed_from_u64(seed);
    by_flux(
        (0..count)
            .map(|_| {
                detection(
                    rng.gen::<f64>() * (width as f64 - 1.0),
                    rng.gen::<f64>() * (height as f64 - 1.0),
                    flux_for(12.0 - 5.0 * rng.gen::<f64>(), 12.0, 1000.0),
                    DETECTION_FWHM_PX,
                )
            })
            .collect(),
    )
}

pub(crate) fn exact_pairs(truth: &TruthField, count: usize, seed: u64) -> Vec<SkyPair> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|_| {
            let x = rng.gen::<f64>() * (truth.width as f64 - 1.0);
            let y = rng.gen::<f64>() * (truth.height as f64 - 1.0);
            let (ra, dec) = truth.to_sky(x, y);
            SkyPair { x, y, ra, dec }
        })
        .collect()
}

pub(crate) type PixelDistortion = Box<dyn Fn(f64, f64) -> (f64, f64) + Send + Sync>;

pub(crate) struct FrameStyle {
    pub(crate) fwhm_px: f64,
    pub(crate) background: f64,
    pub(crate) noise_sigma: f64,
    pub(crate) flux_at_depth: f64,
    pub(crate) uncatalogued: usize,
    pub(crate) seed: u64,
    pub(crate) distortion: Option<PixelDistortion>,
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self {
            fwhm_px: 3.0,
            background: 100.0,
            noise_sigma: 5.0,
            flux_at_depth: 900.0,
            uncatalogued: 400,
            seed: 1,
            distortion: None,
        }
    }
}

pub(crate) fn render_field(
    truth: &TruthField,
    rows: &[CatalogRow],
    depth_mag: f64,
    style: &FrameStyle,
) -> Array2<f32> {
    let margin = 3.0 * style.fwhm_px;
    let mut rng = StdRng::seed_from_u64(style.seed);
    let mut stars: Vec<Star> = rows
        .iter()
        .filter_map(|row| {
            let mag = row.g?;
            let (x, y) = truth.to_pixel(row.ra, row.dec);
            let (x, y) = style
                .distortion
                .as_ref()
                .map_or((x, y), |distort| distort(x, y));
            let near = x > -margin
                && y > -margin
                && x < truth.width as f64 + margin
                && y < truth.height as f64 + margin;
            near.then(|| star(x, y, flux_for(mag, depth_mag, style.flux_at_depth)))
        })
        .collect();
    for _ in 0..style.uncatalogued {
        let mag = depth_mag + 1.5 * rng.gen::<f64>();
        stars.push(star(
            rng.gen::<f64>() * truth.width as f64,
            rng.gen::<f64>() * truth.height as f64,
            flux_for(mag, depth_mag, style.flux_at_depth),
        ));
    }
    let mut image = render_stars(
        &stars,
        &GaussianPsf::from_fwhm(style.fwhm_px),
        truth.width as u32,
        truth.height as u32,
    );
    add_noise(&mut image, style.background, style.noise_sigma, &mut rng);
    image
}

fn star(x: f64, y: f64, flux: f64) -> Star {
    Star {
        x,
        y,
        z: 0.0,
        flux,
        temperature: 5800.0,
    }
}

pub(crate) fn add_noise(image: &mut Array2<f32>, background: f64, sigma: f64, rng: &mut StdRng) {
    for value in image.iter_mut() {
        *value = (f64::from(*value) + background + sigma * gaussian(rng)) as f32;
    }
}

pub(crate) fn noise_frame(width: usize, height: usize, seed: u64) -> Array2<f32> {
    let mut image = Array2::<f32>::zeros((height, width));
    add_noise(&mut image, 100.0, 5.0, &mut StdRng::seed_from_u64(seed));
    image
}

pub(crate) fn flat_frame(width: usize, height: usize) -> Array2<f32> {
    Array2::<f32>::from_elem((height, width), 1000.0)
}

pub(crate) fn gradient_frame(width: usize, height: usize, seed: u64) -> Array2<f32> {
    let mut image = Array2::<f32>::from_shape_fn((height, width), |(row, col)| {
        (1000.0 + 0.5 * col as f64 + 0.3 * row as f64) as f32
    });
    add_noise(&mut image, 0.0, 5.0, &mut StdRng::seed_from_u64(seed));
    image
}

pub(crate) fn hot_pixel_frame(width: usize, height: usize, seed: u64) -> Array2<f32> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut image = noise_frame(width, height, seed ^ 0x5eed);
    for _ in 0..300 {
        let (row, col) = (rng.gen_range(1..height - 1), rng.gen_range(1..width - 1));
        image[[row, col]] += rng.gen_range(500.0..20_000.0);
    }
    for _ in 0..60 {
        let (row, col) = (rng.gen_range(1..height - 2), rng.gen_range(1..width - 2));
        let level = rng.gen_range(300.0..5_000.0);
        for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            image[[row + dr, col + dc]] += level;
        }
    }
    image
}
