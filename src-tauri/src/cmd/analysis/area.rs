use ndarray::Array2;

use crate::cmd::common::{load_cached, source_path};
use crate::core::analysis::photometry::{area_map_mean, MaskedImage};
use crate::core::metadata::photcal::{is_relative_pixel_area, FluxConvention, PhotCal};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::dispatcher::list_source_extensions;
use crate::infra::fits::reader::HduInfo;
use crate::types::image_ref::ImageRef;

pub(crate) const AREA_EXTNAME: &str = "AREA";
const SCI_EXTNAME: &str = "SCI";
const PIXAR_SR_KEY: &str = "PIXAR_SR";
const DEFAULT_EXTVER: i64 = 1;

pub(crate) fn extname_is(name: Option<&str>, wanted: &str) -> bool {
    name.is_some_and(|n| n.trim().trim_matches('\'').trim().eq_ignore_ascii_case(wanted))
}

pub(crate) fn holds_one_plane(hdu: &HduInfo) -> bool {
    hdu.has_data && (hdu.naxis == 2 || (hdu.naxis == 3 && hdu.naxis3 == 1))
}

enum AreaLookup {
    Found(HduInfo),
    OtherExtver { area: i64, sci: i64 },
    Absent,
}

fn area_lookup(path: &str, entry: &ImageEntry) -> AreaLookup {
    let Some(info) = entry.plane_info() else {
        return AreaLookup::Absent;
    };
    if !extname_is(info.extname.as_deref(), SCI_EXTNAME) {
        return AreaLookup::Absent;
    }
    let Ok(hdus) = list_source_extensions(&source_path(path)) else {
        return AreaLookup::Absent;
    };
    let candidates: Vec<HduInfo> = hdus
        .into_iter()
        .filter(|h| holds_one_plane(h) && extname_is(h.extname.as_deref(), AREA_EXTNAME))
        .collect();
    let sci = info.extver.unwrap_or(DEFAULT_EXTVER);
    let (matching, other): (Vec<HduInfo>, Vec<HduInfo>) =
        candidates.into_iter().partition(|h| h.extver.unwrap_or(DEFAULT_EXTVER) == sci);
    match (matching.into_iter().next(), other.first()) {
        (Some(hdu), _) => AreaLookup::Found(hdu),
        (None, Some(hdu)) => AreaLookup::OtherExtver { area: hdu.extver.unwrap_or(DEFAULT_EXTVER), sci },
        (None, None) => AreaLookup::Absent,
    }
}

fn located_area(path: &str, entry: &ImageEntry) -> Result<Option<HduInfo>, String> {
    match area_lookup(path, entry) {
        AreaLookup::Found(hdu) if same_size(&hdu, entry) => Ok(Some(hdu)),
        AreaLookup::Found(hdu) => {
            let (rows, cols) = entry.arr().dim();
            Err(format!("its size {} x {} differs from the image {cols} x {rows}", hdu.naxis1, hdu.naxis2))
        }
        AreaLookup::OtherExtver { area, sci } => Err(format!("EXTVER {area} differs from SCI EXTVER {sci}")),
        AreaLookup::Absent => Ok(None),
    }
}

fn load_area(path: &str, hdu: &HduInfo) -> Result<ImageEntry, String> {
    load_cached(&ImageRef::hdu(&source_path(path), hdu.index).cache_key())
        .map_err(|e| format!("HDU {} failed to load: {e:#}", hdu.index))
}

fn same_size(hdu: &HduInfo, entry: &ImageEntry) -> bool {
    let (rows, cols) = entry.arr().dim();
    hdu.naxis1 == cols as i64 && hdu.naxis2 == rows as i64
}

pub(crate) fn area_companion(path: &str, entry: &ImageEntry) -> Result<Option<ImageEntry>, String> {
    located_area(path, entry)?.map(|hdu| load_area(path, &hdu)).transpose()
}

