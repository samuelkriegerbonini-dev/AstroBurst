use anyhow::{anyhow, bail, Result};
use ndarray::{s, Array2, ArrayView2, Zip};

use super::{CfaPattern, BLUE, GREEN, RED};
use crate::core::imaging::debayer::BayerPattern;
use crate::math::median::exact_median_mut;
use crate::types::header::HduHeader;

const CELL_POSITIONS: [(usize, usize); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];
const SPLIT_TAGS: [&str; 4] = ["CFA0", "CFA1", "CFA2", "CFA3"];
const SPLIT_PROVENANCE: [&str; 4] = ["cfa0", "cfa1", "cfa2", "cfa3"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaExtract {
    Green,
    Ha,
    HaOiii,
    Split,
}

impl CfaExtract {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "green" => Some(Self::Green),
            "ha" => Some(Self::Ha),
            "ha_oiii" => Some(Self::HaOiii),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Ha => "ha",
            Self::HaOiii => "ha_oiii",
            Self::Split => "split",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CfaPlane {
    pub tag: &'static str,
    pub filter: Option<&'static str>,
    pub provenance: &'static str,
    pub data: Array2<f32>,
}

#[derive(Debug, Clone)]
pub struct CfaExtracted {
    pub planes: Vec<CfaPlane>,
    pub oiii_scale: Option<(f64, f64)>,
    pub trimmed: bool,
}

pub fn extraction_pattern(
    requested: Option<&str>,
    header: Option<&HduHeader>,
) -> Result<CfaPattern> {
    if let Some(text) = requested.map(str::trim).filter(|text| !text.is_empty()) {
        return CfaPattern::parse(text)
            .ok_or_else(|| anyhow!("Unknown CFA pattern '{text}'. Use RGGB, BGGR, GRBG or GBRG."));
    }
    header.and_then(CfaPattern::detect).ok_or_else(|| {
        anyhow!(
            "No CFA pattern in the FITS header (BAYERPAT or COLORTYP): CFA extraction needs a \
             one-shot colour mosaic. Select the Bayer pattern manually if the header lacks it."
        )
    })
}

fn bayer_of(pattern: &CfaPattern) -> Result<BayerPattern> {
    match pattern {
        CfaPattern::Bayer(bayer) => Ok(*bayer),
        CfaPattern::XTrans(_) => bail!(
            "CFA extraction works on 2x2 Bayer mosaics; this is an X-Trans mosaic (36-character \
             BAYERPAT), and X-Trans has no extraction mode."
        ),
    }
}

fn cell_plane(img: &Array2<f32>, (dy, dx): (usize, usize)) -> Array2<f32> {
    let (rows, cols) = (img.nrows() / 2, img.ncols() / 2);
    if rows == 0 || cols == 0 {
        return Array2::zeros((rows, cols));
    }
    img.slice(s![dy..dy + 2 * rows - 1;2, dx..dx + 2 * cols - 1;2])
        .to_owned()
}

fn cell_planes(img: &Array2<f32>) -> [Array2<f32>; 4] {
    CELL_POSITIONS.map(|position| cell_plane(img, position))
}

fn position_of(pattern: &BayerPattern, colour: u8, nth: usize) -> (usize, usize) {
    CELL_POSITIONS
        .into_iter()
        .filter(|&(y, x)| pattern.colour_at(y, x) == colour)
        .nth(nth)
        .unwrap_or((0, 0))
}

pub(crate) fn site_planes(img: &Array2<f32>, pattern: &BayerPattern) -> [Array2<f32>; 4] {
    [
        cell_plane(img, position_of(pattern, RED, 0)),
        cell_plane(img, position_of(pattern, GREEN, 0)),
        cell_plane(img, position_of(pattern, GREEN, 1)),
        cell_plane(img, position_of(pattern, BLUE, 0)),
    ]
}

fn mean_of_greens(first: &Array2<f32>, second: &Array2<f32>) -> Array2<f32> {
    let mut green = Array2::<f32>::zeros(first.dim());
    Zip::from(&mut green)
        .and(first)
        .and(second)
        .par_for_each(|out, &a, &b| {
            *out = if a.is_finite() && b.is_finite() {
                ((a as f64 + b as f64) * 0.5) as f32
            } else {
                f32::NAN
            };
        });
    green
}

fn median_and_mad(samples: &mut [f32]) -> Option<(f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let median = exact_median_mut(samples);
    for value in samples.iter_mut() {
        *value = (*value as f64 - median).abs() as f32;
    }
    Some((median, exact_median_mut(samples)))
}

fn finite_samples(planes: &[ArrayView2<'_, f32>]) -> Vec<f32> {
    planes
        .iter()
        .flat_map(|plane| plane.iter().copied())
        .filter(|v| v.is_finite())
        .collect()
}

fn blue_to_green_scale(
    first_green: &Array2<f32>,
    second_green: &Array2<f32>,
    blue: &Array2<f32>,
) -> Result<(f64, f64)> {
    let no_samples = || anyhow!("OIII needs finite green and blue samples; this mosaic has none");
    let mut greens = finite_samples(&[first_green.view(), second_green.view()]);
    let (green_median, green_mad) = median_and_mad(&mut greens).ok_or_else(no_samples)?;
    let mut blues = finite_samples(&[blue.view()]);
    let (blue_median, blue_mad) = median_and_mad(&mut blues).ok_or_else(no_samples)?;
    let scale = if blue_mad > 0.0 {
        green_mad / blue_mad
    } else {
        1.0
    };
    Ok((scale, green_median - scale * blue_median))
}

fn oiii_plane(
    first_green: &Array2<f32>,
    second_green: &Array2<f32>,
    blue: &Array2<f32>,
    (scale, offset): (f64, f64),
) -> Array2<f32> {
    let mut oiii = Array2::<f32>::zeros(blue.dim());
    Zip::from(&mut oiii)
        .and(first_green)
        .and(second_green)
        .and(blue)
        .par_for_each(|out, &g1, &g2, &b| {
            *out = if g1.is_finite() && g2.is_finite() && b.is_finite() {
                ((g1 as f64 + g2 as f64 + scale * b as f64 + offset) / 3.0) as f32
            } else {
                f32::NAN
            };
        });
    oiii
}

fn plane(
    tag: &'static str,
    filter: Option<&'static str>,
    provenance: &'static str,
    data: Array2<f32>,
) -> CfaPlane {
    CfaPlane {
        tag,
        filter,
        provenance,
        data,
    }
}

fn split_planes(img: &Array2<f32>) -> Vec<CfaPlane> {
    cell_planes(img)
        .into_iter()
        .zip(SPLIT_TAGS.into_iter().zip(SPLIT_PROVENANCE))
        .map(|(data, (tag, provenance))| plane(tag, None, provenance, data))
        .collect()
}

pub fn extract_cfa(
    img: &Array2<f32>,
    pattern: &CfaPattern,
    mode: CfaExtract,
) -> Result<CfaExtracted> {
    let bayer = bayer_of(pattern)?;
    let (rows, cols) = img.dim();
    if rows < 2 || cols < 2 {
        bail!("CFA extraction needs at least one 2x2 cell, the frame is {cols}x{rows}");
    }
    let trimmed = rows % 2 == 1 || cols % 2 == 1;
    let (planes, oiii_scale) = match mode {
        CfaExtract::Split => (split_planes(img), None),
        CfaExtract::Green => {
            let [_, first_green, second_green, _] = site_planes(img, &bayer);
            let green = mean_of_greens(&first_green, &second_green);
            (vec![plane("G", Some("G"), "green", green)], None)
        }
        CfaExtract::Ha => {
            let [red, ..] = site_planes(img, &bayer);
            (vec![plane("Ha", Some("Ha"), "ha", red)], None)
        }
        CfaExtract::HaOiii => {
            let [red, first_green, second_green, blue] = site_planes(img, &bayer);
            let scale = blue_to_green_scale(&first_green, &second_green, &blue)?;
            let oiii = oiii_plane(&first_green, &second_green, &blue, scale);
            let planes = vec![
                plane("Ha", Some("Ha"), "ha", red),
                plane("OIII", Some("OIII"), "oiii", oiii),
            ];
            (planes, Some(scale))
        }
    };
    Ok(CfaExtracted {
        planes,
        oiii_scale,
        trimmed,
    })
}

pub fn extract_green(img: &Array2<f32>, pattern: &CfaPattern) -> Result<CfaExtracted> {
    extract_cfa(img, pattern, CfaExtract::Green)
}

pub fn extract_ha(img: &Array2<f32>, pattern: &CfaPattern) -> Result<CfaExtracted> {
    extract_cfa(img, pattern, CfaExtract::Ha)
}

pub fn extract_ha_oiii(img: &Array2<f32>, pattern: &CfaPattern) -> Result<CfaExtracted> {
    extract_cfa(img, pattern, CfaExtract::HaOiii)
}

pub fn split_cfa(img: &Array2<f32>, pattern: &CfaPattern) -> Result<CfaExtracted> {
    extract_cfa(img, pattern, CfaExtract::Split)
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::super::test_fixtures::XTRANS_LAYOUT;
    use super::*;

    const BAYERS: [BayerPattern; 4] = [
        BayerPattern::Rggb,
        BayerPattern::Bggr,
        BayerPattern::Grbg,
        BayerPattern::Gbrg,
    ];
    const RED_LEVEL: f32 = 8.0;
    const FIRST_GREEN_LEVEL: f32 = 5.0;
    const SECOND_GREEN_LEVEL: f32 = 6.0;
    const BLUE_LEVEL: f32 = 2.0;

    fn gradient(y: usize, x: usize) -> f32 {
        (3 * (y / 2) + x / 2) as f32
    }

    fn site_level(pattern: BayerPattern, y: usize, x: usize) -> f32 {
        match pattern.colour_at(y, x) {
            0 => RED_LEVEL,
            2 => BLUE_LEVEL,
            _ if first_green_position(pattern) == (y % 2, x % 2) => FIRST_GREEN_LEVEL,
            _ => SECOND_GREEN_LEVEL,
        }
    }

    fn first_green_position(pattern: BayerPattern) -> (usize, usize) {
        [(0, 0), (0, 1), (1, 0), (1, 1)]
            .into_iter()
            .find(|&(y, x)| pattern.colour_at(y, x) == 1)
            .unwrap_or((0, 0))
    }

    fn mosaic(pattern: BayerPattern, rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            site_level(pattern, y, x) + gradient(y, x)
        })
    }

    fn cell_gradient(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(cy, cx)| gradient(2 * cy, 2 * cx))
    }

    fn plane<'a>(extracted: &'a CfaExtracted, tag: &str) -> &'a CfaPlane {
        extracted
            .planes
            .iter()
            .find(|plane| plane.tag == tag)
            .unwrap_or_else(|| {
                panic!(
                    "no {tag} plane in {:?}",
                    extracted.planes.iter().map(|p| p.tag).collect::<Vec<_>>()
                )
            })
    }

    fn assert_level(data: &Array2<f32>, level: f32, label: &str) {
        let expected = cell_gradient(data.nrows(), data.ncols()) + level;
        assert_eq!(data, &expected, "{label}");
    }

    #[test]
    fn green_ha_and_split_planes_are_the_exact_sites_of_every_bayer_mosaic() {
        for pattern in BAYERS {
            let img = mosaic(pattern, 12, 12);
            let cfa = CfaPattern::Bayer(pattern);

            let green = extract_cfa(&img, &cfa, CfaExtract::Green).unwrap();
            assert_eq!(green.planes.len(), 1);
            let g = plane(&green, "G");
            assert_eq!((g.filter, g.provenance), (Some("G"), "green"));
            assert_eq!(g.data.dim(), (6, 6));
            assert_level(
                &g.data,
                (FIRST_GREEN_LEVEL + SECOND_GREEN_LEVEL) / 2.0,
                pattern.name(),
            );
            assert!(!green.trimmed && green.oiii_scale.is_none());

            let ha = extract_cfa(&img, &cfa, CfaExtract::Ha).unwrap();
            let h = plane(&ha, "Ha");
            assert_eq!((h.filter, h.provenance), (Some("Ha"), "ha"));
            assert_level(&h.data, RED_LEVEL, pattern.name());

            let split = extract_cfa(&img, &cfa, CfaExtract::Split).unwrap();
            assert_eq!(
                split
                    .planes
                    .iter()
                    .map(|p| (p.tag, p.filter, p.provenance))
                    .collect::<Vec<_>>(),
                vec![
                    ("CFA0", None, "cfa0"),
                    ("CFA1", None, "cfa1"),
                    ("CFA2", None, "cfa2"),
                    ("CFA3", None, "cfa3")
                ]
            );
            for (plane, (dy, dx)) in split.planes.iter().zip([(0, 0), (0, 1), (1, 0), (1, 1)]) {
                assert_level(&plane.data, site_level(pattern, dy, dx), plane.tag);
            }
        }
    }

    #[test]
    fn site_planes_are_ordered_red_first_green_second_green_blue() {
        for pattern in BAYERS {
            let img = mosaic(pattern, 8, 10);
            let [red, first_green, second_green, blue] = site_planes(&img, &pattern);
            assert_level(&red, RED_LEVEL, pattern.name());
            assert_level(&first_green, FIRST_GREEN_LEVEL, pattern.name());
            assert_level(&second_green, SECOND_GREEN_LEVEL, pattern.name());
            assert_level(&blue, BLUE_LEVEL, pattern.name());
        }
    }

    fn affine_blue_mosaic(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let green = 5.0 * (7 * (y / 2) + x / 2) as f32 + 50.0;
            match BayerPattern::Rggb.colour_at(y, x) {
                0 => RED_LEVEL,
                1 => green,
                _ => 0.8 * green + 10.0,
            }
        })
    }

    #[test]
    fn oiii_recovers_an_injected_affine_blue_exactly() {
        let img = affine_blue_mosaic(24, 20);
        let extracted = extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::HaOiii,
        )
        .unwrap();
        let (k, c) = extracted.oiii_scale.expect("no OIII scale reported");
        assert!((k - 1.25).abs() < 1e-9, "k = {k}");
        assert!((c + 12.5).abs() < 1e-9, "c = {c}");
        assert_eq!(
            extracted.planes.iter().map(|p| p.tag).collect::<Vec<_>>(),
            vec!["Ha", "OIII"]
        );
        let oiii = &plane(&extracted, "OIII");
        assert_eq!((oiii.filter, oiii.provenance), (Some("OIII"), "oiii"));
        let [_, green, _, _] = site_planes(&img, &BayerPattern::Rggb);
        for (found, expected) in oiii.data.iter().zip(green.iter()) {
            assert!(
                (found - expected).abs() < 1e-6,
                "OIII {found} vs green {expected}"
            );
        }
        assert!(plane(&extracted, "Ha").data.iter().all(|&v| v == RED_LEVEL));
    }

    fn gaussian(rng: &mut StdRng) -> f64 {
        let u1: f64 = rng.gen_range(1e-12..1.0);
        let u2: f64 = rng.gen();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    fn median_and_mad(data: &Array2<f32>) -> (f64, f64) {
        let mut values: Vec<f32> = data.iter().copied().collect();
        let median = exact_median_mut(&mut values);
        let mut deviations: Vec<f32> = data
            .iter()
            .map(|&v| (v as f64 - median).abs() as f32)
            .collect();
        (median, exact_median_mut(&mut deviations))
    }

    #[test]
    fn oiii_noise_is_the_mean_of_three_independent_sites() {
        let sigma = 10.0;
        let mut rng = StdRng::seed_from_u64(0x5144_4F49_4949);
        let img = Array2::from_shape_fn((512, 512), |(y, x)| {
            let level = match BayerPattern::Rggb.colour_at(y, x) {
                0 => 300.0,
                1 => 1000.0,
                _ => 400.0,
            };
            (level + sigma * gaussian(&mut rng)) as f32
        });
        let extracted = extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::HaOiii,
        )
        .unwrap();
        let oiii = &plane(&extracted, "OIII").data;
        let green = &extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::Green,
        )
        .unwrap()
        .planes[0]
            .data;
        let [_, green_site, _, _] = site_planes(&img, &BayerPattern::Rggb);
        let samples = (oiii.len() as f64).sqrt();
        let (oiii_median, oiii_mad) = median_and_mad(oiii);
        let (green_median, _) = median_and_mad(green);
        let (_, site_mad) = median_and_mad(&green_site);
        let bound = 3.0 * 1.2533 * sigma / samples;
        assert!(
            (oiii_median - green_median).abs() <= bound,
            "{oiii_median} vs {green_median}, bound {bound}"
        );
        let ratio = oiii_mad / site_mad;
        let expected = 1.0 / 3f64.sqrt();
        assert!(
            (ratio - expected).abs() <= 0.05 * expected,
            "MAD ratio {ratio}, expected {expected}"
        );
    }

    #[test]
    fn odd_rows_and_columns_are_trimmed() {
        let img = mosaic(BayerPattern::Rggb, 13, 11);
        for mode in [
            CfaExtract::Green,
            CfaExtract::Ha,
            CfaExtract::HaOiii,
            CfaExtract::Split,
        ] {
            let extracted =
                extract_cfa(&img, &CfaPattern::Bayer(BayerPattern::Rggb), mode).unwrap();
            assert!(extracted.trimmed, "{mode:?}");
            assert!(
                extracted.planes.iter().all(|p| p.data.dim() == (6, 5)),
                "{mode:?}"
            );
        }
        let even = extract_cfa(
            &mosaic(BayerPattern::Rggb, 12, 10),
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::Ha,
        )
        .unwrap();
        assert!(!even.trimmed);
    }

    #[test]
    fn a_non_finite_site_makes_its_cell_nan_without_breaking_the_oiii_scale() {
        let mut img = affine_blue_mosaic(24, 20);
        img[[0, 1]] = f32::NAN;
        img[[3, 3]] = f32::INFINITY;
        let extracted = extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::HaOiii,
        )
        .unwrap();
        let (k, c) = extracted.oiii_scale.unwrap();
        assert!(k.is_finite() && c.is_finite(), "({k}, {c})");
        let oiii = &plane(&extracted, "OIII").data;
        assert!(oiii[[0, 0]].is_nan() && oiii[[1, 1]].is_nan());
        assert!(oiii[[0, 1]].is_finite());
        let green = extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::Green,
        )
        .unwrap();
        assert!(green.planes[0].data[[0, 0]].is_nan());
    }

    #[test]
    fn a_constant_blue_is_shifted_to_the_green_level() {
        let flat =
            Array2::from_shape_fn((8, 8), |(y, x)| match BayerPattern::Rggb.colour_at(y, x) {
                0 => RED_LEVEL,
                1 => FIRST_GREEN_LEVEL,
                _ => BLUE_LEVEL,
            });
        let extracted = extract_cfa(
            &flat,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::HaOiii,
        )
        .unwrap();
        assert_eq!(
            extracted.oiii_scale,
            Some((1.0, (FIRST_GREEN_LEVEL - BLUE_LEVEL) as f64))
        );
        assert!(plane(&extracted, "OIII")
            .data
            .iter()
            .all(|&v| v == FIRST_GREEN_LEVEL));
    }

    #[test]
    fn an_xtrans_mosaic_is_refused_by_name() {
        let xtrans = CfaPattern::parse(XTRANS_LAYOUT).unwrap();
        let img = Array2::from_elem((12, 12), 1.0f32);
        for mode in [
            CfaExtract::Green,
            CfaExtract::Ha,
            CfaExtract::HaOiii,
            CfaExtract::Split,
        ] {
            let message = format!("{:#}", extract_cfa(&img, &xtrans, mode).unwrap_err());
            assert!(message.contains("X-Trans"), "{message}");
        }
    }

    #[test]
    fn a_frame_too_small_for_one_cell_is_refused() {
        let img = Array2::from_elem((1, 8), 1.0f32);
        assert!(extract_cfa(
            &img,
            &CfaPattern::Bayer(BayerPattern::Rggb),
            CfaExtract::Green
        )
        .is_err());
    }

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, (*value).to_string());
        }
        h
    }

    #[test]
    fn the_pattern_comes_from_the_request_then_the_header_and_a_mono_frame_is_refused_naming_bayerpat(
    ) {
        let rggb = header(&[("BAYERPAT", "'RGGB'"), ("NAXIS2", "16")]);
        assert_eq!(
            extraction_pattern(None, Some(&rggb)).unwrap(),
            CfaPattern::Bayer(BayerPattern::Rggb)
        );
        assert_eq!(
            extraction_pattern(Some("gbrg"), Some(&rggb)).unwrap(),
            CfaPattern::Bayer(BayerPattern::Gbrg)
        );
        let offset = header(&[("BAYERPAT", "'RGGB'"), ("XBAYROFF", "1")]);
        assert_eq!(
            extraction_pattern(None, Some(&offset)).unwrap(),
            CfaPattern::Bayer(BayerPattern::Grbg)
        );

        let mono = header(&[("NAXIS2", "16"), ("FILTER", "'L'")]);
        for missing in [
            extraction_pattern(None, Some(&mono)),
            extraction_pattern(None, None),
        ] {
            let message = format!("{:#}", missing.unwrap_err());
            assert!(message.contains("BAYERPAT"), "{message}");
        }
        let unknown = format!(
            "{:#}",
            extraction_pattern(Some("RGBG"), Some(&rggb)).unwrap_err()
        );
        assert!(unknown.contains("RGBG"), "{unknown}");

        let xtrans = header(&[("BAYERPAT", &format!("'{XTRANS_LAYOUT}'"))]);
        assert!(extraction_pattern(None, Some(&xtrans)).unwrap().is_xtrans());
    }

    #[test]
    fn the_named_wrappers_return_the_planes_of_extract_cfa() {
        let img = affine_blue_mosaic(16, 16);
        let cfa = CfaPattern::Bayer(BayerPattern::Rggb);
        let data = |extracted: CfaExtracted| {
            extracted
                .planes
                .into_iter()
                .map(|p| (p.tag, p.data))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            data(extract_green(&img, &cfa).unwrap()),
            data(extract_cfa(&img, &cfa, CfaExtract::Green).unwrap())
        );
        assert_eq!(
            data(extract_ha(&img, &cfa).unwrap()),
            data(extract_cfa(&img, &cfa, CfaExtract::Ha).unwrap())
        );
        assert_eq!(
            data(extract_ha_oiii(&img, &cfa).unwrap()),
            data(extract_cfa(&img, &cfa, CfaExtract::HaOiii).unwrap())
        );
        assert_eq!(
            data(split_cfa(&img, &cfa).unwrap()),
            data(extract_cfa(&img, &cfa, CfaExtract::Split).unwrap())
        );
    }

    #[test]
    fn mode_names_round_trip_and_unknown_modes_are_refused() {
        for mode in [
            CfaExtract::Green,
            CfaExtract::Ha,
            CfaExtract::HaOiii,
            CfaExtract::Split,
        ] {
            assert_eq!(CfaExtract::parse(mode.name()), Some(mode));
        }
        assert_eq!(CfaExtract::parse(" HA_OIII "), Some(CfaExtract::HaOiii));
        assert_eq!(CfaExtract::parse("oiii"), None);
    }
}
