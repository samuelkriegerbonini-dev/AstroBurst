use crate::core::astrometry::gwcs::test_support::env_dir;
use crate::core::imaging::asinh::{
    asinh_scale, linearised_warning, BOFFSET_CARD, BSOFTEN_CARD, LINEARISED_OFFSET_CARD, LINEARISED_SOFTEN_CARD,
};
use crate::core::imaging::dq_flags::{bits_present, DqTable};
use crate::core::imaging::layer_roles::{LayerGuess, LayerRole};
use crate::core::metadata::photcal::{missing_calibration_reason, FluxConvention, PhotCal};
use crate::infra::cache::{ImageCache, ImageEntry};
use crate::infra::image_source::{load_plane_header, ErrOrigin, LoadedCompanions};
use crate::service::common::{load_cached, load_companions};
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;
use crate::types::image_ref::{ImageRef, PlaneSelector};

const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
const VALIDATION_SET: &str = "validation-2026-11-13";
const VISIT_IMAGE: &str = "rubin-testdata_images/visit_image.fits";
const MEDEXP: &str = "rubin-afwdata/medexp.fits";
const PS1_STACK: &str = "ps1-stack-1784-059/rings.v3.skycell.1784.059.stk.r.unconv.fits";
const PS1_WT: &str = "ps1-stack-1784-059/rings.v3.skycell.1784.059.stk.r.unconv.wt.fits";
const PS1_MASK: &str = "ps1-stack-1784-059/rings.v3.skycell.1784.059.stk.r.unconv.mask.fits";
const PS1_WT_FILE: &str = "rings.v3.skycell.1784.059.stk.r.unconv.wt.fits";
const RELATIVE_TOLERANCE: f64 = 1e-6;
const PS1_BLANK_PIXELS: usize = 42846;
const PS1_PROBES: [(usize, usize); 4] = [(3139, 3130), (1000, 1000), (2500, 3000), (4000, 5000)];
const AFW_VISIT_PLANES: [&str; 17] = [
    "BAD",
    "SAT",
    "INTRP",
    "CR",
    "EDGE",
    "DETECTED",
    "DETECTED_NEGATIVE",
    "SUSPECT",
    "NO_DATA",
    "VIGNETTED",
    "STREAK",
    "CROSSTALK",
    "ITL_DIP",
    "NOT_DEBLENDED",
    "PARTLY_VIGNETTED",
    "SPIKE",
    "UNMASKEDNAN",
];
const AFW_MEDEXP_PLANES: [&str; 5] = ["BAD", "SAT", "INTRP", "CR", "EDGE"];
const SAT_BIT: u32 = 1 << 1;
const NO_DATA_BIT: u32 = 1 << 8;

fn data(rel: &str) -> String {
    let path = env_dir(SCIENCE_DATA_ENV).join(VALIDATION_SET).join(rel);
    assert!(path.is_file(), "{} is missing", path.display());
    path.to_str().unwrap().to_string()
}

fn opened(cache: &ImageCache, path: &str) -> (ImageEntry, LoadedCompanions) {
    let active = load_cached(cache, path).unwrap();
    let layers = load_companions(cache, path).unwrap();
    (active, layers)
}

fn close(measured: f32, expected: f64) -> bool {
    ((f64::from(measured) - expected) / expected).abs() <= RELATIVE_TOLERANCE
}

fn assert_probes(what: &str, values: &ndarray::Array2<f32>, probes: &[((usize, usize), f64)]) {
    for ((x, y), expected) in probes {
        let measured = values[[*y, *x]];
        println!("{what}: ({x}, {y}) = {measured:e} (oracle {expected:e})");
        assert!(close(measured, *expected), "{what} at ({x}, {y}): {measured:e} against {expected:e}");
    }
}

fn nan_pixels(values: &ndarray::Array2<f32>) -> usize {
    values.iter().filter(|value| value.is_nan()).count()
}

fn pixels_with(plane: &IntPlane, bit: u32) -> usize {
    plane.bits.iter().filter(|bits| *bits & bit != 0).count()
}

fn plane_names(planes: &[&str]) -> Vec<(u32, String)> {
    planes.iter().enumerate().map(|(n, name)| (1u32 << n, name.to_string())).collect()
}

fn science_guess(basis: &str) -> Option<LayerGuess> {
    Some(LayerGuess { role: LayerRole::Science, basis: basis.to_string(), assumed: false })
}

