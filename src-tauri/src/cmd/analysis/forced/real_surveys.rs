use super::real_legacy::{
    assert_within, catalogue, cell, column, data, has_whole_aperture, median, print_band, reported,
};
use super::*;
use crate::cmd::companions::set_companion_json;
use crate::core::imaging::layer_roles::LayerRole;
use crate::infra::tables::TextTable;

const MICROJANSKY_PER_JANSKY: f64 = 1e6;
const CALIBRATION_RELATIVE_TOLERANCE: f64 = 1e-6;
const PIXEL_SCALE_TOLERANCE: f64 = 1e-6;
const ORACLE_FLUX_RELATIVE_TOLERANCE: f64 = 0.002;
const COLOUR_TOLERANCE_MAG: f64 = 0.005;
const WCS_TOLERANCE_PX: f64 = 0.05;
const THREE_ARCSEC: f64 = 3.0;
const STAR_A1: (&str, f64, f64) = ("A1", 53.13873891283809, -28.08538886599608);
const STAR_A2: (&str, f64, f64) = ("A2", 53.1464291497623, -28.10260430600036);

const DES_CATALOGUE: &str = "survey/catalogs/C2_des_dr2_box_A.csv";
const DES_BRIGHTEST_STAR: f64 = 1394595487.0;
const DES_BRIGHTEST_STAR_PIXEL: (f64, f64) = (333.734, 428.664);
const DES_APERTURE_8_RADIUS_PX: f64 = 11.11;
const DES_CUTOUT_SIDE_PX: f64 = 458.0;
const JY_PER_DES_COUNT: f64 = 3.6307805477010066e-9;
const DES_PIXEL_SCALE_ARCSEC: f64 = 0.263;

const SDSS_FRAME: &str = "survey/S1/frame-r-001462-4-0165.fits.bz2";
const SDSS_CATALOGUE: &str = "survey/S1/C4_sdss_dr17_field_1462_4_165.csv";
const SDSS_STAR_TYPE: f64 = 6.0;
const SDSS_APERTURE_7_RADIUS_PX: f64 = 18.58;
const SDSS_SKY_ANNULUS_PX: [f64; 2] = [28.55, 44.21];
const JY_PER_NANOMAGGY: f64 = 3.630780547701009e-6;
const SDSS_PIXEL_SCALE_ARCSEC: f64 = 0.396036;

const PS1_STACK: &str = "survey/P1/ps1_A_r_stack_480px.fits";
const PS1_WEIGHT: &str = "survey/P1/ps1_A_r_wt_480px.fits";
const PS1_CATALOGUE: &str = "survey/P1/C5_ps1_dr2_cone_A.csv";
const PS1_MISSING: f64 = -999.0;
const PS1_FILLED_APERTURE: f64 = 0.97;
const PS1_REFERENCE_STAR: (&str, f64, f64) = ("74290531386797794", 53.13872873150106, -28.085398991615556);
const PS1_REFERENCE_STAR_PIXEL: (f64, f64) = (131.42, 452.29);
const JY_PER_PS1_COUNT: f64 = 6.051300912835008e-10;
const PS1_PIXEL_SCALE_ARCSEC: f64 = 0.25;

const EUCLID_MOSAIC: &str =
    "survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
const EUCLID_RMS_FILE_NAME: &str =
    "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
const EUCLID_GROWTH_RADII_ARCSEC: [f64; 3] = [1.0, 1.25, 1.5];
const JY_PER_EUCLID_UNIT: f64 = 5.248074602497712e-7;
const EUCLID_PIXEL_SCALE_ARCSEC: f64 = 0.1;

const LEGACY_R: &str = "survey/L2/legacysurvey-0532m280-image-r.fits.fz";
const LEGACY_PIXEL_SCALE_ARCSEC: f64 = 0.262;

const BRIGHT_SNR: f64 = 50.0;

struct EuclidPointSource {
    id: &'static str,
    ra_deg: f64,
    dec_deg: f64,
    flux_detection_total_ujy: f64,
    flux_vis_psf_ujy: f64,
}

