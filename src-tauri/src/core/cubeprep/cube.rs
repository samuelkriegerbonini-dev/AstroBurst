use ndarray::{s, Array2};
use serde::{Deserialize, Serialize};

use crate::core::alignment::wcs_reproject::smallest_odd_support;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::cubeprep::align::{noise_correlation_header, same_grid, NoiseSource, CARD_NCORRK, CARD_NCORRSRC, NOISCORR_EXTNAME};
use crate::core::cubeprep::footprint::{CropRounding, FootprintError};
use crate::core::imaging::cutout::{shift_header, CutoutRect};
use crate::core::imaging::dq_flags::DqTable;
use crate::core::imaging::push_history;
use crate::core::metadata::filter_wavelengths::{FilterEntry, FilterRegistry};
use crate::core::metadata::photcal::card_text;
use crate::core::metadata::units::{classify_unit, UnitClass, UnitInfo, CARD_BUNIT_IN, CARD_FLUXCONV};
use crate::infra::fits::table::{BintableData, ColumnValues};
use crate::infra::fits::writer::{hierarch_card_fits, is_wcs_card, TableColumn, TableValues};
use crate::infra::wcs_source::header_wcs;
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;

pub const CTYPE_FILTER: &str = "FILTER";
pub const FILTERS_EXTNAME: &str = "FILTERS";
pub const UNKNOWN_UNIT: &str = "unknown";
pub const LIGHTSTACK_CITATION: &str = "Prepared following LIGHTSTACK (Wille & Pessi 2026, doi:10.5281/zenodo.19393438)";
pub const DEFAULT_FRAC_VALID: f64 = 0.01;
pub const DQ_DO_NOT_USE: u32 = 1;

