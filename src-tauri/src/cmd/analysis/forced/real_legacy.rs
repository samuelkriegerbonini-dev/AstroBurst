use super::*;
use crate::core::astrometry::gwcs::test_support::env_dir;
use crate::infra::tables::{read_text_table, TextTable};
use crate::types::image_ref::BandPlaneKey;

const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
const BRICK_CATALOGUE: &str = "survey/catalogs/C1b_ls_dr10_brick_0532m280.csv";
const BOX_CATALOGUE: &str = "survey/catalogs/C1_ls_dr10_box_A.csv";
const CUTOUT: &str = "survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits";
const ID_COLUMN: &str = "ls_id";
const REFERENCE_STAR: &str = "10995300315432397";
const SATURATED_STAR: &str = "10995300315434171";
const STAR_WITH_ONE_LOST_PIXEL: &str = "10995300315434517";
const JY_PER_NANOMAGGY: f64 = 3.630780547701009e-6;
const TWO_ARCSEC_IN_PIXELS: f64 = 7.633587786259541;
const SEVEN_ARCSEC_IN_PIXELS: f64 = 26.717557251908396;
const NMAD_PER_MAD: f64 = 1.4826;
const BRIGHT_SNR: f64 = 50.0;
const ALONE_FRACFLUX: f64 = 0.01;
const BAILOUT_ROW_BESIDE_SATURATION: &str = "10995300315431731";
const SATUR_R: u32 = 8;
const BAILOUT: u32 = 1024;

pub(super) fn data(rel: &str) -> String {
    let path = env_dir(SCIENCE_DATA_ENV).join(rel);
    assert!(path.is_file(), "{} is missing", path.display());
    path.to_str().unwrap().to_string()
}

pub(super) fn catalogue(rel: &str) -> TextTable {
    read_text_table(&data(rel)).unwrap()
}

pub(super) fn column<'a>(table: &'a TextTable, name: &str) -> &'a [f64] {
    table.f64_column(name).unwrap()
}

fn positions(table: &TextTable, id_column: &str) -> Vec<ForcedSkyPosition> {
    let ids = table.text_column(id_column).unwrap();
    let (ra, dec) = (column(table, "ra"), column(table, "dec"));
    (0..table.n_rows)
        .map(|row| ForcedSkyPosition { id: ids[row].clone(), ra_deg: ra[row], dec_deg: dec[row] })
        .collect()
}

pub(super) fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let upper = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        0.5 * (sorted[upper - 1] + sorted[upper])
    } else {
        sorted[upper]
    }
}

pub(super) fn nmad(values: &[f64]) -> f64 {
    let centre = median(values);
    let deviations: Vec<f64> = values.iter().map(|value| (value - centre).abs()).collect();
    NMAD_PER_MAD * median(&deviations)
}

pub(super) fn report(name: &str, n: usize, median: f64, nmad: f64, min: f64, max: f64) {
    println!("{name}: n {n} median {median:.7} nmad {nmad:.3e} min {min:.6} max {max:.6}");
}

pub(super) fn reported(name: &str, values: &[f64]) -> (f64, f64) {
    let (centre, spread) = (median(values), nmad(values));
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    report(name, values.len(), centre, spread, min, max);
    (centre, spread)
}

pub(super) fn assert_within(name: &str, measured: f64, target: f64, tolerance: f64) {
    assert!((measured - target).abs() <= tolerance, "{name} {measured} is not within {tolerance} of {target}");
}

fn run(
    bands: &[ForcedBandInput],
    positions: &[ForcedSkyPosition],
    radius_arcsec: f64,
    correction: ForcedCorrection,
    options: &ForcedRunOptions,
) -> ForcedPhotometryResult {
    let aperture = ForcedAperture {
        radius_arcsec: Some(radius_arcsec),
        radius_px: None,
        annulus_arcsec: None,
        annulus_px: None,
        correction,
        reference_radius_arcsec: None,
    };
    forced_photometry(bands, positions, &aperture, options, None).unwrap()
}

fn brick_bands(bands: &[&str]) -> Vec<ForcedBandInput> {
    bands
        .iter()
        .map(|band| ForcedBandInput {
            path: data(&format!("survey/L2/legacysurvey-0532m280-image-{band}.fits.fz")),
            label: None,
            dataset: None,
        })
        .collect()
}