fn assert_marked_linear(what: &str, header: &HduHeader, soften: f64, offset: f64) {
    println!(
        "{what}: BSOFTEN {:?}, BOFFSET {:?}, ABSOFTEN {:?}, ABOFFSET {:?}, warning {:?}",
        header.get(BSOFTEN_CARD),
        header.get(BOFFSET_CARD),
        header.get(LINEARISED_SOFTEN_CARD),
        header.get(LINEARISED_OFFSET_CARD),
        linearised_warning(header)
    );
    assert_eq!(header.get(BSOFTEN_CARD), None, "{what}");
    assert_eq!(header.get(BOFFSET_CARD), None, "{what}");
    assert_eq!(asinh_scale(header), None, "{what}");
    assert_eq!(header.get_f64(LINEARISED_SOFTEN_CARD), Some(soften), "{what}");
    assert_eq!(header.get_f64(LINEARISED_OFFSET_CARD), Some(offset), "{what}");
    assert!(linearised_warning(header).is_some(), "{what}");
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: validation-2026-11-13/rubin-testdata_images/visit_image.fits"]
fn real_rubin_visit_image_opens_with_its_layers() {
    let path = data(VISIT_IMAGE);
    let cache = ImageCache::new(8, usize::MAX);
    let (active, layers) = opened(&cache, &path);
    let info = active.plane_info().unwrap();
    println!("visit image: plane {:?}, layer {:?}, notes {:?}", info.kind, info.pairing.layer, info.pairing.notes);
    assert_eq!(info.kind, PlaneSelector::Hdu(1));
    assert_eq!(info.pairing.layer, science_guess("EXTNAME IMAGE"));
    assert_eq!(active.arr().dim(), (250, 256));
    assert_eq!(nan_pixels(active.arr()), 0);
    assert_probes(
        "visit image",
        active.arr(),
        &[
            ((0, 0), -18.510839462280273),
            ((128, 125), -19.438613891601562),
            ((255, 249), -49.85410690307617),
            ((37, 200), -18.63532066345215),
        ],
    );

    let err = layers.err.as_ref().expect("the VARIANCE HDU is the error layer");
    println!("visit image: error origin {:?}", layers.err_origin);
    assert_eq!(active.companions().unwrap().err, Some(ImageRef::hdu(&path, 3)));
    assert_probes(
        "visit image sigma",
        err.arr(),
        &[
            ((0, 0), 21.929170510694412),
            ((128, 125), 21.907756584917234),
            ((255, 249), 22.117270288612275),
            ((37, 200), 21.894117622654022),
        ],
    );
    assert_eq!(nan_pixels(err.arr()), 0);
    assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("nJy"));
    assert_eq!(
        layers.err_origin,
        Some(ErrOrigin {
            role: LayerRole::Variance,
            source: "EXTNAME VARIANCE, HDU 3: square root".to_string(),
            assumed: false,
            n_no_data: 0,
        })
    );
    assert_eq!(layers.err_unavailable, None);

    assert_eq!(active.companions().unwrap().dq, Some(ImageRef::hdu(&path, 2)));
    let (dq, table) = layers.dq.as_ref().expect("the MASK HDU is the mask layer");
    let plane = dq.int_plane().expect("the MASK HDU is read as integers");
    let flags = layers.flags.as_ref().expect("a mask layer has its flag set");
    println!(
        "visit image: mask origin {:?}, flag set {:?}, {} planes, bits present {}, SAT pixels {}, NO_DATA pixels {}",
        layers.dq_origin,
        flags.source,
        flags.flags.len(),
        bits_present(&plane.bits),
        pixels_with(plane, SAT_BIT),
        pixels_with(plane, NO_DATA_BIT)
    );
    assert_eq!(*table, DqTable::Unknown);
    assert_eq!(layers.dq_origin.as_deref(), Some("EXTNAME MASK, HDU 2"));
    assert_eq!(flags.source, "MP_ cards (afw mask planes)");
    assert_eq!(flags.flags, plane_names(&AFW_VISIT_PLANES));
    assert_eq!(plane.bits.dim(), (250, 256));
    assert_eq!(bits_present(&plane.bits), 32951);
    assert_eq!(pixels_with(plane, SAT_BIT), 50);
    assert_eq!(pixels_with(plane, NO_DATA_BIT), 0);
    assert_eq!(plane.value_at(7, 228), 32823);
    assert_eq!(flags.decode(32823), vec!["BAD", "SAT", "INTRP", "EDGE", "DETECTED", "SPIKE"]);

    let header = active.header().unwrap();
    let calibration = PhotCal::from_header(header, None).expect("nJy is a calibration by unit");
    println!(
        "visit image: calibration {:?}, origin {:?}, warnings {:?}",
        calibration.convention, calibration.origin, calibration.warnings
    );
    assert_eq!(calibration.convention, FluxConvention::FluxDensityPerPixel { jy_per_unit: 1e-9, unit: "nJy".to_string() });
    assert_eq!(calibration.jy_per_native_unit, Some(1e-9));
    assert!(calibration.warnings.is_empty(), "{:?}", calibration.warnings);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: validation-2026-11-13/rubin-afwdata/medexp.fits"]
fn real_afwdata_medexp_opens_with_its_layers() {
    let path = data(MEDEXP);
    let cache = ImageCache::new(8, usize::MAX);
    let (active, layers) = opened(&cache, &path);
    let info = active.plane_info().unwrap();
    println!(
        "medexp: plane {:?}, layer {:?}, companions {:?}, notes {:?}",
        info.kind,
        info.pairing.layer,
        active.companions(),
        info.pairing.notes
    );
    assert_eq!(info.kind, PlaneSelector::Hdu(1));
    assert_eq!(info.pairing.layer, science_guess("EXTTYPE IMAGE"));
    assert_eq!(active.arr().dim(), (700, 500));
    assert_eq!(nan_pixels(active.arr()), 0);
    assert_probes(
        "medexp",
        active.arr(),
        &[((0, 0), 2791.0), ((250, 350), 2773.0), ((499, 699), 2820.0), ((123, 456), 2809.0)],
    );

    println!("medexp: error origin {:?}, unavailable {:?}", layers.err_origin, layers.err_unavailable);
    assert_eq!(active.companions().unwrap().err, Some(ImageRef::hdu(&path, 3)));
    let err = layers.err.as_ref().expect("the VARIANCE HDU is the error layer");
    assert_probes(
        "medexp sigma",
        err.arr(),
        &[
            ((0, 0), 41.962880034397664),
            ((250, 350), 41.82734578560717),
            ((499, 699), 42.1803249106774),
            ((123, 456), 42.097977933551036),
        ],
    );
    assert_eq!(nan_pixels(err.arr()), 0);
    assert_eq!(err.arr().iter().filter(|sigma| **sigma == 0.0).count(), 2800);
    assert_eq!(
        layers.err_origin,
        Some(ErrOrigin {
            role: LayerRole::Variance,
            source: "EXTTYPE VARIANCE, HDU 3: square root".to_string(),
            assumed: false,
            n_no_data: 0,
        })
    );

    println!("medexp: mask origin {:?}, unavailable {:?}", layers.dq_origin, layers.dq_unavailable);
    assert_eq!(active.companions().unwrap().dq, Some(ImageRef::hdu(&path, 2)));
    let (dq, _) = layers.dq.as_ref().expect("the MASK HDU is the mask layer");
    let plane = dq.int_plane().expect("the unsigned 16-bit MASK HDU is read as integers");
    let flags = layers.flags.as_ref().expect("a mask layer has its flag set");
    println!(
        "medexp: flag set {:?}, planes {:?}, bits present {}, SAT pixels {}, value(321, 138) {}",
        flags.source,
        flags.flags,
        bits_present(&plane.bits),
        pixels_with(plane, SAT_BIT),
        plane.value_at(321, 138)
    );
    assert_eq!(layers.dq_origin.as_deref(), Some("EXTTYPE MASK, HDU 2"));
    assert_eq!(flags.source, "MP_ cards (afw mask planes)");
    assert_eq!(flags.flags, plane_names(&AFW_MEDEXP_PLANES));
    assert_eq!(bits_present(&plane.bits), 3);
    assert_eq!(pixels_with(plane, SAT_BIT), 180);
    assert_eq!(pixels_with(plane, 1), 2800);
    assert_eq!(plane.value_at(321, 138), 2);

    let header = active.header().unwrap();
    let reason = missing_calibration_reason(Some(header));
    println!("medexp: calibration {:?}, reason {reason:?}", PhotCal::from_header(header, None));
    assert_eq!(PhotCal::from_header(header, None), None);
    assert_eq!(
        reason,
        "no photometric calibration in the header: BUNIT is missing and there is no zero-point card (MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP)"
    );
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: validation-2026-11-13/ps1-stack-1784-059 stack file"]
fn real_ps1_stack_pixels_are_linear_after_load() {
    let path = data(PS1_STACK);
    let cache = ImageCache::new(8, usize::MAX);
    let active = load_cached(&cache, &path).unwrap();
    println!("ps1 stack: plane {:?}, {:?} px", active.plane_info().map(|info| &info.kind), active.arr().dim());
    assert_eq!(active.arr().dim(), (6261, 6279));
    let probes: Vec<((usize, usize), f64)> = PS1_PROBES
        .into_iter()
        .zip([61.337097050629126, -80.77539022426848, -19.224335312282676, 68.98209812282987])
        .collect();
    assert_probes("ps1 stack", active.arr(), &probes);
    let blank = nan_pixels(active.arr());
    println!("ps1 stack: {blank} NaN pixels (oracle {PS1_BLANK_PIXELS} raw values equal to BLANK)");
    assert_eq!(blank, PS1_BLANK_PIXELS);
    assert!(active.arr()[[0, 0]].is_nan());
    assert!(active.arr()[[6260, 6278]].is_nan());

    let header = active.header().unwrap();
    assert_marked_linear("ps1 stack header of the loaded image", header, 107.0458825164, 5.960201263428);
    let read_alone = load_plane_header(&ImageRef::auto(&path)).unwrap();
    assert_marked_linear("ps1 stack header read without pixels", &read_alone, 107.0458825164, 5.960201263428);
}

#[test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: validation-2026-11-13/ps1-stack-1784-059 stack, wt and mask files"]
fn real_ps1_stack_finds_its_weight_and_mask_files() {
    let path = data(PS1_STACK);
    let cache = ImageCache::new(8, usize::MAX);
    let (active, layers) = opened(&cache, &path);
    let info = active.plane_info().unwrap();
    println!(
        "ps1 stack: layer {:?}, companions {:?}, notes {:?}",
        info.pairing.layer,
        active.companions(),
        info.pairing.notes
    );
    println!("ps1 stack: error origin {:?}, unavailable {:?}", layers.err_origin, layers.err_unavailable);
    println!("ps1 stack: mask origin {:?}, unavailable {:?}", layers.dq_origin, layers.dq_unavailable);
    assert_eq!(active.companions().unwrap().err, Some(ImageRef::auto(&data(PS1_WT))));
    assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&data(PS1_MASK))));

    let err = layers.err.as_ref().expect("the .wt file is the error layer");
    let probes: Vec<((usize, usize), f64)> = PS1_PROBES
        .into_iter()
        .zip([65.72028237377383, 58.65044345045991, 65.20902502416406, 81.21893935444245])
        .collect();
    assert_probes("ps1 sigma", err.arr(), &probes);
    let no_data = nan_pixels(err.arr());
    println!("ps1 sigma: {no_data} NaN pixels (oracle {PS1_BLANK_PIXELS})");
    assert_eq!(no_data, PS1_BLANK_PIXELS);
    let err_header = err.header().unwrap();
    assert_marked_linear("ps1 sigma header", err_header, 1784.677115966, 5079.578125);
    let origin = layers.err_origin.as_ref().expect("origin of the error layer");
    assert_eq!((origin.role, origin.assumed, origin.n_no_data), (LayerRole::Variance, false, PS1_BLANK_PIXELS as u64));
    assert!(origin.source.contains(PS1_WT_FILE), "{}", origin.source);
    assert!(origin.source.contains("PS_COVAR"), "{}", origin.source);
    assert!(origin.source.ends_with("square root"), "{}", origin.source);

    let (dq, _) = layers.dq.as_ref().expect("the .mask file is the mask layer");
    let plane = dq.int_plane().expect("the unsigned 16-bit mask is read as integers");
    let flags = layers.flags.as_ref().expect("a mask layer has its flag set");
    println!(
        "ps1 mask: flag set {:?}, exclusion {}, saturated {}, bits present {}, SAT pixels {}, value(0, 0) {}",
        flags.source,
        layers.exclusion_mask(),
        layers.saturated_mask(),
        bits_present(&plane.bits),
        pixels_with(plane, 32),
        plane.value_at(0, 0)
    );
    assert_eq!(flags.source, "MSKNAM cards (PS1 mask)");
    assert_eq!(layers.exclusion_mask(), 8575);
    assert_eq!(layers.saturated_mask(), 32);
    assert_eq!(bits_present(&plane.bits), 30719);
    assert_eq!(pixels_with(plane, 32), 24248);
    assert_eq!(plane.value_at(0, 0), 8251);
    assert_eq!(plane.value_at(3130, 3139), 0);

    let calibration = PhotCal::from_header(active.header().unwrap(), None).expect("the stack has FPA.ZP and EXPTIME");
    println!(
        "ps1 stack: calibration {:?}, origin {:?}, Jy per count {:?}",
        calibration.convention, calibration.origin, calibration.jy_per_native_unit
    );
    let FluxConvention::SurveyZeroPoint { zp_ab, source, exptime_applied } = &calibration.convention else {
        panic!("not a survey zero point: {:?}", calibration.convention);
    };
    assert!((zp_ab - 32.14333124107817).abs() <= 1e-9, "{zp_ab}");
    assert_eq!(source, "FPA.ZP + 2.5 log10(EXPTIME)");
    assert_eq!(*exptime_applied, Some(720.0));
    let factor = calibration.jy_per_native_unit.unwrap();
    assert!(((factor - 5.042750760695831e-10) / 5.042750760695831e-10).abs() <= 1e-9, "{factor:e}");
}