const HIERARCH_PREFIX: &str = "HIERARCH ";
const PLANE_KEY_PREFIXES: [&str; 2] = ["FILTER", "DATASET"];
const PLANE_SUMMARY_KEYS: [&str; 3] = ["NFILTERS", "FILTERS", "DATASETS"];
const CENTRE_KEYS: [&str; 4] = ["GAL_RA", "GAL_DEC", "GAL_XCEN", "GAL_YCEN"];
const PIXEL_BOX_KEYS: [&str; 4] = ["XMINPIX", "XMAXPIX", "YMINPIX", "YMAXPIX"];
const SIP_ORDER_KEYS: [&str; 6] = ["A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER", "A_DMAX", "B_DMAX"];
const SIP_RESIDUAL_KEYS: [&str; 2] = ["SIPMXERR", "SIPIVERR"];
const SIP_COEFFICIENT_PREFIXES: [&str; 4] = ["A_", "B_", "AP_", "BP_"];
const SIP_SUFFIX: &str = "-SIP";
const MATRIX_PREFIXES: [&str; 2] = ["CD", "PC"];
const PARAMETER_PREFIXES: [&str; 2] = ["PV", "PS"];
const AXIS_INDEXED_PREFIXES: [&str; 9] = ["CTYPE", "CRVAL", "CRPIX", "CDELT", "CUNIT", "CROTA", "CNAME", "CRDER", "CSYER"];
const HDU_IDENTITY_KEYS: [&str; 6] = ["EXTNAME", "EXTVER", "XTENSION", "PCOUNT", "GCOUNT", "INHERIT"];
const SIP_WARNING: &str = "the reference grid has SIP distortion, which the lightstack layout does not carry (as LIGHTSTACK); the mef layout keeps it";
const COLUMN_FILTER: &str = "FILTER";
const COLUMN_DATASET: &str = "DATASET";
const COLUMN_INSTRUMENT: &str = "INSTRUME";
const COLUMN_PIVOT: &str = "PIVOT_UM";
const COLUMN_PIXSCALE: &str = "PIXSCALE";
const COLUMN_BUNIT_IN: &str = "BUNIT_IN";
const COLUMN_FLUXCONV: &str = "FLUXCONV";
const COLUMN_PSF_FWHM: &str = "PSF_FWHM";
const COLUMN_KERNEL: &str = "KERNEL";
pub const COLUMN_NOISCORR_ALIGN: &str = "NOISCORR_ALIGN";
pub const COLUMN_NOISCORR_MATCH: &str = "NOISCORR_MATCH";
pub const COLUMN_MATCH: &str = "MATCH";
const UNMATCHED_NOISCORR_MATCH: f64 = 1.0;
const CARD_PHOTFLAM: &str = "PHOTFLAM";
const CARD_PHOTPLAM: &str = "PHOTPLAM";
const UNIT_MICRON: &str = "um";
const UNIT_ARCSEC: &str = "arcsec";
const ANGSTROM_PER_MICRON: f64 = 1e4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CubeLayout {
    #[default]
    Lightstack,
    Mef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneOrder {
    #[default]
    Wavelength,
    Given,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CubePlane {
    pub filter: String,
    pub dataset: String,
    pub instrument: String,
    pub pivot_um: Option<f64>,
    pub pixscale_arcsec: Option<f64>,
    pub bunit_in: String,
    pub fluxconv: Option<f64>,
    pub psf_fwhm_arcsec: Option<f64>,
    pub kernel: String,
}

impl CubePlane {
    pub fn from_entry(entry: &FilterEntry, header: &HduHeader, wcs: &WcsTransform) -> Self {
        let scale = wcs.pixel_scale_arcsec();
        Self {
            filter: entry.filter.clone(),
            dataset: entry.dataset.clone(),
            instrument: entry.instrument.clone(),
            pivot_um: Some(entry.pivot_angstrom / ANGSTROM_PER_MICRON),
            pixscale_arcsec: (scale.is_finite() && scale > 0.0).then_some(scale),
            bunit_in: card_text(header, CARD_BUNIT_IN).or_else(|| card_text(header, "BUNIT")).unwrap_or_default(),
            fluxconv: Some(header.get_f64(CARD_FLUXCONV).filter(|v| v.is_finite()).unwrap_or(1.0)),
            psf_fwhm_arcsec: None,
            kernel: String::new(),
        }
    }

    fn named(filter: String, dataset: String, entry: Option<&FilterEntry>) -> Self {
        Self {
            filter,
            dataset,
            instrument: entry.map(|e| e.instrument.clone()).unwrap_or_default(),
            pivot_um: entry.map(|e| e.pivot_angstrom / ANGSTROM_PER_MICRON),
            pixscale_arcsec: None,
            bunit_in: String::new(),
            fluxconv: None,
            psf_fwhm_arcsec: None,
            kernel: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DqPlanes {
    pub planes: Vec<Array2<u32>>,
    pub signed: bool,
}

#[derive(Debug, Clone)]
pub struct Cube {
    pub header: HduHeader,
    pub planes: Vec<Array2<f32>>,
    pub err: Option<Vec<Array2<f32>>>,
    pub dq: Option<DqPlanes>,
    pub filters: Vec<CubePlane>,
}

impl Cube {
    pub fn dims(&self) -> (usize, usize) {
        self.planes.first().map_or((0, 0), Array2::dim)
    }

    pub fn wcs(&self) -> Result<WcsTransform, CubeError> {
        header_wcs(&self.header).map_err(|error| CubeError::NoWcs(format!("{error:#}")))
    }
}

pub struct CubeInput {
    pub label: String,
    pub sci: Array2<f32>,
    pub err: Option<Array2<f32>>,
    pub dq: Option<IntPlane>,
    pub header: HduHeader,
    pub wcs: WcsTransform,
    pub plane: CubePlane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CubeOptions {
    pub layout: CubeLayout,
    pub order: PlaneOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    Matched,
    Reference,
    Unmatched,
    External,
}

impl MatchStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Reference => "reference",
            Self::Unmatched => "unmatched",
            Self::External => "external",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Matched, Self::Reference, Self::Unmatched, Self::External].into_iter().find(|status| status.label() == text.trim())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputNoise {
    pub correlation: Array2<f32>,
    pub noiscorr_align: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaneNoise {
    pub filter: String,
    pub correlation: Array2<f32>,
    pub noiscorr_align: f64,
    pub noiscorr_match: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CubeNoise {
    pub planes: Vec<PlaneNoise>,
    pub source: NoiseSource,
}

fn centred_pad(map: &Array2<f32>, k: usize) -> Array2<f32> {
    let offset = (k - map.dim().0) / 2;
    let mut padded = Array2::zeros((k, k));
    padded.slice_mut(s![offset..offset + map.dim().0, offset..offset + map.dim().1]).assign(map);
    padded
}

impl CubeNoise {
    pub fn k(&self) -> usize {
        self.planes.iter().map(|plane| plane.correlation.dim().0).max().unwrap_or(1)
    }

    pub fn padded_maps(&self) -> Vec<Array2<f32>> {
        let k = self.k();
        self.planes.iter().map(|plane| centred_pad(&plane.correlation, k)).collect()
    }

    pub fn header(&self) -> HduHeader {
        noise_correlation_header(self.k(), self.source)
    }

    pub fn for_planes(&self, planes: &[CubePlane]) -> Result<CubeNoise, CubeError> {
        let planes = planes
            .iter()
            .map(|plane| self.planes.iter().find(|noise| noise.filter == plane.filter).cloned().ok_or_else(|| CubeError::NoiseFilter(plane.filter.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CubeNoise { planes, source: self.source })
    }

    pub fn from_hdu(table: &BintableData, header: &HduHeader, maps: Vec<Array2<f32>>) -> anyhow::Result<CubeNoise> {
        let declared = card_text(header, CARD_NCORRSRC).unwrap_or_default();
        let source = NoiseSource::parse(&declared).ok_or_else(|| anyhow::anyhow!("{NOISCORR_EXTNAME} {CARD_NCORRSRC}='{declared}' is neither 'align' nor 'match'"))?;
        let k = header
            .get_i64(CARD_NCORRK)
            .filter(|k| *k > 0 && k % 2 == 1)
            .ok_or_else(|| anyhow::anyhow!("{NOISCORR_EXTNAME} {CARD_NCORRK}={} is not a positive odd size", header.get(CARD_NCORRK).unwrap_or("absent")))?;
        if let Some(map) = maps.iter().find(|map| map.nrows() != map.ncols() || map.nrows() as i64 != k) {
            anyhow::bail!("{NOISCORR_EXTNAME} {CARD_NCORRK}={k} disagrees with its {}x{} maps", map.ncols(), map.nrows());
        }
        let filters = table.text_column(COLUMN_FILTER)?;
        let align = finite_column(table, COLUMN_NOISCORR_ALIGN)?;
        let matched = finite_column(table, COLUMN_NOISCORR_MATCH)?;
        if [filters.len(), align.len(), matched.len()].iter().any(|&rows| rows != maps.len()) {
            anyhow::bail!("{FILTERS_EXTNAME} has {} rows but {NOISCORR_EXTNAME} has {} planes", filters.len(), maps.len());
        }
        let planes = maps
            .iter()
            .zip(filters)
            .enumerate()
            .map(|(z, (map, filter))| PlaneNoise {
                filter,
                correlation: smallest_odd_support(map),
                noiscorr_align: align[z].unwrap_or(f64::NAN),
                noiscorr_match: matched[z].unwrap_or(f64::NAN),
            })
            .collect();
        Ok(CubeNoise { planes, source })
    }
}

pub fn match_statuses(table: &BintableData) -> anyhow::Result<Option<Vec<MatchStatus>>> {
    let Ok(labels) = table.text_column(COLUMN_MATCH) else {
        return Ok(None);
    };
    labels
        .iter()
        .enumerate()
        .map(|(z, label)| MatchStatus::parse(label).ok_or_else(|| anyhow::anyhow!("{FILTERS_EXTNAME} row {}: unknown {COLUMN_MATCH} value '{label}'", z + 1)))
        .collect::<anyhow::Result<Vec<_>>>()
        .map(Some)
}

#[derive(Debug, Clone)]
pub struct AssembledCube {
    pub cube: Cube,
    pub order: Vec<usize>,
    pub bunit: String,
    pub warnings: Vec<String>,
    pub noise: Option<CubeNoise>,
}

impl AssembledCube {
    pub fn filters_table(&self) -> Result<FiltersTable, CubeError> {
        let table = FiltersTable::new(&self.cube.filters).with_match(&vec![MatchStatus::Unmatched; self.cube.filters.len()])?;
        match &self.noise {
            Some(noise) => table.with_noise(noise),
            None => Ok(table),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CubeError {
    #[error("no plane to build a cube from")]
    Empty,
    #[error("{label} is {cols}x{rows} but the reference grid is {ref_cols}x{ref_rows}")]
    ShapeMismatch { label: String, cols: usize, rows: usize, ref_cols: usize, ref_rows: usize },
    #[error("{label} is not on the pixel grid of the reference {reference}; align the images first")]
    OffGrid { label: String, reference: String },
    #[error("mixed units: {label} has BUNIT {unit} but the reference {reference} has {reference_unit}; convert the images to one unit first")]
    MixedUnits { label: String, unit: String, reference: String, reference_unit: String },
    #[error("mixed calibrations: {label} and the reference {reference} are both in {unit} but their {cards} differ, so the planes do not share one physical scale; convert the images to Jy per pixel first")]
    MixedCalibration { label: String, unit: String, reference: String, cards: String },
    #[error("filter {filter} appears twice, in {first} and {second}")]
    DuplicateFilter { filter: String, first: String, second: String },
    #[error("{label} ({filter}) has no pivot wavelength to order by; give the planes in order")]
    NoPivot { label: String, filter: String },
    #[error("the cube has no celestial WCS: {0}")]
    NoWcs(String),
    #[error("plane {index} has no FILTER{index} keyword and the file has no FILTERS table: not a filter cube")]
    UnlabelledPlane { index: usize },
    #[error("filter {filter} is not in the cube; its filters are {}", filters.join(", "))]
    FilterNotFound { filter: String, filters: Vec<String> },
    #[error("removing {0} would leave the cube without planes")]
    WouldBeEmpty(String),
    #[error("no valid filters: no plane has a fraction of at least {frac_valid} of its valid pixels above {threshold}; lower threshold or frac_valid")]
    NoValidPlanes { threshold: f64, frac_valid: f64 },
    #[error("the pixel box x {x_start}..{x_end}, y {y_start}..{y_end} does not lie inside the {cols}x{rows} cube")]
    BoxOutside { x_start: i64, x_end: i64, y_start: i64, y_end: i64, cols: usize, rows: usize },
    #[error("reference slice {index} is outside the cube range 0-{last}")]
    SliceOutside { index: usize, last: usize },
    #[error("reference slice {0} contains no valid pixels")]
    EmptyReference(usize),
    #[error("the {0} mask leaves no valid pixel")]
    EmptyMask(&'static str),
    #[error("{0} has {1} planes, the cube has {2}")]
    CompanionMismatch(&'static str, usize, usize),
    #[error("{label}: the {NOISCORR_EXTNAME} map is {cols}x{rows}; it must be square with an odd size")]
    NoiseMap { label: String, rows: usize, cols: usize },
    #[error("the {NOISCORR_EXTNAME} maps do not line up with the {0} plane")]
    NoiseFilter(String),
    #[error(transparent)]
    Footprint(#[from] FootprintError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    #[default]
    Reference,
    Union,
    Intersection,
}

fn default_frac_valid() -> f64 {
    DEFAULT_FRAC_VALID
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CubeEdit {
    DropInvalid {
        #[serde(default)]
        threshold: f64,
        #[serde(default = "default_frac_valid")]
        frac_valid: f64,
        #[serde(default)]
        use_dq: bool,
    },
    RemoveFilter {
        filter: String,
    },
    Cut {
        x_start: i64,
        x_end: i64,
        y_start: i64,
        y_end: i64,
    },
    CutSky {
        ra: f64,
        dec: f64,
        size_arcsec: f64,
        #[serde(default)]
        rounding: CropRounding,
    },
    Header {
        #[serde(default)]
        redshift: Option<f64>,
        #[serde(default)]
        ra_center: Option<f64>,
        #[serde(default)]
        dec_center: Option<f64>,
        #[serde(default)]
        use_brightest_pixel: bool,
    },
    ReferenceNanMask {
        #[serde(default)]
        reference_slice: usize,
        #[serde(default)]
        mode: MaskMode,
    },
}

impl CubeEdit {
    pub fn suffix(&self) -> &'static str {
        match self {
            Self::DropInvalid { .. } => "valid",
            Self::RemoveFilter { .. } => "removed",
            Self::Cut { .. } | Self::CutSky { .. } => "cut",
            Self::Header { .. } => "more",
            Self::ReferenceNanMask { .. } => "masked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CubeCentre {
    pub x: Option<usize>,
    pub y: Option<usize>,
    pub ra: f64,
    pub dec: f64,
    pub kind: &'static str,
}

pub(crate) fn set_text(header: &mut HduHeader, key: &str, value: &str) {
    header.set(key, value.to_string());
    if let Some(keys) = header.string_keys.as_mut() {
        keys.insert(key.to_string());
    }
}

fn put_first(header: &mut HduHeader, key: &str, value: String) {
    header.remove(key);
    header.cards.insert(0, (key.to_string(), value.clone()));
    header.index.insert(key.to_string(), value);
}

pub fn plane_key(prefix: &str, index: usize) -> String {
    let key = format!("{prefix}{index}");
    if key.len() <= 8 {
        key
    } else {
        format!("{HIERARCH_PREFIX}{key}")
    }
}

pub fn plane_text(header: &HduHeader, prefix: &str, index: usize) -> Option<String> {
    let key = format!("{prefix}{index}");
    card_text(header, &key).or_else(|| card_text(header, &format!("{HIERARCH_PREFIX}{key}")))
}

fn is_plane_key(key: &str) -> bool {
    let bare = key.strip_prefix(HIERARCH_PREFIX).unwrap_or(key).trim();
    PLANE_KEY_PREFIXES.iter().any(|prefix| {
        bare.strip_prefix(prefix).is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn remove_keys(header: &mut HduHeader, doomed: impl Fn(&str) -> bool) {
    let keys: Vec<String> = header.cards.iter().map(|(key, _)| key.clone()).filter(|key| doomed(key)).collect();
    for key in keys {
        header.remove(&key);
    }
}

pub fn remove_plane_keys(header: &mut HduHeader) {
    remove_keys(header, |key| is_plane_key(key) || PLANE_SUMMARY_KEYS.contains(&key));
}

pub fn write_plane_keys(header: &mut HduHeader, filters: &[CubePlane]) {
    remove_plane_keys(header);
    for (i, plane) in filters.iter().enumerate() {
        set_text(header, &plane_key("FILTER", i + 1), &plane.filter);
        set_text(header, &plane_key("DATASET", i + 1), &plane.dataset);
    }
    header.set("NFILTERS", filters.len().to_string());
    let join = |field: fn(&CubePlane) -> &str| filters.iter().map(field).collect::<Vec<_>>().join(",");
    set_text(header, "FILTERS", &join(|plane| &plane.filter));
    set_text(header, "DATASETS", &join(|plane| &plane.dataset));
    header.set("NAXIS3", filters.len().to_string());
}

fn axis_number(text: &str) -> Option<usize> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

fn axis_pair(key: &str, prefix: &str) -> Option<(usize, usize)> {
    let (i, j) = key.strip_prefix(prefix)?.split_once('_')?;
    Some((axis_number(i)?, axis_number(j)?))
}

fn is_matrix_card(key: &str) -> bool {
    MATRIX_PREFIXES.iter().any(|prefix| axis_pair(key, prefix).is_some())
}

pub fn is_cube_wcs_card(key: &str) -> bool {
    is_wcs_card(key) || is_matrix_card(key)
}

fn is_higher_axis_card(key: &str) -> bool {
    let indexed = AXIS_INDEXED_PREFIXES.iter().any(|prefix| key.strip_prefix(prefix).and_then(axis_number).is_some_and(|n| n >= 3));
    let matrix = MATRIX_PREFIXES.iter().any(|prefix| axis_pair(key, prefix).is_some_and(|(i, j)| i >= 3 || j >= 3));
    let parameter = PARAMETER_PREFIXES.iter().any(|prefix| axis_pair(key, prefix).is_some_and(|(i, _)| i >= 3));
    indexed || matrix || parameter
}

pub fn is_cube_axis_card(key: &str) -> bool {
    key == "WCSAXES" || is_higher_axis_card(key)
}

pub fn cube_base_header(reference: &HduHeader) -> HduHeader {
    let unit = classify_unit(reference);
    let calibration = calibration_keys(&unit);
    let mut base = HduHeader::empty();
    for (key, value) in &reference.cards {
        let key = key.trim();
        if (is_cube_wcs_card(key) && !is_higher_axis_card(key)) || key == "BUNIT" || calibration.contains(&key) {
            base.cards.push((key.to_string(), value.clone()));
            base.index.insert(key.to_string(), value.clone());
        }
    }
    base.inherit_string_keys(reference);
    base
}

pub fn companion_header(cube_header: &HduHeader, bunit: bool) -> HduHeader {
    let mut header = HduHeader::empty();
    for (key, value) in &cube_header.cards {
        let key = key.trim();
        if is_cube_wcs_card(key) || (bunit && key == "BUNIT") {
            header.cards.push((key.to_string(), value.clone()));
            header.index.insert(key.to_string(), value.clone());
        }
    }
    header.inherit_string_keys(cube_header);
    header
}

fn is_sip_card(key: &str) -> bool {
    SIP_ORDER_KEYS.contains(&key)
        || SIP_RESIDUAL_KEYS.contains(&key)
        || SIP_COEFFICIENT_PREFIXES.iter().any(|prefix| axis_pair(key, prefix).is_some())
}

fn strip_sip(header: &mut HduHeader) -> bool {
    let had_sip = header.cards.iter().any(|(key, _)| is_sip_card(key.trim()));
    remove_keys(header, |key| is_sip_card(key.trim()));
    for key in ["CTYPE1", "CTYPE2"] {
        if let Some(ctype) = card_text(header, key).filter(|ctype| ctype.ends_with(SIP_SUFFIX)) {
            set_text(header, key, ctype.trim_end_matches(SIP_SUFFIX));
        }
    }
    had_sip
}

fn has_cd_matrix(header: &HduHeader) -> bool {
    ["CD1_1", "CD1_2", "CD2_1", "CD2_2"].iter().any(|key| header.get(key).is_some())
}

pub fn cube_header(
    base: HduHeader,
    layout: CubeLayout,
    bunit: &str,
    filters: &[CubePlane],
    dims: (usize, usize),
    warnings: &mut Vec<String>,
) -> HduHeader {
    let mut header = base;
    if layout == CubeLayout::Lightstack && strip_sip(&mut header) {
        warnings.push(SIP_WARNING.to_string());
    }
    put_first(&mut header, "WCSAXES", "3".to_string());
    set_text(&mut header, "CTYPE3", CTYPE_FILTER);
    header.set_f64("CRPIX3", 1.0);
    header.set_f64("CRVAL3", 0.0);
    header.set_f64("CDELT3", 1.0);
    if has_cd_matrix(&header) {
        header.set_f64("CD3_3", 1.0);
    }
    set_text(&mut header, "BUNIT", bunit);
    header.set("NAXIS1", dims.1.to_string());
    header.set("NAXIS2", dims.0.to_string());
    write_plane_keys(&mut header, filters);
    for (i, plane) in filters.iter().enumerate() {
        for key in [plane_key("FILTER", i + 1), plane_key("DATASET", i + 1)] {
            if !hierarch_card_fits(&header, &key) {
                warnings.push(format!(
                    "plane {} ({}): {key} is left out of the file because its value does not fit one 80-character HIERARCH card",
                    i + 1,
                    plane.filter
                ));
            }
        }
    }
    push_history(&mut header, LIGHTSTACK_CITATION);
    header
}

fn close(x: f64, y: f64) -> bool {
    (x - y).abs() <= 1e-12 * x.abs().max(y.abs())
}

fn same_unit(a: &UnitInfo, b: &UnitInfo) -> bool {
    match (&a.class, &b.class) {
        (UnitClass::SurfaceBrightness { jy_per_sr_per_unit: x }, UnitClass::SurfaceBrightness { jy_per_sr_per_unit: y })
        | (UnitClass::PerPixelPhysical { jy_per_unit: x }, UnitClass::PerPixelPhysical { jy_per_unit: y }) => close(*x, *y),
        (UnitClass::SurfaceBrightness { .. } | UnitClass::PerPixelPhysical { .. }, _)
        | (_, UnitClass::SurfaceBrightness { .. } | UnitClass::PerPixelPhysical { .. }) => false,
        _ => a.bunit == b.bunit,
    }
}

fn calibration_keys(info: &UnitInfo) -> Vec<&str> {
    let UnitClass::PerPixelCalibrated { cards } = &info.class else {
        return Vec::new();
    };
    let pivot = cards.iter().any(|card| card == CARD_PHOTFLAM).then_some(CARD_PHOTPLAM);
    cards.iter().map(String::as_str).chain(pivot).collect()
}

fn differing_calibration(header: &HduHeader, info: &UnitInfo, anchor: &HduHeader, anchor_info: &UnitInfo) -> Vec<String> {
    let mut keys: Vec<&str> = Vec::new();
    for key in calibration_keys(info).into_iter().chain(calibration_keys(anchor_info)) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.into_iter()
        .filter(|key| match (header.get_f64(key), anchor.get_f64(key)) {
            (Some(value), Some(reference)) => !close(value, reference),
            (value, reference) => value.is_some() != reference.is_some(),
        })
        .map(String::from)
        .collect()
}

fn unit_text(info: &UnitInfo) -> String {
    info.bunit.clone().unwrap_or_else(|| "(missing)".to_string())
}

fn check_inputs(inputs: &[CubeInput], anchor: &CubeInput) -> Result<(), CubeError> {
    let dims = anchor.sci.dim();
    let anchor_unit = classify_unit(&anchor.header);
    let mismatch = |label: String, (rows, cols): (usize, usize)| CubeError::ShapeMismatch { label, cols, rows, ref_cols: dims.1, ref_rows: dims.0 };
    for input in inputs {
        if input.sci.dim() != dims {
            return Err(mismatch(input.label.clone(), input.sci.dim()));
        }
        if let Some(err) = input.err.as_ref().filter(|err| err.dim() != dims) {
            return Err(mismatch(format!("{} ERR", input.label), err.dim()));
        }
        if let Some(dq) = input.dq.as_ref().filter(|dq| dq.bits.dim() != dims) {
            return Err(mismatch(format!("{} DQ", input.label), dq.bits.dim()));
        }
        if !same_grid(&input.wcs, dims, &anchor.wcs, dims) {
            return Err(CubeError::OffGrid { label: input.label.clone(), reference: anchor.label.clone() });
        }
        let unit = classify_unit(&input.header);
        if !same_unit(&unit, &anchor_unit) {
            return Err(CubeError::MixedUnits {
                label: input.label.clone(),
                unit: unit_text(&unit),
                reference: anchor.label.clone(),
                reference_unit: unit_text(&anchor_unit),
            });
        }
        let differing = differing_calibration(&input.header, &unit, &anchor.header, &anchor_unit);
        if !differing.is_empty() {
            return Err(CubeError::MixedCalibration {
                label: input.label.clone(),
                unit: unit_text(&unit),
                reference: anchor.label.clone(),
                cards: differing.join(", "),
            });
        }
    }
    for (i, first) in inputs.iter().enumerate() {
        if let Some(second) = inputs[i + 1..].iter().find(|other| other.plane.filter.eq_ignore_ascii_case(&first.plane.filter)) {
            return Err(CubeError::DuplicateFilter { filter: second.plane.filter.clone(), first: first.label.clone(), second: second.label.clone() });
        }
    }
    Ok(())
}

fn plane_order(inputs: &[CubeInput], order: PlaneOrder) -> Result<Vec<usize>, CubeError> {
    let mut indices: Vec<usize> = (0..inputs.len()).collect();
    if order == PlaneOrder::Wavelength {
        if let Some(missing) = inputs.iter().find(|input| input.plane.pivot_um.is_none()) {
            return Err(CubeError::NoPivot { label: missing.label.clone(), filter: missing.plane.filter.clone() });
        }
        let pivot = |i: usize| inputs[i].plane.pivot_um.unwrap_or(f64::INFINITY);
        indices.sort_by(|&a, &b| pivot(a).total_cmp(&pivot(b)));
    }
    Ok(indices)
}

fn check_noise(inputs: &[CubeInput], noise: &[InputNoise]) -> Result<(), CubeError> {
    if noise.len() != inputs.len() {
        return Err(CubeError::CompanionMismatch(NOISCORR_EXTNAME, noise.len(), inputs.len()));
    }
    for (input, noise) in inputs.iter().zip(noise) {
        let (rows, cols) = noise.correlation.dim();
        if rows != cols || rows % 2 == 0 {
            return Err(CubeError::NoiseMap { label: input.label.clone(), rows, cols });
        }
    }
    Ok(())
}

pub fn assemble_cube(inputs: Vec<CubeInput>, reference: usize, base: HduHeader, options: CubeOptions) -> Result<AssembledCube, CubeError> {
    assemble(inputs, None, reference, base, options)
}

pub fn assemble_cube_with_noise(inputs: Vec<CubeInput>, noise: Vec<InputNoise>, reference: usize, base: HduHeader, options: CubeOptions) -> Result<AssembledCube, CubeError> {
    check_noise(&inputs, &noise)?;
    assemble(inputs, Some(noise), reference, base, options)
}

fn assemble(inputs: Vec<CubeInput>, noise: Option<Vec<InputNoise>>, reference: usize, base: HduHeader, options: CubeOptions) -> Result<AssembledCube, CubeError> {
    let anchor = inputs.get(reference).ok_or(CubeError::Empty)?;
    check_inputs(&inputs, anchor)?;
    let dims = anchor.sci.dim();
    let bunit = classify_unit(&anchor.header).bunit.unwrap_or_else(|| UNKNOWN_UNIT.to_string());
    let order = plane_order(&inputs, options.order)?;
    let noise = noise.filter(|_| options.layout == CubeLayout::Mef).map(|noise| CubeNoise {
        planes: order
            .iter()
            .map(|&i| PlaneNoise {
                filter: inputs[i].plane.filter.clone(),
                correlation: noise[i].correlation.clone(),
                noiscorr_align: noise[i].noiscorr_align,
                noiscorr_match: UNMATCHED_NOISCORR_MATCH,
            })
            .collect(),
        source: NoiseSource::Align,
    });
    let mut inputs = inputs;
    let mut warnings = Vec::new();
    let any_err = inputs.iter().any(|input| input.err.is_some());
    let any_dq = inputs.iter().any(|input| input.dq.is_some());
    let (err, dq) = match options.layout {
        CubeLayout::Lightstack => {
            let carried: Vec<&str> = [(any_err, "ERR"), (any_dq, "DQ")].into_iter().filter(|(any, _)| *any).map(|(_, name)| name).collect();
            if !carried.is_empty() {
                warnings.push(format!(
                    "the lightstack layout holds the science planes only: the {} planes of the inputs are not written; the mef layout keeps them",
                    carried.join(" and ")
                ));
            }
            (None, None)
        }
        CubeLayout::Mef => {
            let mut err_planes = Vec::with_capacity(order.len());
            let mut dq_planes = Vec::with_capacity(order.len());
            let signed = inputs.iter().filter_map(|input| input.dq.as_ref()).all(|dq| dq.signed);
            for &i in &order {
                let input = &mut inputs[i];
                if any_err {
                    err_planes.push(input.err.take().unwrap_or_else(|| {
                        warnings.push(format!("{} has no ERR plane: its ERR plane in the cube is NaN", input.label));
                        Array2::from_elem(dims, f32::NAN)
                    }));
                }
                if any_dq {
                    dq_planes.push(input.dq.take().map(|dq| dq.bits).unwrap_or_else(|| {
                        warnings.push(format!("{} has no DQ plane: its DQ plane in the cube is zero", input.label));
                        Array2::zeros(dims)
                    }));
                }
            }
            (any_err.then_some(err_planes), any_dq.then_some(DqPlanes { planes: dq_planes, signed }))
        }
    };
    let filters: Vec<CubePlane> = order.iter().map(|&i| inputs[i].plane.clone()).collect();
    let planes: Vec<Array2<f32>> = order.iter().map(|&i| std::mem::take(&mut inputs[i].sci)).collect();
    let header = cube_header(base, options.layout, &bunit, &filters, dims, &mut warnings);
    Ok(AssembledCube { cube: Cube { header, planes, err, dq, filters }, order, bunit, warnings, noise })
}

pub fn without_hdu_identity(header: &HduHeader) -> HduHeader {
    let mut out = header.clone();
    remove_keys(&mut out, |key| HDU_IDENTITY_KEYS.contains(&key.trim()));
    out
}

pub fn planes_from_header(header: &HduHeader, naxis3: usize, registry: &FilterRegistry) -> Result<Vec<CubePlane>, CubeError> {
    (1..=naxis3)
        .map(|index| {
            let filter = plane_text(header, "FILTER", index).ok_or(CubeError::UnlabelledPlane { index })?;
            let dataset = plane_text(header, "DATASET", index).unwrap_or_default();
            let entry = registry.entry(&dataset, &filter);
            Ok(CubePlane::named(filter, dataset, entry))
        })
        .collect()
}

struct NoiseColumns {
    align: Vec<f64>,
    matched: Vec<f64>,
}

pub struct FiltersTable {
    filter: Vec<String>,
    dataset: Vec<String>,
    instrument: Vec<String>,
    pivot_um: Vec<f64>,
    pixscale: Vec<f64>,
    bunit_in: Vec<String>,
    fluxconv: Vec<f64>,
    psf_fwhm: Vec<f64>,
    kernel: Vec<String>,
    noise: Option<NoiseColumns>,
    status: Option<Vec<String>>,
}

impl FiltersTable {
    pub fn new(planes: &[CubePlane]) -> Self {
        let text = |field: fn(&CubePlane) -> &String| planes.iter().map(|p| field(p).clone()).collect();
        let number = |field: fn(&CubePlane) -> Option<f64>| planes.iter().map(|p| field(p).unwrap_or(f64::NAN)).collect();
        Self {
            filter: text(|p| &p.filter),
            dataset: text(|p| &p.dataset),
            instrument: text(|p| &p.instrument),
            pivot_um: number(|p| p.pivot_um),
            pixscale: number(|p| p.pixscale_arcsec),
            bunit_in: text(|p| &p.bunit_in),
            fluxconv: number(|p| p.fluxconv),
            psf_fwhm: number(|p| p.psf_fwhm_arcsec),
            kernel: text(|p| &p.kernel),
            noise: None,
            status: None,
        }
    }

    pub fn with_noise(self, noise: &CubeNoise) -> Result<Self, CubeError> {
        if noise.planes.len() != self.filter.len() {
            return Err(CubeError::CompanionMismatch(NOISCORR_EXTNAME, noise.planes.len(), self.filter.len()));
        }
        if let Some(filter) = self.filter.iter().zip(&noise.planes).find(|(filter, plane)| plane.filter != **filter).map(|(filter, _)| filter) {
            return Err(CubeError::NoiseFilter(filter.clone()));
        }
        let columns = NoiseColumns {
            align: noise.planes.iter().map(|plane| plane.noiscorr_align).collect(),
            matched: noise.planes.iter().map(|plane| plane.noiscorr_match).collect(),
        };
        Ok(Self { noise: Some(columns), ..self })
    }

    pub fn with_match(self, statuses: &[MatchStatus]) -> Result<Self, CubeError> {
        if statuses.len() != self.filter.len() {
            return Err(CubeError::CompanionMismatch(COLUMN_MATCH, statuses.len(), self.filter.len()));
        }
        Ok(Self { status: Some(statuses.iter().map(|status| status.label().to_string()).collect()), ..self })
    }

    pub fn columns(&self) -> Vec<TableColumn<'_>> {
        fn text<'a>(name: &'a str, values: &'a [String]) -> TableColumn<'a> {
            TableColumn { name, unit: None, values: TableValues::Text(values) }
        }
        fn number<'a>(name: &'a str, unit: Option<&'a str>, values: &'a [f64]) -> TableColumn<'a> {
            TableColumn { name, unit, values: TableValues::F64(values) }
        }
        let mut columns = vec![
            text(COLUMN_FILTER, &self.filter),
            text(COLUMN_DATASET, &self.dataset),
            text(COLUMN_INSTRUMENT, &self.instrument),
            number(COLUMN_PIVOT, Some(UNIT_MICRON), &self.pivot_um),
            number(COLUMN_PIXSCALE, Some(UNIT_ARCSEC), &self.pixscale),
            text(COLUMN_BUNIT_IN, &self.bunit_in),
            number(COLUMN_FLUXCONV, None, &self.fluxconv),
            number(COLUMN_PSF_FWHM, Some(UNIT_ARCSEC), &self.psf_fwhm),
            text(COLUMN_KERNEL, &self.kernel),
        ];
        if let Some(noise) = &self.noise {
            columns.push(number(COLUMN_NOISCORR_ALIGN, None, &noise.align));
            columns.push(number(COLUMN_NOISCORR_MATCH, None, &noise.matched));
        }
        if let Some(status) = &self.status {
            columns.push(text(COLUMN_MATCH, status));
        }
        columns
    }
}

fn finite_column(table: &BintableData, name: &str) -> anyhow::Result<Vec<Option<f64>>> {
    let values = match table.flattened_column(name)?.values {
        ColumnValues::F64(values) => values,
        ColumnValues::I64(values) => values.into_iter().map(|v| v as f64).collect(),
    };
    Ok(values.into_iter().map(|v| v.is_finite().then_some(v)).collect())
}

pub fn planes_from_table(table: &BintableData) -> anyhow::Result<Vec<CubePlane>> {
    let filter = table.text_column(COLUMN_FILTER)?;
    let rows = filter.len();
    let text = |name| table.text_column(name).unwrap_or_else(|_| vec![String::new(); rows]);
    let number = |name| finite_column(table, name).unwrap_or_else(|_| vec![None; rows]);
    let (dataset, instrument, bunit_in, kernel) = (text(COLUMN_DATASET), text(COLUMN_INSTRUMENT), text(COLUMN_BUNIT_IN), text(COLUMN_KERNEL));
    let (pivot, pixscale, fluxconv, psf_fwhm) = (number(COLUMN_PIVOT), number(COLUMN_PIXSCALE), number(COLUMN_FLUXCONV), number(COLUMN_PSF_FWHM));
    Ok((0..rows)
        .map(|i| CubePlane {
            filter: filter[i].clone(),
            dataset: dataset[i].clone(),
            instrument: instrument[i].clone(),
            pivot_um: pivot[i],
            pixscale_arcsec: pixscale[i],
            bunit_in: bunit_in[i].clone(),
            fluxconv: fluxconv[i],
            psf_fwhm_arcsec: psf_fwhm[i],
            kernel: kernel[i].clone(),
        })
        .collect())
}

pub fn dq_table_of(plane: &CubePlane) -> DqTable {
    match plane.dataset.trim().to_ascii_uppercase().as_str() {
        "JWST" => DqTable::Jwst,
        "HST" => DqTable::Hst,
        "ROMAN" => DqTable::Roman,
        _ => DqTable::Unknown,
    }
}

fn validity_mask(table: DqTable) -> u32 {
    match table.exclusion_mask() {
        0 => DQ_DO_NOT_USE,
        mask => mask,
    }
}

fn validity_note(plane: &CubePlane) -> String {
    let table = dq_table_of(plane);
    let mask = validity_mask(table);
    let named: Vec<&str> = table.flags().iter().filter(|flag| flag.bit & mask != 0).map(|flag| flag.name).collect();
    let (name, names) = match (table, named.first(), named.last()) {
        (DqTable::Unknown, _, _) => ("unknown", "DO_NOT_USE assumed".to_string()),
        (_, Some(first), Some(last)) if named.len() > 1 => (table.label(), format!("{first}..{last}")),
        (_, Some(first), _) => (table.label(), first.to_string()),
        _ => (table.label(), String::new()),
    };
    format!("{} ({name}): mask 0x{mask:X} ({names})", plane.filter)
}

pub fn plane_is_valid(plane: &Array2<f32>, dq: Option<(&Array2<u32>, u32)>, threshold: f64, frac_valid: f64) -> bool {
    let usable = |index: (usize, usize), value: f32| value.is_finite() && dq.is_none_or(|(bits, mask)| bits[index] & mask == 0);
    let (total, above) = plane
        .indexed_iter()
        .filter(|&(index, &value)| usable(index, value))
        .fold((0usize, 0usize), |(total, above), (_, &value)| (total + 1, above + usize::from(f64::from(value) > threshold)));
    total > 0 && above as f64 / total as f64 >= frac_valid
}

fn select_planes(cube: &Cube, keep: &[usize]) -> Cube {
    let pick = |planes: &Vec<Array2<f32>>| keep.iter().map(|&z| planes[z].clone()).collect::<Vec<_>>();
    let filters: Vec<CubePlane> = keep.iter().map(|&z| cube.filters[z].clone()).collect();
    let mut header = cube.header.clone();
    write_plane_keys(&mut header, &filters);
    Cube {
        header,
        planes: pick(&cube.planes),
        err: cube.err.as_ref().map(pick),
        dq: cube.dq.as_ref().map(|dq| DqPlanes { planes: keep.iter().map(|&z| dq.planes[z].clone()).collect(), signed: dq.signed }),
        filters,
    }
}

pub fn drop_invalid_planes(cube: &Cube, threshold: f64, frac_valid: f64, use_dq: bool) -> Result<(Cube, Vec<String>), CubeError> {
    let dq = cube.dq.as_ref().filter(|_| use_dq);
    let (keep, dropped): (Vec<usize>, Vec<usize>) = (0..cube.planes.len()).partition(|&z| {
        let flagged = dq.map(|dq| (&dq.planes[z], validity_mask(dq_table_of(&cube.filters[z]))));
        plane_is_valid(&cube.planes[z], flagged, threshold, frac_valid)
    });
    if keep.is_empty() {
        return Err(CubeError::NoValidPlanes { threshold, frac_valid });
    }
    let mut valid = select_planes(cube, &keep);
    let dq_note = match dq {
        Some(_) => format!("; DQ-flagged pixels do not count: {}", cube.filters.iter().map(validity_note).collect::<Vec<_>>().join(", ")),
        None => String::new(),
    };
    push_history(&mut valid.header, &format!("Removed invalid filters (threshold={threshold:?}, frac_valid={frac_valid:?}){dq_note}"));
    Ok((valid, dropped.iter().map(|&z| cube.filters[z].filter.clone()).collect()))
}

pub fn remove_filter(cube: &Cube, filter: &str) -> Result<Cube, CubeError> {
    let wanted = filter.trim();
    let (dropped, keep): (Vec<usize>, Vec<usize>) = (0..cube.filters.len()).partition(|&z| cube.filters[z].filter.eq_ignore_ascii_case(wanted));
    let Some(&first) = dropped.first() else {
        return Err(CubeError::FilterNotFound { filter: wanted.to_string(), filters: cube.filters.iter().map(|p| p.filter.clone()).collect() });
    };
    if keep.is_empty() {
        return Err(CubeError::WouldBeEmpty(wanted.to_string()));
    }
    let mut kept = select_planes(cube, &keep);
    push_history(&mut kept.header, &format!("Removed filter {}", cube.filters[first].filter));
    Ok(kept)
}

pub fn subcube(cube: &Cube, x_start: i64, x_end: i64, y_start: i64, y_end: i64) -> Result<Cube, CubeError> {
    let (rows, cols) = cube.dims();
    let inside = 0 <= x_start && x_start < x_end && x_end <= cols as i64 && 0 <= y_start && y_start < y_end && y_end <= rows as i64;
    if !inside {
        return Err(CubeError::BoxOutside { x_start, x_end, y_start, y_end, cols, rows });
    }
    let (x0, x1, y0, y1) = (x_start as usize, x_end as usize, y_start as usize, y_end as usize);
    let cut_f32 = |plane: &Array2<f32>| plane.slice(s![y0..y1, x0..x1]).to_owned();
    let cut_u32 = |plane: &Array2<u32>| plane.slice(s![y0..y1, x0..x1]).to_owned();
    let rect = CutoutRect { x0: x_start, y0: y_start, width: x1 - x0, height: y1 - y0 };
    let mut header = shift_header(&cube.header, &rect);
    for (key, value) in PIXEL_BOX_KEYS.iter().zip([x_start, x_end, y_start, y_end]) {
        header.set(key, value.to_string());
    }
    push_history(&mut header, &format!("Cut x {x_start}:{x_end}, y {y_start}:{y_end} (0-based pixel indices, end excluded)"));
    Ok(Cube {
        header,
        planes: cube.planes.iter().map(cut_f32).collect(),
        err: cube.err.as_ref().map(|planes| planes.iter().map(cut_f32).collect()),
        dq: cube.dq.as_ref().map(|dq| DqPlanes { planes: dq.planes.iter().map(cut_u32).collect(), signed: dq.signed }),
        filters: cube.filters.clone(),
    })
}

pub fn brightest_pixel(planes: &[Array2<f32>]) -> Option<(usize, usize)> {
    let (rows, cols) = planes.first()?.dim();
    let mut best: Option<(f64, usize, usize)> = None;
    for y in 0..rows {
        for x in 0..cols {
            let total: f64 = planes.iter().map(|plane| plane[[y, x]]).filter(|value| !value.is_nan()).map(f64::from).sum();
            if !total.is_nan() && best.is_none_or(|(highest, _, _)| total > highest) {
                best = Some((total, x, y));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

pub fn update_metadata(cube: &mut Cube, redshift: Option<f64>, centre: Option<(f64, f64)>, use_brightest_pixel: bool) -> Result<CubeCentre, CubeError> {
    let found = if use_brightest_pixel {
        let wcs = cube.wcs()?;
        let (x, y) = brightest_pixel(&cube.planes).ok_or(CubeError::Empty)?;
        let sky = wcs.pixel_to_world(x as f64, y as f64);
        CubeCentre { x: Some(x), y: Some(y), ra: sky.ra, dec: sky.dec, kind: "BRIGHTEST" }
    } else if let Some((ra, dec)) = centre {
        CubeCentre { x: None, y: None, ra, dec, kind: "USER" }
    } else {
        CubeCentre { x: None, y: None, ra: f64::NAN, dec: f64::NAN, kind: "UNKNOWN" }
    };
    let header = &mut cube.header;
    let redshift = redshift.filter(|z| z.is_finite());
    match redshift {
        Some(z) => header.set_f64("REDSHIFT", z),
        None => set_text(header, "REDSHIFT", "UNKNOWN"),
    }
    remove_keys(header, |key| CENTRE_KEYS.contains(&key));
    if let (Some(x), Some(y)) = (found.x, found.y) {
        header.set("GAL_XCEN", x.to_string());
        header.set("GAL_YCEN", y.to_string());
    }
    if found.kind != "UNKNOWN" {
        header.set_f64("GAL_RA", found.ra);
        header.set_f64("GAL_DEC", found.dec);
    }
    set_text(header, "CEN_TYPE", found.kind);
    let redshift_text = redshift.map_or_else(|| "UNKNOWN".to_string(), |z| z.to_string());
    let centre_text = match found.kind {
        "UNKNOWN" => "unknown".to_string(),
        kind => format!("{kind} RA {:.8} Dec {:.8}", found.ra, found.dec),
    };
    push_history(header, &format!("Cube metadata: REDSHIFT {redshift_text}, centre {centre_text}"));
    Ok(found)
}

pub fn apply_nan_mask(cube: &mut Cube, mode: MaskMode, reference_slice: usize, dq_flags: &[u32]) -> Result<usize, CubeError> {
    let depth = cube.planes.len();
    if depth == 0 {
        return Err(CubeError::Empty);
    }
    let dims = cube.dims();
    let (mask, note) = match mode {
        MaskMode::Reference => {
            let reference = cube.planes.get(reference_slice).ok_or(CubeError::SliceOutside { index: reference_slice, last: depth - 1 })?;
            if !reference.iter().any(|value| value.is_finite()) {
                return Err(CubeError::EmptyReference(reference_slice));
            }
            let filter = &cube.filters[reference_slice].filter;
            (reference.mapv(|value| !value.is_finite()), format!("Applied the NaN mask of slice {reference_slice} ({filter}) to every slice"))
        }
        MaskMode::Union => {
            let mask = Array2::from_shape_fn(dims, |p| cube.planes.iter().any(|plane| !plane[p].is_finite()));
            if mask.iter().all(|&masked| masked) {
                return Err(CubeError::EmptyMask("union"));
            }
            (mask, "Masked in every slice the pixels that are NaN in any slice (union of the NaN masks)".to_string())
        }
        MaskMode::Intersection => (
            Array2::from_shape_fn(dims, |p| cube.planes.iter().all(|plane| !plane[p].is_finite())),
            "Masked in every slice the pixels that are NaN in every slice (intersection of the NaN masks)".to_string(),
        ),
    };
    let mut masked = 0;
    for plane in &mut cube.planes {
        for (value, &hit) in plane.iter_mut().zip(mask.iter()) {
            if hit && !value.is_nan() {
                *value = f32::NAN;
                masked += 1;
            }
        }
    }
    for plane in cube.err.iter_mut().flatten() {
        plane.zip_mut_with(&mask, |value, &hit| {
            if hit {
                *value = f32::NAN;
            }
        });
    }
    if let Some(dq) = cube.dq.as_mut() {
        for (z, plane) in dq.planes.iter_mut().enumerate() {
            let flag = dq_flags.get(z).copied().unwrap_or(DQ_DO_NOT_USE);
            plane.zip_mut_with(&mask, |bits, &hit| {
                if hit {
                    *bits |= flag;
                }
            });
        }
    }
    push_history(&mut cube.header, &note);
    Ok(masked)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::core::metadata::filter_wavelengths::FilterRegistry;

    pub(crate) const ARCSEC: f64 = 1.0 / 3600.0;

    pub(crate) fn header_of(cards: &[(&str, String)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.clone());
        }
        header
    }

    pub(crate) fn cd_grid(dims: (usize, usize), scale_arcsec: f64, rotation_deg: f64) -> Vec<(&'static str, String)> {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let s = scale_arcsec * ARCSEC;
        vec![
            ("NAXIS1", dims.1.to_string()),
            ("NAXIS2", dims.0.to_string()),
            ("CTYPE1", "RA---TAN".into()),
            ("CTYPE2", "DEC--TAN".into()),
            ("CUNIT1", "deg".into()),
            ("CUNIT2", "deg".into()),
            ("CRPIX1", format!("{:E}", dims.1 as f64 / 2.0 + 0.5)),
            ("CRPIX2", format!("{:E}", dims.0 as f64 / 2.0 + 0.5)),
            ("CRVAL1", "53.1625".into()),
            ("CRVAL2", "-27.7914".into()),
            ("CD1_1", format!("{:E}", -s * cos)),
            ("CD1_2", format!("{:E}", s * sin)),
            ("CD2_1", format!("{:E}", s * sin)),
            ("CD2_2", format!("{:E}", s * cos)),
            ("RADESYS", "ICRS".into()),
        ]
    }

    pub(crate) fn pc_grid(dims: (usize, usize), scale_arcsec: f64, rotation_deg: f64) -> Vec<(&'static str, String)> {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let s = scale_arcsec * ARCSEC;
        vec![
            ("NAXIS1", dims.1.to_string()),
            ("NAXIS2", dims.0.to_string()),
            ("CTYPE1", "RA---TAN".into()),
            ("CTYPE2", "DEC--TAN".into()),
            ("CRPIX1", format!("{:E}", dims.1 as f64 / 2.0 + 0.5)),
            ("CRPIX2", format!("{:E}", dims.0 as f64 / 2.0 + 0.5)),
            ("CRVAL1", "53.1625".into()),
            ("CRVAL2", "-27.7914".into()),
            ("CDELT1", format!("{:E}", -s)),
            ("CDELT2", format!("{:E}", s)),
            ("PC1_1", format!("{:E}", cos)),
            ("PC1_2", format!("{:E}", -sin)),
            ("PC2_1", format!("{:E}", sin)),
            ("PC2_2", format!("{:E}", cos)),
        ]
    }

    pub(crate) fn plane_input(
        registry: &FilterRegistry,
        dataset: &str,
        filter: &str,
        grid: &[(&'static str, String)],
        extra: &[(&'static str, &str)],
        value: impl Fn(usize, usize) -> f32,
    ) -> CubeInput {
        let mut cards = grid.to_vec();
        cards.extend(extra.iter().map(|(key, value)| (*key, value.to_string())));
        let header = header_of(&cards);
        let wcs = header_wcs(&header).unwrap();
        let rows = header.get_i64("NAXIS2").unwrap() as usize;
        let cols = header.get_i64("NAXIS1").unwrap() as usize;
        let entry = registry.entry(dataset, filter).unwrap().clone();
        let plane = CubePlane::from_entry(&entry, &header, &wcs);
        CubeInput {
            label: format!("{dataset}_{filter}"),
            sci: Array2::from_shape_fn((rows, cols), |(y, x)| value(y, x)),
            err: None,
            dq: None,
            header,
            wcs,
            plane,
        }
    }

    pub(crate) fn small_cube(filters: &[(&str, &str)], dims: (usize, usize), value: impl Fn(usize, usize, usize) -> f32) -> Cube {
        let registry = FilterRegistry::builtin();
        let grid = cd_grid(dims, 0.04, 0.0);
        let inputs: Vec<CubeInput> = filters
            .iter()
            .enumerate()
            .map(|(z, (dataset, filter))| plane_input(&registry, dataset, filter, &grid, &[("BUNIT", "Jy")], |y, x| value(z, y, x)))
            .collect();
        let base = cube_base_header(&inputs[0].header);
        assemble_cube(inputs, 0, base, CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Given }).unwrap().cube
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::infra::fits::table::{find_bintable, load_bintable};
    use crate::infra::fits::writer::{write_mef_with_tables, BinTableHdu, HduData, ImageHdu};

    fn text(header: &HduHeader, key: &str) -> Option<String> {
        card_text(header, key)
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.clone()).collect()
    }

    #[test]
    fn lightstack_layout_keywords_match_build_datacube() {
        let registry = FilterRegistry::builtin();
        let dims = (6, 8);
        for (form, mut grid) in [("cd", cd_grid(dims, 0.04, 20.0)), ("pc", pc_grid(dims, 0.04, 20.0))] {
            if form == "cd" {
                grid.extend([("A_ORDER", "2".to_string()), ("B_ORDER", "2".to_string()), ("A_2_0", "1E-6".to_string()), ("B_0_2", "-1E-6".to_string())]);
                for card in grid.iter_mut().filter(|(key, _)| key.starts_with("CTYPE")) {
                    card.1.push_str("-SIP");
                }
            }
            let inputs = vec![
                plane_input(&registry, "JWST", "F444W", &grid, &[("BUNIT", "Jy"), ("FILTER", "F444W")], |y, x| (y * 8 + x) as f32),
                plane_input(&registry, "HST", "F814W", &grid, &[("BUNIT", "Jy")], |_, _| 2.0),
                plane_input(&registry, "JWST", "F150W", &grid, &[("BUNIT", "Jy")], |_, _| 3.0),
            ];
            let reference = inputs[0].header.clone();
            let base = cube_base_header(&reference);
            let assembled = assemble_cube(inputs, 0, base, CubeOptions { layout: CubeLayout::Lightstack, order: PlaneOrder::Wavelength }).unwrap();
            let header = &assembled.cube.header;
            assert_eq!(assembled.order, [1, 2, 0], "{form}: planes follow the pivot wavelength");
            assert_eq!(assembled.cube.planes[2][[1, 1]], 9.0);
            assert_eq!(assembled.bunit, "Jy");

            assert_eq!(text(header, "CTYPE1").as_deref(), Some("RA---TAN"), "{form}");
            assert_eq!(text(header, "CTYPE2").as_deref(), Some("DEC--TAN"), "{form}");
            for key in ["CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2"] {
                assert_eq!(header.get_f64(key), reference.get_f64(key), "{form} {key}");
            }
            let matrix: &[&str] = if form == "cd" { &["CD1_1", "CD1_2", "CD2_1", "CD2_2"] } else { &["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2"] };
            for key in matrix {
                assert_eq!(header.get_f64(key), reference.get_f64(key), "{form} {key}");
            }
            assert_eq!(header.get_i64("WCSAXES"), Some(3));
            assert_eq!(header.cards[0].0, "WCSAXES", "WCSAXES precedes the other WCS keywords");
            assert_eq!(text(header, "CTYPE3").as_deref(), Some("FILTER"));
            assert_eq!((header.get_f64("CRPIX3"), header.get_f64("CRVAL3"), header.get_f64("CDELT3")), (Some(1.0), Some(0.0), Some(1.0)));
            assert_eq!(header.get_f64("CD3_3"), (form == "cd").then_some(1.0), "{form}");
            assert!(header.get("CUNIT3").is_none());
            assert_eq!(text(header, "BUNIT").as_deref(), Some("Jy"));
            for (i, (filter, dataset)) in [("F814W", "HST"), ("F150W", "JWST"), ("F444W", "JWST")].iter().enumerate() {
                assert_eq!(text(header, &format!("FILTER{}", i + 1)).as_deref(), Some(*filter));
                assert_eq!(text(header, &format!("DATASET{}", i + 1)).as_deref(), Some(*dataset));
            }
            assert_eq!(header.get_i64("NFILTERS"), Some(3));
            assert_eq!(header.get_i64("NAXIS3"), Some(3));
            assert_eq!(text(header, "FILTERS").as_deref(), Some("F814W,F150W,F444W"));
            assert_eq!(text(header, "DATASETS").as_deref(), Some("HST,JWST,JWST"));
            assert!(header.get("FILTER").is_none(), "per-plane cards of the reference stay out of the cube header");
            assert!(header.cards.iter().all(|(key, _)| !key.starts_with("A_") && !key.starts_with("B_")), "{form}: no SIP card");
            assert_eq!(assembled.warnings.iter().any(|w| w.contains("SIP")), form == "cd", "{:?}", assembled.warnings);
            assert!(history(header).join(" ").contains("doi:10.5281/zenodo.19393438"));
            assert!(assembled.cube.err.is_none() && assembled.cube.dq.is_none());
        }

        let mixed = vec![
            plane_input(&registry, "JWST", "F444W", &cd_grid(dims, 0.04, 0.0), &[("BUNIT", "MJy/sr")], |_, _| 1.0),
            plane_input(&registry, "JWST", "F150W", &cd_grid(dims, 0.04, 0.0), &[("BUNIT", "Jy")], |_, _| 1.0),
        ];
        let base = cube_base_header(&mixed[0].header);
        let refused = assemble_cube(mixed, 0, base, CubeOptions::default()).unwrap_err();
        assert!(matches!(refused, CubeError::MixedUnits { .. }), "{refused}");

        let shifted = vec![
            plane_input(&registry, "JWST", "F444W", &cd_grid(dims, 0.04, 0.0), &[("BUNIT", "Jy")], |_, _| 1.0),
            plane_input(&registry, "JWST", "F150W", &cd_grid(dims, 0.04, 1.0), &[("BUNIT", "Jy")], |_, _| 1.0),
        ];
        let base = cube_base_header(&shifted[0].header);
        let refused = assemble_cube(shifted, 0, base, CubeOptions::default()).unwrap_err();
        assert_eq!(refused, CubeError::OffGrid { label: "JWST_F150W".into(), reference: "JWST_F444W".into() });

        let twice = vec![
            plane_input(&registry, "JWST", "F444W", &cd_grid(dims, 0.04, 0.0), &[("BUNIT", "Jy")], |_, _| 1.0),
            plane_input(&registry, "JWST", "F444W", &cd_grid(dims, 0.04, 0.0), &[("BUNIT", "Jy")], |_, _| 1.0),
        ];
        let base = cube_base_header(&twice[0].header);
        assert!(matches!(assemble_cube(twice, 0, base, CubeOptions::default()).unwrap_err(), CubeError::DuplicateFilter { .. }));
    }

    #[test]
    fn calibrated_counts_build_a_cube_only_when_they_share_one_calibration() {
        let registry = FilterRegistry::builtin();
        let grid = cd_grid((4, 5), 0.04, 0.0);
        let counts = |filter: &str, cards: &[(&'static str, &str)]| {
            let extra: Vec<(&'static str, &str)> = [("BUNIT", "ELECTRONS/S")].into_iter().chain(cards.iter().copied()).collect();
            plane_input(&registry, "HST", filter, &grid, &extra, |_, _| 1.0)
        };
        let build = |inputs: Vec<CubeInput>| {
            let base = cube_base_header(&inputs[0].header);
            assemble_cube(inputs, 0, base, CubeOptions::default())
        };
        let refused = |cards: &str| CubeError::MixedCalibration {
            label: "HST_F606W".into(),
            unit: "ELECTRONS/S".into(),
            reference: "HST_F814W".into(),
            cards: cards.into(),
        };

        let photfnu = build(vec![counts("F814W", &[("PHOTFNU", "1.5E-7")]), counts("F606W", &[("PHOTFNU", "9.2E-8")])]).unwrap_err();
        assert_eq!(photfnu, refused("PHOTFNU"));
        assert!(photfnu.to_string().contains("Jy per pixel"), "{photfnu}");
        let pivot = build(vec![
            counts("F814W", &[("PHOTFLAM", "7.0E-20"), ("PHOTPLAM", "8045.53")]),
            counts("F606W", &[("PHOTFLAM", "7.0E-20"), ("PHOTPLAM", "5921.88")]),
        ]);
        assert_eq!(pivot.unwrap_err(), refused("PHOTPLAM"));
        let missing = build(vec![counts("F814W", &[("PHOTFNU", "1.5E-7"), ("MAGZERO", "25.0")]), counts("F606W", &[("PHOTFNU", "1.5E-7")])]);
        assert_eq!(missing.unwrap_err(), refused("MAGZERO"));

        let shared = build(vec![counts("F814W", &[("MAGZERO", "25.0")]), counts("F606W", &[("MAGZERO", "25.0")])]).unwrap();
        assert_eq!(shared.cube.header.get_f64("MAGZERO"), Some(25.0), "the shared calibration stays in the cube");
        assert_eq!(classify_unit(&shared.cube.header).class, UnitClass::PerPixelCalibrated { cards: vec!["MAGZERO".into()] });
        assert_eq!(shared.bunit, "ELECTRONS/S");
    }

    #[test]
    fn a_dataset_name_too_long_for_its_hierarch_card_is_reported() {
        let registry = FilterRegistry::builtin();
        let grid = cd_grid((2, 2), 0.04, 0.0);
        let ten = ["F070W", "F090W", "F115W", "F150W", "F200W", "F277W", "F356W", "F410M", "F444W", "F480M"];
        let build = |dataset: String| {
            let mut inputs: Vec<CubeInput> =
                ten.iter().map(|filter| plane_input(&registry, "JWST", filter, &grid, &[("BUNIT", "Jy")], |_, _| 1.0)).collect();
            inputs[9].plane.dataset = dataset;
            let base = cube_base_header(&inputs[0].header);
            assemble_cube(inputs, 0, base, CubeOptions { layout: CubeLayout::Lightstack, order: PlaneOrder::Given }).unwrap()
        };
        let fitting = build("x".repeat(58));
        assert!(fitting.warnings.iter().all(|w| !w.contains("DATASET10")), "{:?}", fitting.warnings);
        let dropped = build("x".repeat(59));
        assert!(
            dropped.warnings.iter().any(|w| w.contains("HIERARCH DATASET10") && w.contains("F480M")),
            "a dataset name the writer cannot fit in its HIERARCH card is reported: {:?}",
            dropped.warnings
        );
    }

    #[test]
    fn mef_assembly_keeps_err_and_dq_and_fills_missing_companions() {
        let registry = FilterRegistry::builtin();
        let dims = (4, 5);
        let grid = cd_grid(dims, 0.04, 0.0);
        let mut with_companions = plane_input(&registry, "JWST", "F444W", &grid, &[("BUNIT", "Jy")], |_, _| 4.0);
        with_companions.err = Some(Array2::from_elem(dims, 0.5));
        with_companions.dq = Some(IntPlane { bits: Array2::from_elem(dims, 1u32 << 31), signed: false });
        let bare = plane_input(&registry, "JWST", "F150W", &grid, &[("BUNIT", "Jy")], |_, _| 1.5);
        let base = cube_base_header(&with_companions.header);
        let assembled =
            assemble_cube(vec![with_companions, bare], 0, base, CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength }).unwrap();
        let cube = &assembled.cube;
        assert_eq!(cube.filters.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), ["F150W", "F444W"]);
        let err = cube.err.as_ref().unwrap();
        assert!(err[0].iter().all(|v| v.is_nan()), "a plane without ERR gets an unknown error");
        assert!(err[1].iter().all(|&v| v == 0.5));
        let dq = cube.dq.as_ref().unwrap();
        assert!(!dq.signed);
        assert!(dq.planes[0].iter().all(|&b| b == 0) && dq.planes[1].iter().all(|&b| b == 1u32 << 31));
        assert_eq!(assembled.warnings.len(), 2, "{:?}", assembled.warnings);
        assert!(assembled.warnings.iter().all(|w| w.contains("F150W")), "{:?}", assembled.warnings);
        assert!((cube.filters[1].pivot_um.unwrap() - 4.404315).abs() < 1e-12);
        assert_eq!(cube.filters[1].fluxconv, Some(1.0));
        assert_eq!(cube.filters[1].bunit_in, "Jy");
        assert!((cube.filters[1].pixscale_arcsec.unwrap() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn plane_labels_come_from_filter_keywords() {
        let registry = FilterRegistry::builtin();
        let filters = ["F070W", "F090W", "F115W", "F150W", "F200W", "F277W", "F356W", "F410M", "F444W", "F480M", "F335M"];
        let mut header = header_of(&[("NAXIS3", "11".to_string()), ("CTYPE3", "FILTER".to_string())]);
        for (i, filter) in filters.iter().enumerate() {
            let dataset_key = if i + 1 >= 10 { format!("HIERARCH DATASET{}", i + 1) } else { format!("DATASET{}", i + 1) };
            header.set(&format!("FILTER{}", i + 1), filter.to_string());
            header.set(&dataset_key, "JWST".to_string());
        }
        let planes = planes_from_header(&header, 11, &registry).unwrap();
        assert_eq!(planes.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), filters);
        assert!(planes.iter().all(|p| p.dataset == "JWST" && p.instrument == "NIRCam"), "{planes:?}");
        assert!((planes[10].pivot_um.unwrap() - 3.362067).abs() < 1e-12, "the HIERARCH DATASET11 key names the dataset");
        assert_eq!(planes[0].fluxconv, None, "a LIGHTSTACK cube does not record its conversion");

        let mut unknown = header.clone();
        unknown.set("FILTER2", "NOTAFILTER".into());
        let planes = planes_from_header(&unknown, 11, &registry).unwrap();
        assert_eq!((planes[1].filter.as_str(), planes[1].pivot_um), ("NOTAFILTER", None));

        let mut missing = header.clone();
        missing.remove("FILTER3");
        assert_eq!(planes_from_header(&missing, 11, &registry).unwrap_err(), CubeError::UnlabelledPlane { index: 3 });
    }

    fn cube_filters(cube: &Cube) -> Vec<&str> {
        cube.filters.iter().map(|p| p.filter.as_str()).collect()
    }

    fn same_bits(a: &[Array2<f32>], b: &[Array2<f32>]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.dim() == y.dim() && x.iter().zip(y.iter()).all(|(p, q)| p.to_bits() == q.to_bits()))
    }

    #[test]
    fn invalid_plane_rule_matches_lightstack() {
        let filters = [("JWST", "F070W"), ("JWST", "F090W"), ("JWST", "F115W"), ("JWST", "F150W"), ("JWST", "F200W"), ("JWST", "F277W")];
        let dims = (10, 20);
        let cube = small_cube(&filters, dims, |z, y, x| {
            let i = y * 20 + x;
            match z {
                0 => f32::NAN,
                1 => 0.0,
                2 => if i == 0 { 1.0 } else { -1.0 },
                3 => if i < 2 { 1.0 } else if i < 4 { f32::NAN } else { 0.0 },
                4 => if i < 2 { 1.0 } else { 0.0 },
                _ => 5.0,
            }
        });
        assert!(!plane_is_valid(&cube.planes[0], None, 0.0, DEFAULT_FRAC_VALID), "no finite pixel");
        assert!(!plane_is_valid(&cube.planes[1], None, 0.0, DEFAULT_FRAC_VALID), "nothing above the threshold");
        assert!(!plane_is_valid(&cube.planes[2], None, 0.0, DEFAULT_FRAC_VALID), "0.5 percent above");
        assert!(plane_is_valid(&cube.planes[3], None, 0.0, DEFAULT_FRAC_VALID), "2 of 198 finite pixels is above 1 percent");
        assert!(plane_is_valid(&cube.planes[4], None, 0.0, DEFAULT_FRAC_VALID), "exactly 1 percent is kept, as n_above / n_total >= frac_valid");
        assert!(!plane_is_valid(&cube.planes[4], None, 1.0, DEFAULT_FRAC_VALID), "strictly above the threshold");

        let (valid, removed) = drop_invalid_planes(&cube, 0.0, DEFAULT_FRAC_VALID, false).unwrap();
        assert_eq!(removed, ["F070W", "F090W", "F115W"]);
        assert_eq!(cube_filters(&valid), ["F150W", "F200W", "F277W"]);
        assert!(same_bits(&valid.planes, &cube.planes[3..]), "the kept planes are copied unchanged");
        assert_eq!(valid.header.get_i64("NFILTERS"), Some(3));
        assert_eq!(valid.header.get_i64("NAXIS3"), Some(3));
        assert_eq!(card_text(&valid.header, "FILTERS").as_deref(), Some("F150W,F200W,F277W"));
        assert_eq!(card_text(&valid.header, "FILTER1").as_deref(), Some("F150W"));
        assert!(valid.header.get("FILTER4").is_none() && valid.header.get("DATASET4").is_none());
        assert_eq!(valid.err.as_ref().map(Vec::len), None);
        assert!(history(&valid.header).contains(&"Removed invalid filters (threshold=0.0, frac_valid=0.01)".to_string()), "{:?}", history(&valid.header));

        let mut flagged = cube.clone();
        flagged.dq = Some(DqPlanes {
            planes: (0..6).map(|z| Array2::from_shape_fn(dims, |(y, x)| u32::from(z == 4 && y * 20 + x >= 100) * DQ_DO_NOT_USE)).collect(),
            signed: false,
        });
        assert!(plane_is_valid(&flagged.planes[4], Some((&flagged.dq.as_ref().unwrap().planes[4], DQ_DO_NOT_USE)), 0.0, 0.015), "2 of 100 unflagged pixels");
        let (dq_aware, removed) = drop_invalid_planes(&flagged, 0.0, 0.015, true).unwrap();
        assert_eq!(removed, ["F070W", "F090W", "F115W", "F150W"]);
        assert_eq!(cube_filters(&dq_aware), ["F200W", "F277W"]);
        assert_eq!(dq_aware.dq.as_ref().unwrap().planes.len(), 2);
        let (blind, _) = drop_invalid_planes(&flagged, 0.0, 0.015, false).unwrap();
        assert_eq!(cube_filters(&blind), ["F277W"], "without DQ the flagged half of F200W dilutes it below 1.5 percent");

        let empty = small_cube(&filters[..2], dims, |_, _, _| f32::NAN);
        assert_eq!(drop_invalid_planes(&empty, 0.0, DEFAULT_FRAC_VALID, false).unwrap_err(), CubeError::NoValidPlanes { threshold: 0.0, frac_valid: 0.01 });
    }

    #[test]
    fn remove_filter_updates_dataset_keys_too() {
        let filters = [("HST", "F814W"), ("JWST", "F150W"), ("JWST", "F200W"), ("JWST", "F277W"), ("JWST", "F356W"), ("JWST", "F444W")];
        let mut cube = small_cube(&filters, (3, 4), |z, _, _| z as f32);
        cube.err = Some((0..6).map(|z| Array2::from_elem((3, 4), z as f32 * 0.1)).collect());
        cube.dq = Some(DqPlanes { planes: (0..6).map(|z| Array2::from_elem((3, 4), z as u32)).collect(), signed: true });
        let removed = remove_filter(&cube, "f814w").unwrap();
        assert_eq!(cube_filters(&removed), ["F150W", "F200W", "F277W", "F356W", "F444W"]);
        assert_eq!(removed.planes, cube.planes[1..].to_vec());
        assert_eq!(removed.err.as_ref().unwrap(), &cube.err.as_ref().unwrap()[1..].to_vec());
        assert_eq!(removed.dq.as_ref().unwrap().planes, cube.dq.as_ref().unwrap().planes[1..].to_vec());
        assert!(removed.dq.as_ref().unwrap().signed);
        let header = &removed.header;
        for (i, filter) in ["F150W", "F200W", "F277W", "F356W", "F444W"].iter().enumerate() {
            assert_eq!(card_text(header, &format!("FILTER{}", i + 1)).as_deref(), Some(*filter));
            assert_eq!(card_text(header, &format!("DATASET{}", i + 1)).as_deref(), Some("JWST"), "LIGHTSTACK leaves DATASET{} stale", i + 1);
        }
        assert!(header.get("FILTER6").is_none() && header.get("DATASET6").is_none());
        assert_eq!(card_text(header, "DATASETS").as_deref(), Some("JWST,JWST,JWST,JWST,JWST"));
        assert_eq!(header.get_i64("NFILTERS"), Some(5));
        assert!(history(header).contains(&"Removed filter F814W".to_string()));

        assert!(matches!(remove_filter(&cube, "F090W").unwrap_err(), CubeError::FilterNotFound { .. }));
        let single = small_cube(&filters[..1], (3, 4), |_, _, _| 1.0);
        assert_eq!(remove_filter(&single, "F814W").unwrap_err(), CubeError::WouldBeEmpty("F814W".into()));

        let eleven = [
            ("JWST", "F070W"), ("JWST", "F090W"), ("JWST", "F115W"), ("JWST", "F150W"), ("JWST", "F200W"), ("JWST", "F277W"),
            ("JWST", "F356W"), ("JWST", "F410M"), ("JWST", "F444W"), ("JWST", "F480M"), ("HST", "F160W"),
        ];
        let big = small_cube(&eleven, (2, 2), |z, _, _| z as f32);
        assert_eq!(card_text(&big.header, "HIERARCH DATASET11").as_deref(), Some("HST"));
        assert_eq!(card_text(&big.header, "HIERARCH DATASET10").as_deref(), Some("JWST"));
        let smaller = remove_filter(&big, "F070W").unwrap();
        assert_eq!(card_text(&smaller.header, "HIERARCH DATASET10").as_deref(), Some("HST"));
        assert!(smaller.header.get("HIERARCH DATASET11").is_none());
        assert!(smaller.header.get("FILTER11").is_none());
    }

    #[test]
    fn subcube_slices_every_plane_and_shifts_the_grid() {
        let filters = [("JWST", "F150W"), ("JWST", "F444W")];
        let mut cube = small_cube(&filters, (6, 8), |z, y, x| (z * 100 + y * 10 + x) as f32);
        cube.err = Some(cube.planes.iter().map(|p| p * 0.1).collect());
        let cut = subcube(&cube, 2, 7, 1, 4).unwrap();
        assert_eq!(cut.dims(), (3, 5));
        assert_eq!(cut.planes[1], cube.planes[1].slice(s![1..4, 2..7]).to_owned());
        assert_eq!(cut.err.as_ref().unwrap()[0], cube.err.as_ref().unwrap()[0].slice(s![1..4, 2..7]).to_owned());
        assert_eq!(cut.header.get_f64("CRPIX1"), cube.header.get_f64("CRPIX1").map(|v| v - 2.0));
        assert_eq!(cut.header.get_f64("CRPIX2"), cube.header.get_f64("CRPIX2").map(|v| v - 1.0));
        assert_eq!((cut.header.get_i64("NAXIS1"), cut.header.get_i64("NAXIS2")), (Some(5), Some(3)));
        let bounds: Vec<Option<i64>> = PIXEL_BOX_KEYS.iter().map(|key| cut.header.get_i64(key)).collect();
        assert_eq!(bounds, [Some(2), Some(7), Some(1), Some(4)]);
        let before = cube.wcs().unwrap().pixel_to_world(2.0, 1.0);
        let after = cut.wcs().unwrap().pixel_to_world(0.0, 0.0);
        assert!((before.ra - after.ra).abs() < 1e-12 && (before.dec - after.dec).abs() < 1e-12);
        assert_eq!(cut.filters, cube.filters);
        assert!(matches!(subcube(&cube, 2, 9, 0, 3).unwrap_err(), CubeError::BoxOutside { .. }));
        assert!(matches!(subcube(&cube, 4, 4, 0, 3).unwrap_err(), CubeError::BoxOutside { .. }));
    }

    #[test]
    fn brightest_pixel_centre_matches_lightstack() {
        let filters = [("JWST", "F150W"), ("JWST", "F277W"), ("JWST", "F444W")];
        let mut cube = small_cube(&filters, (5, 7), |z, y, x| match (z, y, x) {
            (0, 3, 5) => f32::NAN,
            (1, 3, 5) => 10.0,
            (2, 3, 5) => 2.0,
            (0, 1, 2) => 6.0,
            (1, 1, 2) => 5.0,
            (2, 1, 2) => f32::NAN,
            _ => 0.5,
        });
        assert_eq!(brightest_pixel(&cube.planes), Some((5, 3)), "nansum skips NaN: 12 at x 5, y 3 beats 11 at x 2, y 1");
        let tie = small_cube(&filters, (5, 7), |z, y, x| if (y, x) == (1, 2) || (y, x) == (3, 5) { 1.0 + z as f32 } else { 0.0 });
        assert_eq!(brightest_pixel(&tie.planes), Some((2, 1)), "a tie goes to the first pixel in row order, as numpy argmax");
        let blank = small_cube(&filters, (5, 7), |_, _, _| f32::NAN);
        assert_eq!(brightest_pixel(&blank.planes), Some((0, 0)), "an all-NaN cube sums to zero everywhere");
        assert_eq!(brightest_pixel(&[]), None);
        let centre = update_metadata(&mut cube, Some(0.303), None, true).unwrap();
        assert_eq!((centre.x, centre.y, centre.kind), (Some(5), Some(3), "BRIGHTEST"));
        let (x, y) = (centre.x.unwrap(), centre.y.unwrap());
        let expected = cube.wcs().unwrap().pixel_to_world(x as f64, y as f64);
        assert_eq!(cube.header.get_i64("GAL_XCEN"), Some(x as i64));
        assert_eq!(cube.header.get_i64("GAL_YCEN"), Some(y as i64));
        assert!((cube.header.get_f64("GAL_RA").unwrap() - expected.ra).abs() < 1e-11);
        assert!((cube.header.get_f64("GAL_DEC").unwrap() - expected.dec).abs() < 1e-11);
        assert!((centre.ra - expected.ra).abs() < 1e-15 && (centre.dec - expected.dec).abs() < 1e-15);
        assert_eq!(card_text(&cube.header, "CEN_TYPE").as_deref(), Some("BRIGHTEST"));
        assert_eq!(cube.header.get_f64("REDSHIFT"), Some(0.303));

        let centre = update_metadata(&mut cube, None, Some((53.16, -27.79)), false).unwrap();
        assert_eq!((centre.kind, centre.x), ("USER", None));
        assert_eq!(card_text(&cube.header, "REDSHIFT").as_deref(), Some("UNKNOWN"));
        assert_eq!((cube.header.get_f64("GAL_RA"), cube.header.get_f64("GAL_DEC")), (Some(53.16), Some(-27.79)));
        assert!(cube.header.get("GAL_XCEN").is_none(), "a user centre has no brightest pixel");
        assert_eq!(card_text(&cube.header, "CEN_TYPE").as_deref(), Some("USER"));

        update_metadata(&mut cube, Some(f64::NAN), None, false).unwrap();
        assert_eq!(card_text(&cube.header, "CEN_TYPE").as_deref(), Some("UNKNOWN"));
        assert!(CENTRE_KEYS.iter().all(|key| cube.header.get(key).is_none()));
        assert_eq!(card_text(&cube.header, "REDSHIFT").as_deref(), Some("UNKNOWN"));
    }

    #[test]
    fn nan_masks_follow_a_reference_plane_or_combine_every_plane() {
        let filters = [("JWST", "F150W"), ("JWST", "F277W"), ("JWST", "F444W")];
        let nan_at = |z: usize, y: usize, x: usize| matches!((z, y, x), (0, 0, 0) | (0, 1, 1) | (1, 1, 1) | (2, 1, 1) | (2, 2, 3));
        let build = || {
            let mut cube = small_cube(&filters, (3, 4), |z, y, x| if nan_at(z, y, x) { f32::NAN } else { 1.0 + z as f32 });
            cube.err = Some(cube.planes.iter().map(|p| p.mapv(|v| if v.is_nan() { f32::NAN } else { 0.1 })).collect());
            cube.dq = Some(DqPlanes { planes: vec![Array2::from_elem((3, 4), 4u32); 3], signed: false });
            cube
        };
        let flags = [1, 1, 1 | 512];
        let nans = |cube: &Cube, z: usize| cube.planes[z].indexed_iter().filter(|(_, v)| v.is_nan()).map(|(p, _)| p).collect::<Vec<_>>();

        let mut reference = build();
        let masked = apply_nan_mask(&mut reference, MaskMode::Reference, 0, &flags).unwrap();
        assert_eq!(masked, 2, "pixel (0, 0) of planes 1 and 2 becomes NaN; (1, 1) is NaN already and (2, 3) of plane 2 stays");
        for z in 0..3 {
            let expected: Vec<(usize, usize)> = if z == 2 { vec![(0, 0), (1, 1), (2, 3)] } else { vec![(0, 0), (1, 1)] };
            assert_eq!(nans(&reference, z), expected, "plane {z}");
            assert!(reference.err.as_ref().unwrap()[z][[0, 0]].is_nan());
        }
        assert_eq!(reference.dq.as_ref().unwrap().planes[2][[0, 0]], 4 | 1 | 512);
        assert_eq!(reference.dq.as_ref().unwrap().planes[1][[0, 1]], 4);

        let mut union = build();
        apply_nan_mask(&mut union, MaskMode::Union, 0, &flags).unwrap();
        for z in 0..3 {
            assert_eq!(nans(&union, z), [(0, 0), (1, 1), (2, 3)], "plane {z}");
        }
        let mut intersection = build();
        assert_eq!(apply_nan_mask(&mut intersection, MaskMode::Intersection, 0, &flags).unwrap(), 0);
        assert_eq!(nans(&intersection, 0), [(0, 0), (1, 1)]);
        assert_eq!(intersection.dq.as_ref().unwrap().planes[0][[1, 1]], 4 | 1, "a pixel NaN in every plane is flagged");

        let mut outside = build();
        assert_eq!(apply_nan_mask(&mut outside, MaskMode::Reference, 3, &flags).unwrap_err(), CubeError::SliceOutside { index: 3, last: 2 });
        let mut blank = small_cube(&filters, (3, 4), |z, _, _| if z == 1 { f32::NAN } else { 1.0 });
        assert_eq!(apply_nan_mask(&mut blank, MaskMode::Reference, 1, &flags).unwrap_err(), CubeError::EmptyReference(1));
        assert_eq!(apply_nan_mask(&mut blank, MaskMode::Union, 0, &flags).unwrap_err(), CubeError::EmptyMask("union"));
    }

    #[test]
    fn the_filters_table_columns_carry_every_plane() {
        let filters = [("JWST", "F150W"), ("HST", "F814W")];
        let cube = small_cube(&filters, (2, 3), |_, _, _| 1.0);
        let table = FiltersTable::new(&cube.filters);
        let columns = table.columns();
        let names: Vec<&str> = columns.iter().map(|c| c.name).collect();
        assert_eq!(names, ["FILTER", "DATASET", "INSTRUME", "PIVOT_UM", "PIXSCALE", "BUNIT_IN", "FLUXCONV", "PSF_FWHM", "KERNEL"]);
        let TableValues::F64(psf) = columns[7].values else { panic!("PSF_FWHM is not numeric") };
        assert!(psf.iter().all(|v| v.is_nan()), "an unknown PSF FWHM is NaN in the table");
        let TableValues::Text(instrument) = columns[2].values else { panic!("INSTRUME is not text") };
        assert_eq!(instrument, ["NIRCam", "ACS"]);
        assert_eq!(columns[3].unit, Some("um"));
    }

    #[test]
    fn drop_invalid_uses_the_dataset_exclusion_mask_and_names_it_in_history() {
        use crate::core::imaging::dq_flags::DqTable;

        let filters = [("HST", "F606W"), ("HST", "F814W"), ("JWST", "F150W"), ("JWST", "F200W"), ("JWST", "F277W")];
        let dims = (10, 20);
        let bright = |y: usize, x: usize| y == 0 && x < 3;
        let mut cube = small_cube(&filters, dims, |_, y, x| f32::from(u8::from(bright(y, x))));
        cube.filters[4].dataset = "SUBARU".into();
        let flagged = |bits: u32| Array2::from_shape_fn(dims, |(y, x)| u32::from(bright(y, x)) * bits);
        cube.dq = Some(DqPlanes { planes: vec![flagged(4096), flagged(0), flagged(4096), flagged(1), flagged(4096)], signed: false });
        let dq = cube.dq.as_ref().unwrap();
        assert_eq!(dq_table_of(&cube.filters[0]), DqTable::Hst);
        assert_eq!(dq_table_of(&cube.filters[2]), DqTable::Jwst);
        assert_eq!(dq_table_of(&cube.filters[4]), DqTable::Unknown);
        assert!(plane_is_valid(&cube.planes[0], Some((&dq.planes[0], DqTable::Jwst.exclusion_mask())), 0.0, DEFAULT_FRAC_VALID), "bit 0 alone lets the COSMIC_RAY plane pass");
        assert!(!plane_is_valid(&cube.planes[0], Some((&dq.planes[0], DqTable::Hst.exclusion_mask())), 0.0, DEFAULT_FRAC_VALID), "the HST mask excludes its bright pixels");

        let (valid, removed) = drop_invalid_planes(&cube, 0.0, DEFAULT_FRAC_VALID, true).unwrap();
        assert_eq!(removed, ["F606W", "F200W"], "HST COSMIC_RAY excludes, JWST WARM does not, an unknown dataset tests DO_NOT_USE only");
        assert_eq!(cube_filters(&valid), ["F814W", "F150W", "F277W"]);
        let note = history(&valid.header).join(" ");
        assert!(note.contains("Removed invalid filters (threshold=0.0, frac_valid=0.01); DQ-flagged pixels do not count:"), "{note}");
        for expected in [
            "F606W (hst): mask 0x3FFF (REED_SOLOMON..COSMIC_RAY_SINGLE)",
            "F814W (hst): mask 0x3FFF (REED_SOLOMON..COSMIC_RAY_SINGLE)",
            "F150W (jwst): mask 0x1 (DO_NOT_USE)",
            "F200W (jwst): mask 0x1 (DO_NOT_USE)",
            "F277W (unknown): mask 0x1 (DO_NOT_USE assumed)",
        ] {
            assert!(note.contains(expected), "{expected} missing from {note}");
        }
        assert!(!note.contains("bit 0 (DO_NOT_USE) do not count"), "{note}");
        let (blind, removed) = drop_invalid_planes(&cube, 0.0, DEFAULT_FRAC_VALID, false).unwrap();
        assert!(removed.is_empty() && blind.filters.len() == 5);
        assert!(history(&blind.header).contains(&"Removed invalid filters (threshold=0.0, frac_valid=0.01)".to_string()), "{:?}", history(&blind.header));
    }

    fn noise_map(values: &[f32]) -> Array2<f32> {
        let k = (values.len() as f64).sqrt() as usize;
        Array2::from_shape_vec((k, k), values.to_vec()).unwrap()
    }

    #[test]
    fn assemble_cube_with_noise_pads_the_maps_to_the_largest_k_and_fills_the_filters_columns() {
        let registry = FilterRegistry::builtin();
        let dims = (4, 5);
        let grid = cd_grid(dims, 0.04, 0.0);
        let inputs = || {
            vec![
                plane_input(&registry, "JWST", "F444W", &grid, &[("BUNIT", "Jy")], |_, _| 4.0),
                plane_input(&registry, "JWST", "F150W", &grid, &[("BUNIT", "Jy")], |_, _| 1.5),
                plane_input(&registry, "HST", "F814W", &grid, &[("BUNIT", "Jy")], |_, _| 0.8),
            ]
        };
        let delta = noise_map(&[1.0]);
        let wide = Array2::from_shape_fn((5, 5), |(y, x)| 1.0 / (1.0 + (y as f32 - 2.0).abs() + (x as f32 - 2.0).abs()));
        let triangle = noise_map(&[0.0, 0.0, 0.0, 0.5, 1.0, 0.5, 0.0, 0.0, 0.0]);
        let noise = || {
            vec![
                InputNoise { correlation: delta.clone(), noiscorr_align: 1.0 },
                InputNoise { correlation: wide.clone(), noiscorr_align: 2.0 },
                InputNoise { correlation: triangle.clone(), noiscorr_align: 2f64.sqrt() },
            ]
        };
        let base = cube_base_header(&inputs()[0].header);
        let mef = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        let assembled = assemble_cube_with_noise(inputs(), noise(), 0, base.clone(), mef).unwrap();
        assert_eq!(assembled.order, [2, 1, 0]);
        let cube_noise = assembled.noise.as_ref().expect("a mef cube carries its noise correlation");
        assert_eq!(cube_noise.source, NoiseSource::Align);
        assert_eq!(cube_noise.k(), 5);
        let planes = &cube_noise.planes;
        assert_eq!(planes.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), ["F814W", "F150W", "F444W"]);
        assert!(planes.iter().all(|p| p.noiscorr_match == 1.0));
        assert_eq!(planes.iter().map(|p| p.noiscorr_align).collect::<Vec<_>>(), [2f64.sqrt(), 2.0, 1.0]);
        assert_eq!(planes[0].correlation, triangle, "a plane keeps its own map");
        assert_eq!(planes[2].correlation, delta, "the grid anchor keeps its own map");
        let padded = cube_noise.padded_maps();
        assert_eq!(padded.len(), 3);
        assert!(padded.iter().all(|map| map.dim() == (5, 5)));
        assert_eq!(padded[1], wide);
        for ((y, x), value) in padded[0].indexed_iter() {
            let inner = (1..=3).contains(&y) && (1..=3).contains(&x);
            assert_eq!(*value, if inner { triangle[[y - 1, x - 1]] } else { 0.0 }, "({x}, {y})");
        }
        assert_eq!(padded[2][[2, 2]], 1.0);
        assert_eq!(padded[2].iter().filter(|v| **v != 0.0).count(), 1);
        let header = cube_noise.header();
        assert_eq!(header.get_i64(CARD_NCORRK), Some(5));
        assert_eq!(header.get(CARD_NCORRSRC), Some("align"));

        let table = assembled.filters_table().unwrap();
        let columns = table.columns();
        let names: Vec<&str> = columns.iter().map(|c| c.name).collect();
        assert_eq!(names[9..], ["NOISCORR_ALIGN", "NOISCORR_MATCH", "MATCH"]);
        let TableValues::F64(align) = columns[9].values else { panic!("NOISCORR_ALIGN is not numeric") };
        assert_eq!(align, [2f64.sqrt(), 2.0, 1.0]);
        let TableValues::F64(matched) = columns[10].values else { panic!("NOISCORR_MATCH is not numeric") };
        assert_eq!(matched, [1.0, 1.0, 1.0]);
        let TableValues::Text(status) = columns[11].values else { panic!("MATCH is not text") };
        assert_eq!(status, ["unmatched", "unmatched", "unmatched"], "nothing is PSF-matched at the cube stage, the grid anchor included");
        let TableValues::F64(psf) = columns[7].values else { panic!("PSF_FWHM is not numeric") };
        assert!(psf.iter().all(|v| v.is_nan()), "no plane is matched at the cube stage");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube.fits").to_string_lossy().into_owned();
        let hdus = [
            ImageHdu { data: HduData::F32Cube(&assembled.cube.planes), header: Some(&assembled.cube.header), extname: "SCI", extver: 1 },
            ImageHdu { data: HduData::F32Cube(&padded), header: Some(&header), extname: NOISCORR_EXTNAME, extver: 1 },
        ];
        let tables = [BinTableHdu { columns: &columns, extname: FILTERS_EXTNAME, extver: 1 }];
        write_mef_with_tables(&path, None, &hdus, &tables).unwrap();
        let read = load_bintable(&path, find_bintable(&path, FILTERS_EXTNAME, 1).unwrap()).unwrap();
        let back = CubeNoise::from_hdu(&read, &header, padded.clone()).unwrap();
        assert_eq!(&back, cube_noise, "the padded maps come back trimmed to their own support");
        assert_eq!(back.padded_maps(), padded);
        assert_eq!(match_statuses(&read).unwrap(), Some(vec![MatchStatus::Unmatched; 3]));
        assert_eq!(planes_from_table(&read).unwrap(), assembled.cube.filters);
        let mut wrong_k = header.clone();
        wrong_k.set(CARD_NCORRK, "3".to_string());
        assert!(CubeNoise::from_hdu(&read, &wrong_k, padded.clone()).is_err());
        let mut wrong_source = header.clone();
        wrong_source.set(CARD_NCORRSRC, "Align".to_string());
        assert!(CubeNoise::from_hdu(&read, &wrong_source, padded.clone()).is_err());
        assert!(CubeNoise::from_hdu(&read, &header, padded[..2].to_vec()).is_err());

        let kept = cube_noise.for_planes(&[assembled.cube.filters[2].clone(), assembled.cube.filters[0].clone()]).unwrap();
        assert_eq!(kept.planes.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), ["F444W", "F814W"]);
        assert_eq!(kept.k(), 3, "the padded size follows the kept planes");
        let unknown = CubePlane { filter: "F999W".into(), ..assembled.cube.filters[0].clone() };
        assert_eq!(cube_noise.for_planes(&[unknown]), Err(CubeError::NoiseFilter("F999W".into())));

        let lightstack = assemble_cube_with_noise(inputs(), noise(), 0, base.clone(), CubeOptions { layout: CubeLayout::Lightstack, order: PlaneOrder::Wavelength }).unwrap();
        assert!(lightstack.noise.is_none(), "the lightstack layout carries no noise correlation");
        let plain = assemble_cube(inputs(), 0, base, mef).unwrap();
        assert!(plain.noise.is_none());
        assert_eq!(plain.cube.filters, assembled.cube.filters);
        assert!(assembled.cube.planes.iter().zip(&plain.cube.planes).all(|(a, b)| a == b));
        let plain_table = plain.filters_table().unwrap();
        let plain_columns = plain_table.columns();
        assert_eq!(plain_columns[9..].iter().map(|c| c.name).collect::<Vec<_>>(), ["MATCH"], "a mef cube without NOISCORR still states its match status");
        assert_eq!(MatchStatus::parse("external"), Some(MatchStatus::External));
        assert_eq!(MatchStatus::parse("other"), None);
        assert_eq!(MatchStatus::Matched.label(), "matched");
    }

    #[test]
    fn filters_table_refuses_noise_and_status_rows_that_do_not_line_up_with_its_planes() {
        let registry = FilterRegistry::builtin();
        let grid = cd_grid((4, 5), 0.04, 0.0);
        let inputs = vec![
            plane_input(&registry, "JWST", "F444W", &grid, &[("BUNIT", "Jy")], |_, _| 4.0),
            plane_input(&registry, "JWST", "F150W", &grid, &[("BUNIT", "Jy")], |_, _| 1.5),
            plane_input(&registry, "HST", "F814W", &grid, &[("BUNIT", "Jy")], |_, _| 0.8),
        ];
        let noise = (0..3).map(|_| InputNoise { correlation: noise_map(&[1.0]), noiscorr_align: 1.0 }).collect();
        let base = cube_base_header(&inputs[0].header);
        let assembled = assemble_cube_with_noise(inputs, noise, 0, base, CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength }).unwrap();
        let cube_noise = assembled.noise.as_ref().unwrap();
        let filters = &assembled.cube.filters;
        assert_eq!(FiltersTable::new(&filters[..2]).with_noise(cube_noise).err(), Some(CubeError::CompanionMismatch(NOISCORR_EXTNAME, 3, 2)));
        assert_eq!(FiltersTable::new(&filters[..2]).with_match(&[MatchStatus::Unmatched; 3]).err(), Some(CubeError::CompanionMismatch(COLUMN_MATCH, 3, 2)));
        let reversed: Vec<CubePlane> = filters.iter().rev().cloned().collect();
        assert_eq!(FiltersTable::new(&reversed).with_noise(cube_noise).err(), Some(CubeError::NoiseFilter("F444W".into())), "rows are matched by filter, never shifted");
        let realigned = cube_noise.for_planes(&reversed).unwrap();
        assert!(FiltersTable::new(&reversed).with_noise(&realigned).is_ok());
    }

    #[test]
    fn a_noiscorr_hdu_is_read_only_when_ncorrk_is_a_positive_odd_size() {
        let registry = FilterRegistry::builtin();
        let grid = cd_grid((4, 5), 0.04, 0.0);
        let inputs = vec![plane_input(&registry, "JWST", "F444W", &grid, &[("BUNIT", "Jy")], |_, _| 4.0)];
        let noise = vec![InputNoise { correlation: noise_map(&[1.0]), noiscorr_align: 1.0 }];
        let base = cube_base_header(&inputs[0].header);
        let assembled = assemble_cube_with_noise(inputs, noise, 0, base, CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength }).unwrap();
        let table = assembled.filters_table().unwrap();
        let columns = table.columns();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube.fits").to_string_lossy().into_owned();
        let hdus = [ImageHdu { data: HduData::F32Cube(&assembled.cube.planes), header: Some(&assembled.cube.header), extname: "SCI", extver: 1 }];
        write_mef_with_tables(&path, None, &hdus, &[BinTableHdu { columns: &columns, extname: FILTERS_EXTNAME, extver: 1 }]).unwrap();
        let read = load_bintable(&path, find_bintable(&path, FILTERS_EXTNAME, 1).unwrap()).unwrap();
        let declaring = |k: Option<&str>| {
            let mut header = noise_correlation_header(1, NoiseSource::Align);
            match k {
                Some(k) => header.set(CARD_NCORRK, k.to_string()),
                None => header.remove(CARD_NCORRK),
            }
            header
        };
        let even = |at: (usize, usize)| Array2::from_shape_fn((4, 4), |index| if index == (2, 2) { 1.0 } else if index == at { 0.1 } else { 0.0 });
        let refused = [
            (Some("4"), vec![even((0, 0))]),
            (Some("4"), vec![even((3, 3))]),
            (Some("0"), vec![Array2::<f32>::zeros((0, 0))]),
            (Some("-1"), vec![noise_map(&[1.0])]),
            (None, vec![noise_map(&[1.0])]),
        ];
        for (k, maps) in refused {
            let error = CubeNoise::from_hdu(&read, &declaring(k), maps).expect_err(&format!("NCORRK {k:?} has no centre pixel and must be refused"));
            assert!(error.to_string().contains("NCORRK") && error.to_string().contains("positive odd"), "{k:?}: {error}");
        }
        let accepted = CubeNoise::from_hdu(&read, &declaring(Some("1")), vec![noise_map(&[1.0])]).unwrap();
        assert_eq!(accepted.planes[0].correlation, noise_map(&[1.0]));
    }
}