fn brick_run(bands: &[&str], radius_arcsec: f64, correction: ForcedCorrection) -> ForcedPhotometryResult {
    let brick_positions = positions(&catalogue(BRICK_CATALOGUE), ID_COLUMN);
    assert_eq!(brick_positions.len(), 69, "rows of C1b");
    run(&brick_bands(bands), &brick_positions, radius_arcsec, correction, &ForcedRunOptions::default())
}

pub(super) fn cell(result: &ForcedPhotometryResult, row: usize, band: usize) -> &ForcedCell {
    result.sources[row].cells[band].as_ref().unwrap()
}

fn cell_of<'a>(result: &'a ForcedPhotometryResult, id: &str, band: usize) -> &'a ForcedCell {
    let row = result.sources.iter().position(|source| source.id == id).unwrap();
    cell(result, row, band)
}

fn native_flux(result: &ForcedPhotometryResult, row: usize, band: usize) -> f64 {
    cell(result, row, band).flux_native.unwrap()
}

pub(super) fn has_whole_aperture(result: &ForcedPhotometryResult, row: usize, band: usize) -> bool {
    cell(result, row, band).masked_fraction == Some(0.0)
}

fn catalogue_snr(table: &TextTable, row: usize) -> f64 {
    column(table, "apflux_r_5")[row] * column(table, "apflux_ivar_r_5")[row].sqrt()
}

fn is_unmasked_and_alone(table: &TextTable, row: usize) -> bool {
    column(table, "maskbits")[row] == 0.0 && column(table, "fracflux_r")[row] < ALONE_FRACFLUX
}

fn point_source_rows(table: &TextTable) -> Vec<usize> {
    let kind = table.text_column("type").unwrap();
    (0..table.n_rows).filter(|&row| kind[row] == "PSF" && is_unmasked_and_alone(table, row)).collect()
}

fn bright_rows(table: &TextTable) -> Vec<usize> {
    (0..table.n_rows)
        .filter(|&row| is_unmasked_and_alone(table, row) && catalogue_snr(table, row) > BRIGHT_SNR)
        .collect()
}

fn isolated_star_rows(table: &TextTable, two_arcsec_r: &ForcedPhotometryResult) -> Vec<usize> {
    let anymask = column(table, "anymask_r");
    point_source_rows(table)
        .into_iter()
        .filter(|&row| anymask[row] == 0.0 && has_whole_aperture(two_arcsec_r, row, 0))
        .collect()
}

fn whole_aperture_rows(result: &ForcedPhotometryResult, band: usize) -> Vec<usize> {
    (0..result.sources.len()).filter(|&row| has_whole_aperture(result, row, band)).collect()
}