fn calibration_refusal(photcal: Option<&PhotCal>, entry: &ImageEntry) -> Option<String> {
    let Some(cal) = photcal else {
        return Some("the header has no photometric calibration".into());
    };
    if cal.pixar_from_header() {
        return None;
    }
    let has_pixar_card = entry
        .header()
        .and_then(|h| h.get_f64(PIXAR_SR_KEY))
        .is_some_and(|v| v.is_finite() && v > 0.0);
    Some(match &cal.convention {
        FluxConvention::JwstMjySr { .. } if has_pixar_card => {
            "PIXAR_SR was replaced by the WCS pixel area (resampled image)".into()
        }
        FluxConvention::JwstMjySr { .. } => "PIXAR_SR is missing, so the pixel area comes from the WCS".into(),
        _ => format!("{} is not JWST MJy/sr", cal.label()),
    })
}

fn ignored(reason: String) -> String {
    format!("AREA extension ignored: {reason}")
}

pub(crate) struct AreaWeights {
    map: Option<ImageEntry>,
}

impl AreaWeights {
    pub(crate) fn resolve(path: &str, entry: &ImageEntry, photcal: Option<&mut PhotCal>, warnings: &mut Vec<String>) -> Self {
        Self { map: area_map(path, entry, photcal, warnings) }
    }

    pub(crate) fn masked_image<'a>(
        &'a self,
        entry: &'a ImageEntry,
        excluded: Option<&'a Array2<u8>>,
    ) -> Result<MaskedImage<'a>, String> {
        MaskedImage::new(entry.arr(), excluded).map(|source| source.with_area(self.map.as_ref().map(|a| a.arr())))
    }

    #[cfg(test)]
    pub(crate) fn map(&self) -> Option<&Array2<f32>> {
        self.map.as_ref().map(|a| a.arr())
    }
}