const EUCLID_POINT_SOURCES: [EuclidPointSource; 2] = [
    EuclidPointSource {
        id: "E1",
        ra_deg: 53.12884377,
        dec_deg: -28.10480644,
        flux_detection_total_ujy: 4.0499644279,
        flux_vis_psf_ujy: 3.4198615551,
    },
    EuclidPointSource {
        id: "E2",
        ra_deg: 53.13334162,
        dec_deg: -28.09770003,
        flux_detection_total_ujy: 1.1232932806,
        flux_vis_psf_ujy: 0.98780959845,
    },
];

fn sky_position((id, ra_deg, dec_deg): (&str, f64, f64)) -> ForcedSkyPosition {
    ForcedSkyPosition { id: id.to_string(), ra_deg, dec_deg }
}

fn two_stars() -> [ForcedSkyPosition; 2] {
    [sky_position(STAR_A1), sky_position(STAR_A2)]
}

fn row_positions(table: &TextTable, ra_column: &str, dec_column: &str, rows: &[usize]) -> Vec<ForcedSkyPosition> {
    let (ra, dec) = (column(table, ra_column), column(table, dec_column));
    rows.iter().map(|&row| sky_position((&format!("row {row}"), ra[row], dec[row]))).collect()
}

fn band_of(path: String) -> ForcedBandInput {
    ForcedBandInput { path, label: None, dataset: None }
}

fn arcsec_aperture(radius_arcsec: f64) -> ForcedAperture {
    ForcedAperture {
        radius_arcsec: Some(radius_arcsec),
        radius_px: None,
        annulus_arcsec: None,
        annulus_px: None,
        correction: ForcedCorrection::None,
        reference_radius_arcsec: None,
    }
}

fn pixel_aperture(radius_px: f64, annulus_px: Option<[f64; 2]>) -> ForcedAperture {
    ForcedAperture {
        radius_arcsec: None,
        radius_px: Some(radius_px),
        annulus_arcsec: None,
        annulus_px,
        correction: ForcedCorrection::None,
        reference_radius_arcsec: None,
    }
}

fn measure(bands: &[ForcedBandInput], positions: &[ForcedSkyPosition], aperture: &ForcedAperture) -> ForcedPhotometryResult {
    forced_photometry(bands, positions, aperture, &ForcedRunOptions::default(), None).unwrap()
}

fn microjansky(result: &ForcedPhotometryResult, row: usize, band: usize) -> f64 {
    cell(result, row, band).flux_jy.unwrap() * MICROJANSKY_PER_JANSKY
}

fn assert_calibration(band: &ForcedBand, jy_per_native_unit: f64, pixel_scale_arcsec: f64) {
    assert_within(
        &format!("jy_per_native_unit of {}", band.file_name),
        band.jy_per_native_unit.unwrap(),
        jy_per_native_unit,
        CALIBRATION_RELATIVE_TOLERANCE * jy_per_native_unit,
    );
    assert_within(
        &format!("pixel_scale_arcsec of {}", band.file_name),
        band.pixel_scale_arcsec.unwrap(),
        pixel_scale_arcsec,
        PIXEL_SCALE_TOLERANCE,
    );
}

fn assert_oracle_flux(name: &str, measured_ujy: f64, oracle_ujy: f64) {
    assert_within(name, measured_ujy, oracle_ujy, ORACLE_FLUX_RELATIVE_TOLERANCE * oracle_ujy);
}

fn copied(dir: &Path, rel: &str) -> String {
    let source = data(rel);
    let target = dir.join(Path::new(&source).file_name().unwrap());
    std::fs::copy(&source, &target).unwrap();
    target.to_str().unwrap().to_string()
}

fn des_layer(band: &str, extension: u8) -> String {
    format!("survey/D1/des-dr2_A_{band}_extn{extension}.fits")
}

fn des_band_with_weight(dir: &Path, band: &str) -> ForcedBandInput {
    let image = copied(dir, &des_layer(band, 1));
    let weight = copied(dir, &des_layer(band, 3));
    set_companion_json(&image, "err", &weight, None).unwrap();
    band_of(image)
}

