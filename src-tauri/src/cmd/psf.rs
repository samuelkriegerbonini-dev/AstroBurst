use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, invalidate_written, load_cached_full, load_from_cache_or_disk, output_stem};
use crate::cmd::cubeprep::filter_registry;
use crate::cmd::cutout::resolve_output_target;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::cubeprep::psf_io::{fit_gaussian, wcs_scale};
use crate::core::imaging::psf_estimation::{
    build_empirical_psf, empirical_psf_header, estimate_psf, parse_radec_list, propose_psf_stars, stars_from_regions, CentroidMethod,
    EmpiricalPsfOptions, PsfHeaderLabels, PsfResult, PsfStar, StarReport,
};
use crate::core::imaging::push_history;
use crate::core::imaging::region::PhysicalMap;
use crate::core::imaging::region_file::parse_reg_with_physical;
use crate::infra::fits::writer::{write_fits_f64_with_tables, BinTableHdu, TableColumn, TableValues};
use crate::infra::wcs_source::load_wcs;
use crate::types::header::HduHeader;
use crate::types::image_ref::sanitize_fragment;
use crate::types::constants::{
    RES_X, RES_Y, RES_PEAK, RES_FLUX, RES_FWHM, RES_ELLIPTICITY, RES_SNR,
    RES_KERNEL_SIZE, RES_AVERAGE_FWHM, RES_AVERAGE_ELLIPTICITY,
    RES_SPREAD_PIXELS, RES_STARS_USED, RES_STARS_REJECTED, RES_KERNEL,
};

pub(crate) use crate::core::ops::psf::psf_config;

pub(crate) fn psf_result_json(result: &PsfResult) -> serde_json::Value {
    let stars_json: Vec<serde_json::Value> = result.stars_used.iter().map(|s| {
        json!({
            RES_X: s.x,
            RES_Y: s.y,
            RES_PEAK: s.peak,
            RES_FLUX: s.flux,
            RES_FWHM: s.fwhm,
            RES_ELLIPTICITY: s.ellipticity,
            RES_SNR: s.snr,
        })
    }).collect();

    json!({
        RES_KERNEL_SIZE: result.kernel_size,
        RES_AVERAGE_FWHM: result.average_fwhm,
        RES_AVERAGE_ELLIPTICITY: result.average_ellipticity,
        RES_SPREAD_PIXELS: result.spread_pixels,
        RES_STARS_USED: stars_json,
        RES_STARS_REJECTED: result.stars_rejected,
        RES_KERNEL: result.kernel,
    })
}