fn area_map(path: &str, entry: &ImageEntry, photcal: Option<&mut PhotCal>, warnings: &mut Vec<String>) -> Option<ImageEntry> {
    let hdu = match located_area(path, entry) {
        Ok(Some(hdu)) => hdu,
        Ok(None) => return None,
        Err(reason) => {
            warnings.push(ignored(reason));
            return None;
        }
    };
    if let Some(reason) = calibration_refusal(photcal.as_deref(), entry) {
        warnings.push(ignored(reason));
        return None;
    }
    let area = match load_area(path, &hdu) {
        Ok(entry) => entry,
        Err(reason) => {
            warnings.push(ignored(reason));
            return None;
        }
    };
    let mean = area_map_mean(area.arr());
    if !is_relative_pixel_area(mean) {
        warnings.push(ignored(format!("map mean {mean:.2e} is not a relative pixel area (expected about 1)")));
        return None;
    }
    if let Some(cal) = photcal {
        cal.apply_area_map(hdu.index, mean);
    }
    Some(area)
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};

    pub const CAL_SIZE: usize = 64;
    pub const CAL_PIXAR_SR: f64 = 2.29249e-14;
    pub const CAL_APERTURE_RADIUS: f64 = 5.0;
    pub const CAL_EXCLUDED_PIXEL: (usize, usize) = (31, 34);
    pub const AREA_BLOCK_ROWS: std::ops::Range<usize> = 30..35;
    pub const AREA_BLOCK_COLS: std::ops::Range<usize> = 32..38;
    pub const AREA_BLOCK_VALUE: f32 = 1.02;

    #[derive(Clone, Copy)]
    pub struct CalPlanes {
        pub err: bool,
        pub dq: bool,
    }

    impl CalPlanes {
        pub const ALL: CalPlanes = CalPlanes { err: true, dq: true };
        pub const SCI_ERR: CalPlanes = CalPlanes { err: true, dq: false };
        pub const SCI_DQ: CalPlanes = CalPlanes { err: false, dq: true };
    }

    pub struct CalFixture {
        pub key: String,
        pub path: String,
        pub sci: ndarray::Array2<f32>,
        pub err: ndarray::Array2<f32>,
    }

    pub fn gaussian_pixels(size: usize, amp: f32, sigma: f32, bg: f32) -> Vec<f32> {
        let c = (size / 2) as f32;
        (0..size * size)
            .map(|i| {
                let x = (i % size) as f32 - c;
                let y = (i / size) as f32 - c;
                bg + amp * (-(x * x + y * y) / (2.0 * sigma * sigma)).exp()
            })
            .collect()
    }

    pub fn square(values: &[f32]) -> ndarray::Array2<f32> {
        ndarray::Array2::from_shape_vec((CAL_SIZE, CAL_SIZE), values.to_vec()).unwrap()
    }

    pub fn cal_sci_cards() -> Vec<(&'static str, String)> {
        vec![
            ("BUNIT", "'MJy/sr'".into()),
            ("PIXAR_SR", "2.29249E-14".into()),
            ("PIXAR_A2", "9.7536E-4".into()),
            ("PHOTMJSR", "0.5".into()),
            ("TELESCOP", "'JWST'".into()),
            ("INSTRUME", "'NIRCAM'".into()),
        ]
    }

    pub fn cal_sci_pixels() -> Vec<f32> {
        gaussian_pixels(CAL_SIZE, 1000.0, 2.0, 100.0)
            .into_iter()
            .enumerate()
            .map(|(i, v)| v + 0.5 * ((((i / CAL_SIZE) * 7 + (i % CAL_SIZE) * 13) % 5) as f32 - 2.0))
            .collect()
    }

    pub fn cal_err_pixels() -> Vec<f32> {
        (0..CAL_SIZE * CAL_SIZE).map(|i| 0.3 + 0.002 * (i % 17) as f32).collect()
    }

    pub fn area_hdu(values: Vec<f32>, size: usize, extver: Option<i64>) -> TestHdu {
        TestHdu { extname: Some("AREA"), extver, cols: size, rows: size, data: HduData::F32(values), extra_cards: vec![] }
    }

    pub fn uniform_area(value: f32) -> Vec<f32> {
        vec![value; CAL_SIZE * CAL_SIZE]
    }

    pub fn blocked_area() -> Vec<f32> {
        let mut area = uniform_area(1.0);
        for y in AREA_BLOCK_ROWS {
            for x in AREA_BLOCK_COLS {
                area[y * CAL_SIZE + x] = AREA_BLOCK_VALUE;
            }
        }
        area
    }

    pub fn jwst_cal_mef(
        path: &std::path::Path,
        primary_cards: &[(&str, String)],
        sci_cards: Vec<(&'static str, String)>,
        planes: CalPlanes,
        extra: Vec<TestHdu>,
    ) -> CalFixture {
        let size = CAL_SIZE;
        let sci = cal_sci_pixels();
        let err = cal_err_pixels();
        let mut hdus = vec![TestHdu {
            extname: Some("SCI"),
            extver: Some(1),
            cols: size,
            rows: size,
            data: HduData::F32(sci.clone()),
            extra_cards: sci_cards,
        }];
        if planes.err {
            hdus.push(TestHdu {
                extname: Some("ERR"),
                extver: Some(1),
                cols: size,
                rows: size,
                data: HduData::F32(err.clone()),
                extra_cards: vec![("BUNIT", "'MJy/sr'".into())],
            });
        }
        if planes.dq {
            let mut dq = vec![0i32 - 2147483647 - 1; size * size];
            dq[CAL_EXCLUDED_PIXEL.0 * size + CAL_EXCLUDED_PIXEL.1] = 1 - 2147483647 - 1;
            hdus.push(TestHdu {
                extname: Some("DQ"),
                extver: Some(1),
                cols: size,
                rows: size,
                data: HduData::I32(dq),
                extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())],
            });
        }
        hdus.extend(extra);
        write_test_mef(path, primary_cards, &hdus);
        let path = path.to_str().unwrap().to_string();
        CalFixture { key: format!("{path}#hdu=1"), path, sci: square(&sci), err: square(&err) }
    }
}