fn des_r_aperture_8(dir: &Path, table: &TextTable) -> ForcedPhotometryResult {
    assert_eq!(table.n_rows, 148, "rows of C2");
    let band = des_band_with_weight(dir, "r");
    let mask = copied(dir, &des_layer("r", 2));
    set_companion_json(&band.path, "dq", &mask, None).unwrap();
    let every_row: Vec<usize> = (0..table.n_rows).collect();
    let positions = row_positions(table, "alphawin_j2000", "deltawin_j2000", &every_row);
    measure(&[band], &positions, &pixel_aperture(DES_APERTURE_8_RADIUS_PX, None))
}

fn des_aperture_8_edge_margin_px(measured: &ForcedCell) -> f64 {
    [measured.x, measured.y]
        .into_iter()
        .flat_map(|centre| {
            [centre - DES_APERTURE_8_RADIUS_PX + 0.5, DES_CUTOUT_SIDE_PX - 0.5 - centre - DES_APERTURE_8_RADIUS_PX]
        })
        .fold(f64::INFINITY, f64::min)
}

fn des_catalogue_snr(table: &TextTable, row: usize) -> f64 {
    column(table, "flux_aper_8_r")[row] / column(table, "fluxerr_aper_8_r")[row]
}

fn ps1_r_band_with_variance(dir: &Path) -> ForcedBandInput {
    let image = copied(dir, PS1_STACK);
    let weight = copied(dir, PS1_WEIGHT);
    set_companion_json(&image, "err", &weight, Some(LayerRole::Variance)).unwrap();
    band_of(image)
}