#[tauri::command]
pub async fn estimate_psf_cmd(
    path: String,
    num_stars: Option<usize>,
    cutout_radius: Option<usize>,
    saturation_threshold: Option<f64>,
    max_ellipticity: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let entry = load_from_cache_or_disk(&path)?;

        let config = psf_config(num_stars, cutout_radius, saturation_threshold, max_ellipticity);

        let result = estimate_psf(entry.arr(), &config)
            .map_err(|e| anyhow::anyhow!(e))?;

        Ok(psf_result_json(&result))
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PsfStarSource {
    Regions { path: String },
    RaDec { text: String },
    Pixels { stars: Vec<PsfStar> },
    Auto {
        #[serde(default)]
        num_stars: Option<usize>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EmpiricalPsfReport {
    pub source: String,
    pub filter: Option<String>,
    pub dataset: Option<String>,
    pub pixel_scale_arcsec: Option<f64>,
    pub normalization_radius_px: f64,
    pub psf_size: usize,
    pub fwhm_px: Option<f64>,
    pub fwhm_arcsec: Option<f64>,
    pub stars_used: usize,
    pub stars_rejected: usize,
    pub stars: Vec<StarReport>,
    pub kernel: Vec<Vec<f64>>,
    pub profile: Vec<f64>,
    pub oversampling: Option<usize>,
    pub oversampled_size: Option<usize>,
    pub output_path: Option<String>,
    pub oversampled_path: Option<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

const STARS_EXTNAME: &str = "STARS";
const STARS_HISTORY: &str = "Extension STARS lists every requested star: 1-based pixel position, measured centre, \
centroid method, border background, aperture flux and the reason a rejected star was left out.";

impl PsfStarSource {
    fn label(&self) -> &'static str {
        match self {
            PsfStarSource::Regions { .. } => "regions",
            PsfStarSource::RaDec { .. } => "ra_dec",
            PsfStarSource::Pixels { .. } => "pixels",
            PsfStarSource::Auto { .. } => "auto",
        }
    }
}

fn source_stars(
    image: &Array2<f32>,
    header: Option<&HduHeader>,
    wcs: Option<&WcsTransform>,
    source: &PsfStarSource,
    warnings: &mut Vec<String>,
) -> Result<Vec<PsfStar>> {
    match source {
        PsfStarSource::Regions { path } => {
            let text = std::fs::read_to_string(path).with_context(|| format!("Failed to read the region file {path}"))?;
            let physical = header.map(PhysicalMap::from_header).unwrap_or_else(PhysicalMap::identity);
            let parsed = parse_reg_with_physical(&text, wcs, &physical)?;
            warnings.extend(parsed.warnings);
            let stars = stars_from_regions(&parsed.regions);
            if stars.is_empty() {
                bail!("the region file {path} holds no included region to use as a star");
            }
            Ok(stars)
        }
        PsfStarSource::RaDec { text } => {
            let wcs = wcs.context("an RA/Dec star list needs a celestial WCS on the image; give pixel positions or regions instead")?;
            Ok(parse_radec_list(text)
                .map_err(|e| anyhow!(e))?
                .into_iter()
                .map(|(name, ra, dec)| {
                    let (x, y) = wcs.world_to_pixel(ra, dec);
                    PsfStar { name, x, y }
                })
                .collect())
        }
        PsfStarSource::Pixels { stars } => Ok(stars.clone()),
        PsfStarSource::Auto { num_stars } => {
            let proposed = propose_psf_stars(image, &psf_config(*num_stars, None, None, None)).map_err(|e| anyhow!(e))?;
            Ok(proposed
                .iter()
                .enumerate()
                .map(|(index, star)| PsfStar { name: format!("auto{}", index + 1), x: star.x, y: star.y })
                .collect())
        }
    }
}

fn resolve_pixel_scale(given: Option<f64>, header: Option<&HduHeader>, wcs: Option<&WcsTransform>) -> Result<Option<f64>> {
    match given {
        Some(scale) if scale.is_finite() && scale > 0.0 => Ok(Some(scale)),
        Some(scale) => bail!("pixel_scale_arcsec must be a positive number, got {scale}"),
        None => Ok(header
            .and_then(wcs_scale)
            .or_else(|| wcs.map(WcsTransform::pixel_scale_arcsec).filter(|scale| scale.is_finite() && *scale > 0.0))),
    }
}

fn psf_labels(
    path: &str,
    header: Option<&HduHeader>,
    dataset: Option<&str>,
    filter: Option<&str>,
    warnings: &mut Vec<String>,
) -> (Option<String>, Option<String>) {
    let given = |value: Option<&str>| value.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    let (dataset, filter) = (given(dataset), given(filter));
    if dataset.is_some() && filter.is_some() {
        return (dataset, filter);
    }
    match filter_registry().identify(header, path, dataset.as_deref()) {
        Ok(found) => {
            warnings.extend(found.warnings);
            (dataset.or(Some(found.entry.dataset)), filter.or(Some(found.entry.filter)))
        }
        Err(error) => {
            warnings.push(format!("the filter was not identified ({error}); give dataset and filter to label the PSF"));
            (dataset, filter)
        }
    }
}

fn empirical_psf_file_name(path: &str, dataset: Option<&str>, filter: Option<&str>, oversampling: Option<usize>) -> String {
    let base = match (dataset, filter) {
        (Some(dataset), Some(filter)) => {
            format!("empirical_psf_{}_{}", sanitize_fragment(&dataset.to_lowercase()), sanitize_fragment(&filter.to_lowercase()))
        }
        _ => format!("{}_empirical_psf", output_stem(path)),
    };
    match oversampling {
        Some(factor) => format!("{base}_os{factor}.fits"),
        None => format!("{base}.fits"),
    }
}

struct StarsTable {
    name: Vec<String>,
    x: Vec<f64>,
    y: Vec<f64>,
    x_centre: Vec<f64>,
    y_centre: Vec<f64>,
    centroid: Vec<String>,
    background: Vec<f64>,
    background_std: Vec<f64>,
    aperture_flux: Vec<f64>,
    cleaning_radius: Vec<f64>,
    status: Vec<String>,
    reason: Vec<String>,
}

impl StarsTable {
    fn new(stars: &[StarReport]) -> Self {
        let number = |field: fn(&StarReport) -> Option<f64>| stars.iter().map(|star| field(star).unwrap_or(f64::NAN)).collect();
        Self {
            name: stars.iter().map(|star| star.name.clone()).collect(),
            x: stars.iter().map(|star| star.x + 1.0).collect(),
            y: stars.iter().map(|star| star.y + 1.0).collect(),
            x_centre: number(|star| star.centre_x.map(|x| x + 1.0)),
            y_centre: number(|star| star.centre_y.map(|y| y + 1.0)),
            centroid: stars
                .iter()
                .map(|star| match star.centroid {
                    Some(CentroidMethod::Quadratic) => "quadratic".to_string(),
                    Some(CentroidMethod::Peak) => "peak".to_string(),
                    None => String::new(),
                })
                .collect(),
            background: number(|star| star.background),
            background_std: number(|star| star.background_std),
            aperture_flux: number(|star| star.aperture_flux),
            cleaning_radius: number(|star| star.cleaning_radius),
            status: stars.iter().map(|star| if star.used { "used" } else { "rejected" }.to_string()).collect(),
            reason: stars.iter().map(|star| star.rejected.clone().unwrap_or_default()).collect(),
        }
    }

    fn columns(&self) -> Vec<TableColumn<'_>> {
        fn text<'a>(name: &'a str, values: &'a [String]) -> TableColumn<'a> {
            TableColumn { name, unit: None, values: TableValues::Text(values) }
        }
        fn number<'a>(name: &'a str, unit: Option<&'a str>, values: &'a [f64]) -> TableColumn<'a> {
            TableColumn { name, unit, values: TableValues::F64(values) }
        }
        vec![
            text("NAME", &self.name),
            number("X", Some("pix"), &self.x),
            number("Y", Some("pix"), &self.y),
            number("XCENTRE", Some("pix"), &self.x_centre),
            number("YCENTRE", Some("pix"), &self.y_centre),
            text("CENTROID", &self.centroid),
            number("BKG", None, &self.background),
            number("BKGSTD", None, &self.background_std),
            number("APFLUX", None, &self.aperture_flux),
            number("CLEANRAD", Some("pix"), &self.cleaning_radius),
            text("STATUS", &self.status),
            text("REASON", &self.reason),
        ]
    }
}