pub(super) fn print_band(band: &ForcedBand) {
    println!(
        "band {} ({}): pixel_scale_arcsec {:?} aperture_radius_px {:?} bunit {:?} jy_per_native_unit {:?} mag_system {:?}",
        band.label, band.path, band.pixel_scale_arcsec, band.aperture_radius_px, band.bunit, band.jy_per_native_unit, band.mag_system
    );
    println!("  calibration_origin {:?}", band.calibration_origin);
    println!("  err_origin {:?}", band.err_origin);
    println!("  dq_source {:?}", band.dq_source);
    println!("  filter {:?}", band.filter);
    println!("  n_measured {} n_failed {} warnings {:?}", band.n_measured, band.n_failed, band.warnings);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_r_two_arcsec_fluxes_match_apflux_r_5() {
    let table = catalogue(BRICK_CATALOGUE);
    let selections = [("F1", point_source_rows(&table), 21), ("Q", bright_rows(&table), 38)];
    for (name, rows, expected) in &selections {
        assert_eq!(rows.len(), *expected, "{name} rows of C1b");
    }
    let result = brick_run(&["r"], 2.0, ForcedCorrection::None);
    let band = &result.bands[0];
    print_band(band);
    let reference = cell_of(&result, REFERENCE_STAR, 0);
    println!("reference star at ({:.6}, {:.6}) px", reference.x, reference.y);
    let catalogue_flux = column(&table, "apflux_r_5");
    let measured: Vec<(&str, f64, f64)> = selections
        .iter()
        .map(|(name, rows, _)| {
            let ratios: Vec<f64> = rows.iter().map(|&row| native_flux(&result, row, 0) / catalogue_flux[row]).collect();
            let (centre, spread) = reported(&format!("{name} flux_native / apflux_r_5"), &ratios);
            (*name, centre, spread)
        })
        .collect();
    assert_within("aperture_radius_px", band.aperture_radius_px.unwrap(), TWO_ARCSEC_IN_PIXELS, 1e-9);
    assert_within("pixel_scale_arcsec", band.pixel_scale_arcsec.unwrap(), 0.262, 1e-9);
    for (name, centre, spread) in measured {
        assert_within(&format!("{name} median ratio (research target)"), centre, 1.0, 0.01);
        assert!((centre - 1.0).abs() < 3e-4, "{name} median ratio {centre} is not within 3e-4 of 1 (planner pin)");
        assert!(spread < 5e-4, "{name} nmad {spread} is not below 5e-4 (planner pin)");
    }
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_r_formal_error_is_within_five_percent_of_the_catalogue() {
    let table = catalogue(BRICK_CATALOGUE);
    let rows = bright_rows(&table);
    assert_eq!(rows.len(), 38, "Q rows of C1b");
    let result = brick_run(&["r"], 2.0, ForcedCorrection::None);
    let band = &result.bands[0];
    print_band(band);
    let catalogue_ivar = column(&table, "apflux_ivar_r_5");
    let ratios: Vec<f64> =
        rows.iter().map(|&row| cell(&result, row, 0).flux_err_native.unwrap() * catalogue_ivar[row].sqrt()).collect();
    let (centre, _) = reported("Q flux_err_native x sqrt(apflux_ivar_r_5)", &ratios);
    assert_eq!(band.err_origin.as_ref().unwrap().role, "inverse_variance");
    assert_within("median error ratio (research target)", centre, 1.0, 0.05);
    assert!(
        (centre - 0.9844).abs() <= 0.005,
        "median error ratio {centre} is not within 0.005 of 0.9844: the catalogue error is sqrt(sum w sigma^2) and this app's is sqrt(sum (w sigma)^2), so the catalogue is 1.6 percent higher by construction"
    );
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_g_and_z_fluxes_match_their_catalogue_apertures() {
    let table = catalogue(BRICK_CATALOGUE);
    let result = brick_run(&["g", "r", "z"], 2.0, ForcedCorrection::None);
    result.bands.iter().for_each(print_band);
    let (g, r, z) = (0, 1, 2);
    let clean_g = whole_aperture_rows(&result, g);
    let clean_z = whole_aperture_rows(&result, z);
    println!("CLEAN(g) n {} CLEAN(z) n {}", clean_g.len(), clean_z.len());
    assert_eq!(clean_g.len(), 67, "CLEAN(g) rows of C1b");
    assert_eq!(clean_z.len(), 66, "CLEAN(z) rows of C1b");
    let ratios_of = |rows: &[usize], band: usize, catalogue_column: &str| -> Vec<f64> {
        let catalogue_flux = column(&table, catalogue_column);
        rows.iter().map(|&row| native_flux(&result, row, band) / catalogue_flux[row]).collect()
    };
    let (g_median, _) = reported("CLEAN(g) flux_native / apflux_g_5", &ratios_of(&clean_g, g, "apflux_g_5"));
    let (z_median, _) = reported("CLEAN(z) flux_native / apflux_z_5", &ratios_of(&clean_z, z, "apflux_z_5"));
    let colour = cell_of(&result, REFERENCE_STAR, g).mag_ab.unwrap() - cell_of(&result, REFERENCE_STAR, r).mag_ab.unwrap();
    println!("reference star g - r {colour:.7} AB mag");
    let pivots: Vec<Option<f64>> =
        result.bands.iter().map(|band| band.filter.as_ref().map(|filter| filter.pivot_um)).collect();
    println!("pivot_um g, r, z: {pivots:?}");
    assert_within("CLEAN(g) median ratio", g_median, 1.0, 0.01);
    assert_within("CLEAN(z) median ratio", z_median, 1.0, 0.01);
    assert_within("reference star g - r", colour, 1.0460, 0.005);
    if let [Some(pivot_g), Some(pivot_r), Some(pivot_z)] = pivots[..] {
        assert!(pivot_g < pivot_r && pivot_r < pivot_z, "pivot wavelengths {pivots:?} do not increase g, r, z");
    }
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_one_over_two_arcsec_flux_ratio_of_isolated_stars_matches_the_catalogue() {
    let table = catalogue(BRICK_CATALOGUE);
    let two_arcsec = brick_run(&["r"], 2.0, ForcedCorrection::None);
    let rows = isolated_star_rows(&table, &two_arcsec);
    assert_eq!(rows.len(), 16, "ISO rows of C1b");
    let one_arcsec = brick_run(&["r"], 1.0, ForcedCorrection::None);
    let (catalogue_one, catalogue_two) = (column(&table, "apflux_r_3"), column(&table, "apflux_r_5"));
    let measured: Vec<f64> =
        rows.iter().map(|&row| native_flux(&one_arcsec, row, 0) / native_flux(&two_arcsec, row, 0)).collect();
    let listed: Vec<f64> = rows.iter().map(|&row| catalogue_one[row] / catalogue_two[row]).collect();
    let (measured_median, _) = reported("ISO flux(1.0 arcsec) / flux(2.0 arcsec)", &measured);
    let (listed_median, _) = reported("ISO apflux_r_3 / apflux_r_5", &listed);
    assert_within("ISO median flux ratio", measured_median, listed_median, 0.01);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_apertures_on_zero_inverse_variance_report_what_is_missing() {
    let two_arcsec = brick_run(&["r"], 2.0, ForcedCorrection::None);
    let one_arcsec = brick_run(&["r"], 1.0, ForcedCorrection::None);
    let saturated = cell_of(&two_arcsec, SATURATED_STAR, 0);
    let saturated_core = cell_of(&one_arcsec, SATURATED_STAR, 0);
    let lost_pixel = cell_of(&two_arcsec, STAR_WITH_ONE_LOST_PIXEL, 0);
    println!(
        "{SATURATED_STAR} 2.0 arcsec: masked_fraction {:?} flux_native {:?} n_masked {} saturated {}",
        saturated.masked_fraction, saturated.flux_native, saturated.n_masked, saturated.saturated
    );
    println!(
        "{SATURATED_STAR} 1.0 arcsec: masked_fraction {:?} flux_native {:?} error {:?}",
        saturated_core.masked_fraction, saturated_core.flux_native, saturated_core.error
    );
    println!(
        "{STAR_WITH_ONE_LOST_PIXEL} 2.0 arcsec: masked_fraction {:?} flux_native {:?} ratio to the catalogue 774.9868 {:?}",
        lost_pixel.masked_fraction,
        lost_pixel.flux_native,
        lost_pixel.flux_native.map(|flux| flux / 774.9868)
    );
    println!("band warnings {:?}", two_arcsec.bands[0].warnings);
    assert_within("saturated star masked_fraction", saturated.masked_fraction.unwrap(), 0.704, 0.005);
    assert_within("saturated star flux_native", saturated.flux_native.unwrap(), 91.55489, 0.01 * 91.55489);
    assert_eq!(saturated_core.error.as_deref(), Some("the aperture holds no pixel with data"));
    assert_eq!(saturated_core.masked_fraction, Some(1.0));
    assert_within("lost pixel masked_fraction", lost_pixel.masked_fraction.unwrap(), 0.00546, 5e-4);
    assert_within("lost pixel flux_native", lost_pixel.flux_native.unwrap(), 752.14, 0.001 * 752.14);
    assert!(
        two_arcsec.bands[0].warnings.iter().any(|warning| warning.contains("hold pixels without data")),
        "{:?}",
        two_arcsec.bands[0].warnings
    );
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_reference_star_is_calibrated_to_jy_and_ab() {
    let result = brick_run(&["r"], 7.0, ForcedCorrection::None);
    let band = &result.bands[0];
    print_band(band);
    let star = cell_of(&result, REFERENCE_STAR, 0);
    let flux_ujy = star.flux_jy.unwrap() * 1e6;
    println!(
        "{REFERENCE_STAR} 7.0 arcsec: flux_native {:?} nanomaggy flux {flux_ujy:.5} uJy mag_ab {:?}",
        star.flux_native, star.mag_ab
    );
    assert_within("mag_ab (research target)", star.mag_ab.unwrap(), 17.684281, 0.02);
    assert_within("flux (uJy, planner pin)", flux_ujy, 306.395, 0.001 * 306.395);
    assert_within("jy_per_native_unit", band.jy_per_native_unit.unwrap(), JY_PER_NANOMAGGY, 1e-6 * JY_PER_NANOMAGGY);
    assert_eq!(band.bunit.as_deref(), Some("nanomaggy"));
    assert!(band.calibration_origin.is_some());
    assert_within("aperture_radius_px", band.aperture_radius_px.unwrap(), SEVEN_ARCSEC_IN_PIXELS, 1e-9);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_flag_bits_under_each_aperture_contain_the_catalogue_maskbits() {
    let table = catalogue(BRICK_CATALOGUE);
    assert_eq!(table.n_rows, 69, "rows of C1b");
    let maskbits: Vec<u32> = column(&table, "maskbits").iter().map(|&bits| bits as u32).collect();
    let bailout_only: Vec<usize> = (0..table.n_rows).filter(|&row| maskbits[row] == BAILOUT).collect();
    assert_eq!(bailout_only.len(), 4, "rows of C1b whose maskbits is exactly BAILOUT");
    let result = brick_run(&["r"], 2.0, ForcedCorrection::None);
    let band = &result.bands[0];
    print_band(band);
    println!("flag_names {:?}", band.flag_names);
    let containing =
        (0..table.n_rows).filter(|&row| cell(&result, row, 0).flags_or.unwrap() & maskbits[row] == maskbits[row]).count();
    println!("apertures whose flags_or contains the catalogue maskbits: {containing} of {}", table.n_rows);
    let excluding = run(
        &brick_bands(&["r"]),
        &positions(&table, ID_COLUMN),
        2.0,
        ForcedCorrection::None,
        &ForcedRunOptions { exclude_dq: Some(true), ..ForcedRunOptions::default() },
    );
    for &row in &bailout_only {
        let (plain, excluded) = (cell(&result, row, 0), cell(&excluding, row, 0));
        println!(
            "BAILOUT-only row {} at ({:.3}, {:.3}) px: flags_or {:?}; masked_fraction {:?} without and {:?} with exclude_dq (n_masked {})",
            result.sources[row].id, plain.x, plain.y, plain.flags_or, plain.masked_fraction, excluded.masked_fraction, excluded.n_masked
        );
    }
    println!("warnings with exclude_dq {:?}", excluding.bands[0].warnings);
    assert_eq!(containing, 69);
    for (value, name) in [(1, "NPRIMARY"), (BAILOUT, "BAILOUT")] {
        assert!(band.flag_names.contains(&ForcedFlagName { value, name: name.to_string() }), "{:?}", band.flag_names);
    }
    let (bailout_alone, beside_saturation): (Vec<usize>, Vec<usize>) =
        bailout_only.iter().copied().partition(|&row| cell(&result, row, 0).flags_or == Some(BAILOUT));
    assert_eq!(bailout_alone.len(), 3, "BAILOUT-only rows whose aperture holds no other flag bit");
    for row in bailout_alone {
        assert_eq!(
            cell(&excluding, row, 0).masked_fraction,
            Some(0.0),
            "BAILOUT is not a pixel defect, yet exclude_dq masked pixels of {}",
            result.sources[row].id
        );
    }
    let [neighbour] = beside_saturation[..] else {
        panic!("one BAILOUT-only row has saturated pixels of a neighbour under its aperture, got {beside_saturation:?}");
    };
    assert_eq!(result.sources[neighbour].id, BAILOUT_ROW_BESIDE_SATURATION);
    assert_eq!(cell(&result, neighbour, 0).flags_or.unwrap() & SATUR_R, SATUR_R);
    assert_within(
        "masked_fraction of the SATUR_R pixels of the neighbour",
        cell(&excluding, neighbour, 0).masked_fraction.unwrap(),
        0.4348,
        0.005,
    );
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
fn real_legacy_l2_growth_correction_from_two_to_three_and_a_half_arcsec() {
    let result = brick_run(&["r"], 2.0, ForcedCorrection::GrowthCurve);
    let band = &result.bands[0];
    println!("aperture {:?}", result.aperture);
    println!("aperture_correction {:?}", band.aperture_correction);
    println!("band warnings {:?}", band.warnings);
    let correction = band.aperture_correction.as_ref().unwrap();
    println!(
        "factor {} scatter {} n_stars {} n_candidates {}",
        correction.factor, correction.scatter, correction.n_stars, correction.n_candidates
    );
    let star = cell_of(&result, REFERENCE_STAR, 0);
    assert_eq!(result.aperture.reference_radius_arcsec, Some(3.5));
    assert_within("factor (first-run target)", correction.factor, 1.0427, 0.03);
    assert!(correction.n_stars >= 3, "n_stars {}", correction.n_stars);
    assert_within("factor (planner pin)", correction.factor, 1.041, 0.01);
    assert!(correction.scatter < 0.02, "scatter {} is not below 0.02 (planner pin)", correction.scatter);
    assert_eq!(star.flux_total_jy.unwrap(), star.flux_jy.unwrap() * correction.factor);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1 and survey/catalogs"]
fn real_legacy_l1_cutout_r_plane_matches_apflux_r_5() {
    let table = catalogue(BOX_CATALOGUE);
    let bright: Vec<usize> = (0..table.n_rows).filter(|&row| catalogue_snr(&table, row) > BRIGHT_SNR).collect();
    assert_eq!(bright.len(), 35, "C1 rows above S/N 50");
    let all_positions = positions(&table, ID_COLUMN);
    let bright_positions: Vec<ForcedSkyPosition> = bright.iter().map(|&row| all_positions[row].clone()).collect();
    let cutout = data(CUTOUT);
    let bands: Vec<ForcedBandInput> = [0, 1, 3]
        .into_iter()
        .map(|plane| ForcedBandInput {
            path: BandPlaneKey::new(&cutout, 0, plane).key(),
            label: None,
            dataset: Some("DECAM".to_string()),
        })
        .collect();
    let result = run(&bands, &bright_positions, 2.0, ForcedCorrection::None, &ForcedRunOptions::default());
    result.bands.iter().for_each(print_band);
    let r = 1;
    let whole: Vec<usize> = (0..bright.len())
        .filter(|&k| cell(&result, k, r).on_image && has_whole_aperture(&result, k, r))
        .collect();
    println!("C1 rows above S/N {BRIGHT_SNR}: {}; on the image with a whole aperture: {}", bright.len(), whole.len());
    for k in (0..bright.len()).filter(|k| !whole.contains(k)) {
        let partial = cell(&result, k, r);
        println!(
            "not whole: {} at ({:.3}, {:.3}) px on_image {} masked_fraction {:?} n_masked {} n_nonfinite {} flux_native {:?} apflux_r_5 {}",
            result.sources[k].id,
            partial.x,
            partial.y,
            partial.on_image,
            partial.masked_fraction,
            partial.n_masked,
            partial.n_nonfinite,
            partial.flux_native,
            column(&table, "apflux_r_5")[bright[k]]
        );
    }
    assert_eq!(whole.len(), 33, "C1 rows above S/N 50 whose aperture lies on the cutout with a masked fraction of 0");
    let catalogue_flux = column(&table, "apflux_r_5");
    let ratio_of = |k: usize| native_flux(&result, k, r) / catalogue_flux[bright[k]];
    let every_ratio: Vec<f64> = (0..bright.len()).map(ratio_of).collect();
    reported("L1 r plane flux_native / apflux_r_5, the 35 rows with the two clipped by the edge", &every_ratio);
    let ratios: Vec<f64> = whole.iter().copied().map(ratio_of).collect();
    let (centre, _) = reported("L1 r plane flux_native / apflux_r_5", &ratios);
    let band = &result.bands[r];
    assert_within("median ratio (research target)", centre, 1.0, 0.03);
    assert_within("median ratio (planner pin)", centre, 1.0, 0.005);
    let labels: Vec<&str> = result.bands.iter().map(|band| band.label.as_str()).collect();
    assert_eq!(labels, ["g", "r", "z"]);
    assert_within("jy_per_native_unit", band.jy_per_native_unit.unwrap(), JY_PER_NANOMAGGY, 1e-6 * JY_PER_NANOMAGGY);
    assert!(
        band.warnings.iter().any(|warning| warning.contains("nanomaggy") && warning.contains("SURVEY = LegacySurvey")),
        "{:?}",
        band.warnings
    );
}