fn assert_ps1_reference_star_lands_on_its_pixel(stack: &str) {
    let (id, ra_deg, dec_deg) = PS1_REFERENCE_STAR;
    let (expected_x, expected_y) = PS1_REFERENCE_STAR_PIXEL;
    let wcs = load_wcs(stack, &cached_header(stack).unwrap()).unwrap();
    let (x, y) = wcs.world_to_pixel(ra_deg, dec_deg);
    println!("PS1 star {id} at ({x:.6}, {y:.6}) px");
    assert!(
        (x - expected_x).abs() <= WCS_TOLERANCE_PX && (y - expected_y).abs() <= WCS_TOLERANCE_PX,
        "PS1 star {id} lands at ({x}, {y}), not within {WCS_TOLERANCE_PX} px of ({expected_x}, {expected_y}): the WCS of the cutout was read without its PC001001 and PC002002 cards (package SURVEY-PHOT-R9)"
    );
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1 and survey/catalogs"]
fn real_des_d1_r_aperture_8_matches_flux_aper_8() {
    let table = catalogue(DES_CATALOGUE);
    let dir = tempfile::tempdir().unwrap();
    let result = des_r_aperture_8(dir.path(), &table);
    let band = &result.bands[0];
    print_band(band);
    let brightest = column(&table, "coadd_object_id").iter().position(|&id| id == DES_BRIGHTEST_STAR).unwrap();
    let star = cell(&result, brightest, 0);
    println!("brightest star at ({:.6}, {:.6}) px", star.x, star.y);
    let (expected_x, expected_y) = DES_BRIGHTEST_STAR_PIXEL;
    assert_within("x of the brightest star (mirrored when the CD matrix is not used in full)", star.x, expected_x, WCS_TOLERANCE_PX);
    assert_within("y of the brightest star", star.y, expected_y, WCS_TOLERANCE_PX);
    let (flags, image_flags) = (column(&table, "flags_r"), column(&table, "imaflags_iso_r"));
    let whole: Vec<usize> = (0..table.n_rows).filter(|&row| has_whole_aperture(&result, row, 0)).collect();
    let edge_margins: Vec<f64> =
        (0..table.n_rows).map(|row| des_aperture_8_edge_margin_px(cell(&result, row, 0))).collect();
    let inside: Vec<usize> = (0..table.n_rows).filter(|&row| edge_margins[row] >= 0.0).collect();
    let grazing: Vec<usize> = whole.iter().copied().filter(|&row| edge_margins[row] < 0.0).collect();
    println!(
        "apertures inside the cutout: {}; cells with a masked fraction of 0: {} of {}",
        inside.len(),
        whole.len(),
        table.n_rows
    );
    for &row in &grazing {
        let grazed = cell(&result, row, 0);
        println!(
            "masked fraction 0 although the aperture leaves the cutout by {:.4} px: coadd_object_id {} at ({:.4}, {:.4}) px",
            -edge_margins[row],
            column(&table, "coadd_object_id")[row],
            grazed.x,
            grazed.y
        );
    }
    let rows: Vec<usize> = whole
        .iter()
        .copied()
        .filter(|&row| flags[row] < 4.0 && image_flags[row] == 0.0 && des_catalogue_snr(&table, row) > BRIGHT_SNR)
        .collect();
    assert_eq!(rows.len(), 13, "C2 rows with flags_r < 4, imaflags_iso_r 0, S/N above 50 and a whole aperture");
    let catalogue_flux = column(&table, "flux_aper_8_r");
    let ratios: Vec<f64> =
        rows.iter().map(|&row| cell(&result, row, 0).flux_native.unwrap() / catalogue_flux[row]).collect();
    let (centre, _) = reported("DES r flux_native / flux_aper_8_r", &ratios);
    assert_eq!(inside.len(), 135, "apertures of C2 that lie inside the cutout");
    assert!(
        inside.iter().all(|row| whole.contains(row)),
        "an aperture inside the cutout has a masked fraction above 0 although the mask is empty and the weight has no zero"
    );
    let subsample_step_px = 1.0 / f64::from(FORCED_SUBSAMPLES);
    assert!(
        grazing.iter().all(|&row| -edge_margins[row] < subsample_step_px),
        "a masked fraction of 0 on an aperture that leaves the cutout by a subsample step or more: rows {grazing:?}"
    );
    assert_within("median ratio (research target)", centre, 1.0, 0.03);
    assert_within("median ratio (planner pin)", centre, 0.9964, 0.002);
    assert_calibration(band, JY_PER_DES_COUNT, DES_PIXEL_SCALE_ARCSEC);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1 and survey/catalogs"]
fn real_des_d1_r_formal_error_of_faint_sources_is_within_twenty_percent_of_the_catalogue() {
    let table = catalogue(DES_CATALOGUE);
    let dir = tempfile::tempdir().unwrap();
    let result = des_r_aperture_8(dir.path(), &table);
    let band = &result.bands[0];
    print_band(band);
    let flags = column(&table, "flags_r");
    let rows: Vec<usize> = (0..table.n_rows)
        .filter(|&row| {
            let snr = des_catalogue_snr(&table, row);
            flags[row] == 0.0 && snr > 5.0 && snr < 20.0 && has_whole_aperture(&result, row, 0)
        })
        .collect();
    assert_eq!(rows.len(), 29, "C2 rows with flags_r 0, S/N between 5 and 20 and a whole aperture");
    let catalogue_err = column(&table, "fluxerr_aper_8_r");
    let ratios: Vec<f64> =
        rows.iter().map(|&row| cell(&result, row, 0).flux_err_native.unwrap() / catalogue_err[row]).collect();
    let (centre, _) = reported("DES r flux_err_native / fluxerr_aper_8_r", &ratios);
    assert_eq!(band.err_origin.as_ref().unwrap().role, "inverse_variance");
    assert_within("median error ratio (research target)", centre, 1.0, 0.20);
    assert_within("median error ratio (planner pin)", centre, 0.982, 0.005);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1"]
fn real_des_d1_colours_of_two_stars_follow_the_three_bands() {
    let dir = tempfile::tempdir().unwrap();
    let bands: Vec<ForcedBandInput> = ["g", "r", "i"].iter().map(|band| des_band_with_weight(dir.path(), band)).collect();
    let result = measure(&bands, &two_stars(), &arcsec_aperture(THREE_ARCSEC));
    result.bands.iter().for_each(print_band);
    for band in &result.bands {
        assert_eq!(band.n_measured, 2, "stars measured on {}", band.file_name);
    }
    let oracle = [
        ("A1", [117.30520, 305.56565, 417.92085], [1.03947, 0.33997]),
        ("A2", [28.17811, 105.01678, 299.25691], [1.42837, 1.13696]),
    ];
    for (star, (name, oracle_ujy, oracle_colours)) in oracle.into_iter().enumerate() {
        let fluxes: Vec<f64> = (0..3).map(|band| microjansky(&result, star, band)).collect();
        let magnitudes: Vec<f64> = (0..3).map(|band| cell(&result, star, band).mag_ab.unwrap()).collect();
        let colours = [magnitudes[0] - magnitudes[1], magnitudes[1] - magnitudes[2]];
        println!("{name} g, r, i: {fluxes:?} uJy; g - r {:.5} r - i {:.5} AB mag", colours[0], colours[1]);
        for (band, label) in ["g", "r", "i"].into_iter().enumerate() {
            assert_oracle_flux(&format!("{name} {label} flux (uJy)"), fluxes[band], oracle_ujy[band]);
        }
        assert_within(&format!("{name} g - r"), colours[0], oracle_colours[0], COLOUR_TOLERANCE_MAG);
        assert_within(&format!("{name} r - i"), colours[1], oracle_colours[1], COLOUR_TOLERANCE_MAG);
    }
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1"]
fn real_sdss_s1_r_aperture_7_matches_aperflux7() {
    let table = catalogue(SDSS_CATALOGUE);
    let (kind, clean, psf_mag) = (column(&table, "type"), column(&table, "clean"), column(&table, "psfMag_r"));
    let every_row: Vec<usize> = (0..table.n_rows).collect();
    let positions = row_positions(&table, "ra", "dec", &every_row);
    let frame = [band_of(data(SDSS_FRAME))];
    let with_sky = measure(&frame, &positions, &pixel_aperture(SDSS_APERTURE_7_RADIUS_PX, Some(SDSS_SKY_ANNULUS_PX)));
    let without_sky = measure(&frame, &positions, &pixel_aperture(SDSS_APERTURE_7_RADIUS_PX, None));
    let band = &with_sky.bands[0];
    print_band(band);
    let rows: Vec<usize> = (0..table.n_rows)
        .filter(|&row| {
            kind[row] == SDSS_STAR_TYPE
                && clean[row] == 1.0
                && (15.0..=18.0).contains(&psf_mag[row])
                && cell(&with_sky, row, 0).flux_native.is_some()
        })
        .collect();
    assert_eq!(rows.len(), 31, "clean C4 stars with psfMag_r from 15 to 18 that were measured");
    let catalogue_flux = column(&table, "aperFlux7_r");
    let ratios_of = |result: &ForcedPhotometryResult| -> Vec<f64> {
        rows.iter().map(|&row| cell(result, row, 0).flux_native.unwrap() / catalogue_flux[row]).collect()
    };
    let (centre, _) = reported("SDSS r flux_native / aperFlux7_r, sky annulus", &ratios_of(&with_sky));
    let (centre_without_sky, _) = reported("SDSS r flux_native / aperFlux7_r, no sky annulus", &ratios_of(&without_sky));
    assert_within("median ratio (research target)", centre, 1.0, 0.03);
    assert_within("median ratio (planner pin)", centre, 1.0030, 0.004);
    assert_within("median ratio without the sky annulus (research target)", centre_without_sky, 1.0, 0.03);
    assert_eq!(band.bunit.as_deref(), Some("nanomaggy"));
    assert_calibration(band, JY_PER_NANOMAGGY, SDSS_PIXEL_SCALE_ARCSEC);
    assert!(band.err_origin.is_none(), "the frame has no error layer, got {:?}", band.err_origin);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1"]
fn real_ps1_p1_r_three_arcsec_flux_matches_rflxr5() {
    let table = catalogue(PS1_CATALOGUE);
    let (flux, flux_err, fill) = (column(&table, "rflxr5"), column(&table, "rflxr5err"), column(&table, "rflxr5fill"));
    let listed: Vec<usize> = (0..table.n_rows)
        .filter(|&row| [flux[row], flux_err[row], fill[row]].iter().all(|&value| value != PS1_MISSING))
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let stack = ps1_r_band_with_variance(dir.path());
    assert_ps1_reference_star_lands_on_its_pixel(&stack.path);
    let positions = row_positions(&table, "ramean", "decmean", &listed);
    let result = measure(std::slice::from_ref(&stack), &positions, &arcsec_aperture(THREE_ARCSEC));
    let band = &result.bands[0];
    print_band(band);
    println!("C5 rows: {}; with rflxr5, rflxr5err and rflxr5fill: {}", table.n_rows, listed.len());
    let selected: Vec<usize> = (0..listed.len())
        .filter(|&k| {
            let row = listed[k];
            fill[row] >= PS1_FILLED_APERTURE && flux[row] / flux_err[row] > BRIGHT_SNR && has_whole_aperture(&result, k, 0)
        })
        .collect();
    assert_eq!(selected.len(), 4, "C5 rows with rflxr5fill >= 0.97, S/N above 50 and a whole aperture");
    let ratios: Vec<f64> = selected.iter().map(|&k| cell(&result, k, 0).flux_jy.unwrap() / flux[listed[k]]).collect();
    let err_ratios: Vec<f64> =
        selected.iter().map(|&k| cell(&result, k, 0).flux_err_jy.unwrap() / flux_err[listed[k]]).collect();
    let (centre, _) = reported("PS1 r flux_jy / rflxr5", &ratios);
    reported("PS1 r flux_err_jy / rflxr5err", &err_ratios);
    assert_within("median ratio (research target)", centre, 1.0, 0.05);
    assert_within("median ratio (planner pin)", centre, 1.0167, 0.003);
    assert_calibration(band, JY_PER_PS1_COUNT, PS1_PIXEL_SCALE_ARCSEC);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1"]
fn real_euclid_e1_growth_plateau_of_the_point_like_sources_matches_flux_detection_total() {
    let mosaic = [band_of(data(EUCLID_MOSAIC))];
    let positions: Vec<ForcedSkyPosition> = EUCLID_POINT_SOURCES
        .iter()
        .map(|source| sky_position((source.id, source.ra_deg, source.dec_deg)))
        .collect();
    let growth: Vec<ForcedPhotometryResult> = EUCLID_GROWTH_RADII_ARCSEC
        .iter()
        .map(|&radius_arcsec| measure(&mosaic, &positions, &arcsec_aperture(radius_arcsec)))
        .collect();
    let widest = growth.last().unwrap();
    let band = &widest.bands[0];
    print_band(band);
    println!("flag_names {:?}", band.flag_names);
    for run in &growth {
        assert_eq!(run.bands[0].n_measured, 2, "point-like sources measured at {:?} px", run.bands[0].aperture_radius_px);
    }
    let plateaus: Vec<f64> = (0..EUCLID_POINT_SOURCES.len())
        .map(|source| median(&growth.iter().map(|run| microjansky(run, source, 0)).collect::<Vec<f64>>()))
        .collect();
    println!("plateau of flux_jy at {EUCLID_GROWTH_RADII_ARCSEC:?} arcsec: {plateaus:?} uJy");
    let over_total: Vec<f64> =
        plateaus.iter().zip(&EUCLID_POINT_SOURCES).map(|(plateau, source)| plateau / source.flux_detection_total_ujy).collect();
    let over_psf: Vec<f64> =
        plateaus.iter().zip(&EUCLID_POINT_SOURCES).map(|(plateau, source)| plateau / source.flux_vis_psf_ujy).collect();
    reported("Euclid VIS plateau / flux_detection_total", &over_total);
    reported("Euclid VIS plateau / flux_vis_psf (not asserted: the oracle has 1.19 and 1.14)", &over_psf);
    let excluding = forced_photometry(
        &mosaic,
        &positions,
        &arcsec_aperture(EUCLID_GROWTH_RADII_ARCSEC[2]),
        &ForcedRunOptions { exclude_dq: Some(true), ..ForcedRunOptions::default() },
        None,
    )
    .unwrap();
    for (source, ratio) in EUCLID_POINT_SOURCES.iter().zip(&over_total) {
        assert_within(&format!("{} plateau / flux_detection_total (first-run target)", source.id), *ratio, 1.0, 0.03);
    }
    assert_calibration(band, JY_PER_EUCLID_UNIT, EUCLID_PIXEL_SCALE_ARCSEC);
    let err_source = &band.err_origin.as_ref().unwrap().source;
    assert!(err_source.contains(EUCLID_RMS_FILE_NAME), "error layer {err_source} is not the RMS sibling");
    assert!(band.flag_names.is_empty(), "{:?}", band.flag_names);
    for source in 0..EUCLID_POINT_SOURCES.len() {
        let (plain, excluded) = (cell(widest, source, 0), cell(&excluding, source, 0));
        println!(
            "{} at 1.5 arcsec: masked_fraction {:?} without and {:?} with exclude_dq, flags_or {:?}",
            EUCLID_POINT_SOURCES[source].id, plain.masked_fraction, excluded.masked_fraction, plain.flags_or
        );
        assert_eq!(plain.masked_fraction, Some(0.0));
        assert_eq!(excluded.masked_fraction, Some(0.0));
        assert_eq!(excluded.flux_native.unwrap().to_bits(), plain.flux_native.unwrap().to_bits());
        assert_eq!(excluded.flux_jy.unwrap().to_bits(), plain.flux_jy.unwrap().to_bits());
    }
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2, survey/D1 and survey/P1"]
fn real_cross_survey_r_fluxes_of_two_stars_agree_within_the_filter_differences() {
    let dir = tempfile::tempdir().unwrap();
    let bands =
        [band_of(data(LEGACY_R)), des_band_with_weight(dir.path(), "r"), ps1_r_band_with_variance(dir.path())];
    let (legacy, des, ps1) = (0, 1, 2);
    let result = measure(&bands, &two_stars(), &arcsec_aperture(THREE_ARCSEC));
    result.bands.iter().for_each(print_band);
    let stars = ["A1", "A2"];
    let fluxes_of = |band: usize| -> Vec<f64> {
        (0..stars.len()).filter_map(|star| cell(&result, star, band).flux_jy).map(|flux| flux * MICROJANSKY_PER_JANSKY).collect()
    };
    let (legacy_ujy, des_ujy, ps1_ujy) = (fluxes_of(legacy), fluxes_of(des), fluxes_of(ps1));
    println!("Legacy r {legacy_ujy:?} uJy; DES r {des_ujy:?} uJy; PS1 r {ps1_ujy:?} uJy");
    assert_eq!(legacy_ujy.len(), 2, "stars measured on the Legacy brick");
    assert_eq!(des_ujy.len(), 2, "stars measured on the DES cutout");
    let des_over_legacy: Vec<f64> = des_ujy.iter().zip(&legacy_ujy).map(|(des, legacy)| des / legacy).collect();
    reported("DES r / Legacy r (oracle 1.0312 and 0.9947)", &des_over_legacy);
    for (star, name) in stars.into_iter().enumerate() {
        assert_oracle_flux(&format!("{name} Legacy r flux (uJy)"), legacy_ujy[star], [296.32717, 105.57135][star]);
        assert_oracle_flux(&format!("{name} DES r flux (uJy)"), des_ujy[star], [305.56565, 105.01678][star]);
        assert!(
            (0.98..=1.04).contains(&des_over_legacy[star]),
            "{name} DES r / Legacy r {} is outside 0.98 to 1.04",
            des_over_legacy[star]
        );
    }
    assert_calibration(&result.bands[legacy], JY_PER_NANOMAGGY, LEGACY_PIXEL_SCALE_ARCSEC);
    assert_calibration(&result.bands[des], JY_PER_DES_COUNT, DES_PIXEL_SCALE_ARCSEC);

    assert_ps1_reference_star_lands_on_its_pixel(&bands[ps1].path);
    assert_eq!(ps1_ujy.len(), 2, "stars measured on the PS1 cutout");
    let ps1_over_legacy: Vec<f64> = ps1_ujy.iter().zip(&legacy_ujy).map(|(ps1, legacy)| ps1 / legacy).collect();
    reported("PS1 r / Legacy r (oracle 0.919 and 0.789)", &ps1_over_legacy);
    for (star, name) in stars.into_iter().enumerate() {
        assert_oracle_flux(&format!("{name} PS1 r flux (uJy)"), ps1_ujy[star], [272.33272, 83.28894][star]);
        assert!(
            (0.75..=0.95).contains(&ps1_over_legacy[star]),
            "{name} PS1 r / Legacy r {} is outside 0.75 to 0.95",
            ps1_over_legacy[star]
        );
    }
    assert_calibration(&result.bands[ps1], JY_PER_PS1_COUNT, PS1_PIXEL_SCALE_ARCSEC);
}