pub(crate) fn build_empirical_psf_file(
    path: &str,
    source: &PsfStarSource,
    options: &EmpiricalPsfOptions,
    dataset: Option<&str>,
    filter: Option<&str>,
    pixel_scale_arcsec: Option<f64>,
    output_dir: Option<&str>,
) -> Result<EmpiricalPsfReport> {
    let started = Instant::now();
    let entry = load_cached_full(path)?;
    let header = entry.header();
    let wcs = header.and_then(|header| load_wcs(path, header).ok());
    let mut warnings = Vec::new();
    let stars = source_stars(entry.arr(), header, wcs.as_ref(), source, &mut warnings)?;
    let pixel_scale_arcsec = resolve_pixel_scale(pixel_scale_arcsec, header, wcs.as_ref())?;
    let built = build_empirical_psf(entry.arr(), &stars, pixel_scale_arcsec, options).map_err(|e| anyhow!(e))?;
    let (dataset, filter) = psf_labels(path, header, dataset, filter, &mut warnings);
    let fwhm_px = fit_gaussian(&built.psf).map(|fit| fit.fwhm_px);
    if fwhm_px.is_none() {
        warnings.push("the Gaussian FWHM fit of the PSF failed".to_string());
    }
    let output_dir = output_dir.map(str::trim).filter(|dir| !dir.is_empty());
    let (output_path, oversampled_path) = match output_dir {
        Some(dir) => {
            let labels = PsfHeaderLabels {
                filter: filter.as_deref(),
                dataset: dataset.as_deref(),
                pixel_scale_arcsec,
                stars_used: built.stars_used(),
            };
            let table = StarsTable::new(&built.stars);
            let columns = table.columns();
            let tables = [BinTableHdu { columns: &columns, extname: STARS_EXTNAME, extver: 1 }];
            let write = |data: &Array2<f64>, oversampling: Option<usize>| -> Result<String> {
                let name = empirical_psf_file_name(path, dataset.as_deref(), filter.as_deref(), oversampling);
                let target = resolve_output_target(path, "", Some(dir), &name)?;
                let mut header = empirical_psf_header(data, &labels, options, oversampling);
                push_history(&mut header, STARS_HISTORY);
                write_fits_f64_with_tables(&target, data, Some(&header), &tables)?;
                invalidate_written(&target);
                Ok(target)
            };
            let written = write(&built.psf, None)?;
            let oversampled = built.oversampled.as_ref().map(|data| write(data, options.oversampling)).transpose()?;
            (Some(written), oversampled)
        }
        None => (None, None),
    };
    Ok(EmpiricalPsfReport {
        source: source.label().to_string(),
        filter,
        dataset,
        pixel_scale_arcsec,
        normalization_radius_px: built.normalization_radius_px,
        psf_size: built.psf.nrows(),
        fwhm_px,
        fwhm_arcsec: fwhm_px.zip(pixel_scale_arcsec).map(|(fwhm, scale)| fwhm * scale),
        stars_used: built.stars_used(),
        stars_rejected: built.stars.len() - built.stars_used(),
        kernel: built.psf.outer_iter().map(|row| row.to_vec()).collect(),
        profile: built.profile,
        oversampling: options.oversampling,
        oversampled_size: built.oversampled.as_ref().map(Array2::nrows),
        stars: built.stars,
        output_path,
        oversampled_path,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn build_empirical_psf_cmd(
    path: String,
    source: PsfStarSource,
    options: Option<EmpiricalPsfOptions>,
    dataset: Option<String>,
    filter: Option<String>,
    pixel_scale_arcsec: Option<f64>,
    output_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = build_empirical_psf_file(
            &path,
            &source,
            &options.unwrap_or_default(),
            dataset.as_deref(),
            filter.as_deref(),
            pixel_scale_arcsec,
            output_dir.as_deref(),
        )?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use ndarray::Array2;

    use super::*;
    use crate::core::imaging::psf_estimation::CentroidMethod;
    use crate::infra::fits::reader::extract_image_mmap_by_index;
    use crate::infra::fits::reader::test_fixtures::write_raw_hdus;
    use crate::infra::fits::table::{find_bintable, load_bintable, ColumnValues};
    use crate::infra::wcs_source::load_wcs;

    const SIZE: usize = 200;
    const SCALE_ARCSEC: f64 = 0.04;
    const STARS: [(f64, f64); 4] = [(50.3, 60.6), (140.3, 60.6), (50.3, 150.6), (140.3, 150.6)];

    fn star_field(dir: &tempfile::TempDir, wcs: bool) -> String {
        let mut image = Array2::<f64>::from_elem((SIZE, SIZE), 3.0);
        for (x, y) in STARS {
            let norm = 4000.0 / (2.0 * PI * 1.8 * 1.8);
            for ((r, c), value) in image.indexed_iter_mut() {
                *value += norm * (-((c as f64 - x).powi(2) + (r as f64 - y).powi(2)) / (2.0 * 1.8 * 1.8)).exp();
            }
        }
        let path = dir.path().join(if wcs { "jw_f444w_field.fits" } else { "plain_field.fits" });
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", SIZE.to_string()),
            ("NAXIS2", SIZE.to_string()),
        ];
        if wcs {
            let s = SCALE_ARCSEC / 3600.0;
            cards.extend([
                ("TELESCOP", "'JWST'".into()),
                ("INSTRUME", "'NIRCAM'".into()),
                ("FILTER", "'F444W'".into()),
                ("CTYPE1", "'RA---TAN'".into()),
                ("CTYPE2", "'DEC--TAN'".into()),
                ("CRPIX1", "100.5".into()),
                ("CRPIX2", "100.5".into()),
                ("CRVAL1", "53.1625".into()),
                ("CRVAL2", "-27.7914".into()),
                ("CD1_1", format!("{:E}", -s)),
                ("CD1_2", "0.0".into()),
                ("CD2_1", "0.0".into()),
                ("CD2_2", format!("{:E}", s)),
            ]);
        }
        let data: Vec<u8> = image.iter().flat_map(|v| (*v as f32).to_be_bytes()).collect();
        write_raw_hdus(&path, &[(cards, data)]);
        path.to_str().unwrap().to_string()
    }

    fn sky_of(path: &str, x: f64, y: f64) -> (f64, f64) {
        let entry = crate::cmd::common::load_cached_full(path).unwrap();
        let wcs = load_wcs(path, entry.header().unwrap()).unwrap();
        let sky = wcs.pixel_to_world(x, y);
        (sky.ra, sky.dec)
    }

    fn centred_near(report: &EmpiricalPsfReport, truth: &[(f64, f64)]) {
        for (star, &(x, y)) in report.stars.iter().zip(truth) {
            let (cx, cy) = (star.centre_x.unwrap(), star.centre_y.unwrap());
            assert!((cx - x).abs() < 0.05 && (cy - y).abs() < 0.05, "{}: centre ({cx}, {cy}) against ({x}, {y})", star.name);
        }
    }

    #[test]
    fn regions_in_physical_image_and_sky_frames_build_and_write_the_psf() {
        let dir = tempfile::tempdir().unwrap();
        let image = star_field(&dir, true);
        let (ra, dec) = sky_of(&image, STARS[3].0, STARS[3].1);
        let regions = dir.path().join("stars.reg");
        std::fs::write(
            &regions,
            format!(
                "# Region file format: DS9 version 4.1\nglobal color=green\nphysical\nbox(51.3,61.6,9,9,0) # text={{first}}\nbox(141.3,61.6,9,9,0)\nimage\npoint(51.3,151.6) # point=x\n-circle(20,20,3)\nfk5\ncircle({ra:.9},{dec:.9},0.5\") # text={{sky}}\n"
            ),
        )
        .unwrap();
        let out = dir.path().join("psfs");
        let options = EmpiricalPsfOptions { oversampling: Some(2), ..EmpiricalPsfOptions::default() };
        let report = build_empirical_psf_file(
            &image,
            &PsfStarSource::Regions { path: regions.to_str().unwrap().to_string() },
            &options,
            None,
            None,
            None,
            Some(out.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(report.source, "regions");
        let names: Vec<&str> = report.stars.iter().map(|star| star.name.as_str()).collect();
        assert_eq!(names, ["first", "region2", "region3", "sky"]);
        assert_eq!((report.stars_used, report.stars_rejected), (4, 0));
        centred_near(&report, &STARS);
        assert!(report.stars.iter().all(|star| star.centroid == Some(CentroidMethod::Quadratic)));
        assert_eq!((report.dataset.as_deref(), report.filter.as_deref()), (Some("JWST"), Some("F444W")));
        assert!((report.pixel_scale_arcsec.unwrap() - SCALE_ARCSEC).abs() < 1e-12);
        assert_eq!((report.psf_size, report.kernel.len(), report.oversampled_size), (51, 51, Some(102)));
        let fwhm = report.fwhm_px.unwrap();
        assert!((fwhm / (1.8 * 2.354_820_045) - 1.0).abs() < 0.02, "FWHM {fwhm} px");
        assert!((report.fwhm_arcsec.unwrap() - fwhm * SCALE_ARCSEC).abs() < 1e-12);

        let written = report.output_path.as_deref().unwrap();
        assert!(written.ends_with("empirical_psf_jwst_f444w.fits"), "{written}");
        let file = std::fs::File::open(written).unwrap();
        let primary = extract_image_mmap_by_index(&file, 0).unwrap();
        assert_eq!(primary.image.dim(), (51, 51));
        let sum: f64 = primary.image.iter().map(|v| *v as f64).sum();
        assert!((sum - 1.0).abs() < 1e-5, "sum {sum}");
        let header = &primary.header;
        assert_eq!(header.get("BUNIT").map(str::trim), Some("normalized"));
        assert_eq!(header.get("FILTER").map(str::trim), Some("F444W"));
        assert_eq!(header.get("DATASET").map(str::trim), Some("JWST"));
        assert_eq!(header.get("METHOD").map(str::trim), Some("median"));
        assert_eq!(header.get_i64("NSTARS"), Some(4));
        assert_eq!(header.get_i64("CUTSIZE"), Some(61));
        assert_eq!(header.get_i64("PSFSIZE"), Some(51));
        assert!((header.get_f64("PIXSCALE").unwrap() - SCALE_ARCSEC).abs() < 1e-12);
        assert_eq!(header.get("CTYPE1").map(str::trim), Some("RA---TAN"));

        let table = load_bintable(written, find_bintable(written, "STARS", 1).unwrap()).unwrap();
        assert_eq!(table.text_column("NAME").unwrap(), ["first", "region2", "region3", "sky"]);
        assert_eq!(table.text_column("STATUS").unwrap(), ["used"; 4]);
        let ColumnValues::F64(xs) = table.flattened_column("X").unwrap().values else { panic!("X is not numeric") };
        assert!((xs[0] - 51.3).abs() < 1e-9 && (xs[1] - 141.3).abs() < 1e-9, "{xs:?}");

        let oversampled = report.oversampled_path.as_deref().unwrap();
        assert!(oversampled.ends_with("empirical_psf_jwst_f444w_os2.fits"), "{oversampled}");
        let file = std::fs::File::open(oversampled).unwrap();
        let os = extract_image_mmap_by_index(&file, 0).unwrap();
        assert_eq!(os.image.dim(), (102, 102));
        assert_eq!(os.header.get_i64("OVERSAMP"), Some(2));
        assert!((os.header.get_f64("PIXSCALE").unwrap() - SCALE_ARCSEC / 2.0).abs() < 1e-12);
    }

    #[test]
    fn radec_lists_pixel_lists_and_automatic_proposals_feed_the_recipe() {
        let dir = tempfile::tempdir().unwrap();
        let image = star_field(&dir, true);
        let list: String = STARS
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| {
                let (ra, dec) = sky_of(&image, x, y);
                format!("{ra:.10} {dec:.10} s{i}\n")
            })
            .collect();
        let options = EmpiricalPsfOptions::default();
        let by_sky = build_empirical_psf_file(&image, &PsfStarSource::RaDec { text: list }, &options, None, None, None, None).unwrap();
        assert_eq!((by_sky.source.as_str(), by_sky.stars_used), ("ra_dec", 4));
        centred_near(&by_sky, &STARS);
        assert!(by_sky.output_path.is_none());

        let pixels: Vec<PsfStar> = STARS.iter().map(|&(x, y)| PsfStar { name: format!("p{x}"), x, y }).collect();
        let by_pixels = build_empirical_psf_file(&image, &PsfStarSource::Pixels { stars: pixels }, &options, None, None, None, None).unwrap();
        assert_eq!(by_pixels.kernel, by_sky.kernel);

        let auto = build_empirical_psf_file(&image, &PsfStarSource::Auto { num_stars: Some(3) }, &options, None, None, None, None).unwrap();
        assert_eq!(auto.source, "auto");
        assert_eq!(auto.stars.len(), 3);
        assert!(auto.stars.iter().all(|star| star.name.starts_with("auto") && star.used), "{:?}", auto.stars);

        let plain = star_field(&dir, false);
        let error = build_empirical_psf_file(&plain, &PsfStarSource::RaDec { text: "53.1 -27.7\n".into() }, &options, None, None, None, None).unwrap_err();
        assert!(error.to_string().contains("WCS"), "{error}");
        let arcsec = EmpiricalPsfOptions { normalization_radius_unit: crate::core::imaging::psf_estimation::RadiusUnit::Arcsec, normalization_radius: 0.8, ..options };
        let scaled = build_empirical_psf_file(&plain, &PsfStarSource::Auto { num_stars: None }, &arcsec, Some("hst"), Some("f814w"), Some(0.04), None).unwrap();
        assert_eq!(scaled.normalization_radius_px, 20.0);
        assert_eq!((scaled.dataset.as_deref(), scaled.filter.as_deref()), (Some("hst"), Some("f814w")));
    }

    #[test]
    fn panel_payloads_deserialize_into_sources_and_a_partial_recipe() {
        let sources: Vec<PsfStarSource> = serde_json::from_str(
            r#"[{"kind":"regions","path":"C:/d/stars_f444w.reg"},{"kind":"ra_dec","text":"53.16 -27.79"},{"kind":"auto","num_stars":30},{"kind":"auto"},{"kind":"pixels","stars":[{"name":"a","x":1.5,"y":2.5}]}]"#,
        )
        .unwrap();
        assert_eq!(
            sources,
            [
                PsfStarSource::Regions { path: "C:/d/stars_f444w.reg".into() },
                PsfStarSource::RaDec { text: "53.16 -27.79".into() },
                PsfStarSource::Auto { num_stars: Some(30) },
                PsfStarSource::Auto { num_stars: None },
                PsfStarSource::Pixels { stars: vec![PsfStar { name: "a".into(), x: 1.5, y: 2.5 }] },
            ]
        );
        let options: EmpiricalPsfOptions = serde_json::from_str(
            r#"{"cutout_size":31,"psf_size":21,"normalization_radius":0.48,"normalization_radius_unit":"arcsec","oversampling":null,"stack":"mean","radial_cleaning":true}"#,
        )
        .unwrap();
        assert_eq!(
            options,
            EmpiricalPsfOptions {
                cutout_size: 31,
                psf_size: 21,
                normalization_radius: 0.48,
                normalization_radius_unit: crate::core::imaging::psf_estimation::RadiusUnit::Arcsec,
                oversampling: None,
                stack: crate::core::imaging::psf_estimation::PsfStack::Mean,
                radial_cleaning: true,
                ..EmpiricalPsfOptions::default()
            }
        );
    }
}
