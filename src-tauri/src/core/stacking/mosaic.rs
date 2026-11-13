use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use ndarray::{s, Array2, Axis, Zip};
use serde::Serialize;

use crate::core::alignment::wcs_reproject::{point_source_residual, reproject_to_grid, StarResidual};
use crate::core::analysis::star_detection::detect_star_extents;
use crate::core::astrometry::gwcs::sip_fit::lu_solve;
use crate::core::astrometry::local_solve::GAIA_ONLINE_TIER;
use crate::core::astrometry::plate_solve::SolveResult;
use crate::core::astrometry::solve_hints::solve_hints;
use crate::core::astrometry::wcs::{pixel_center, pixel_edge_corners, WcsKind, WcsTransform};
use crate::core::imaging::dq_flags::{exclusion_map, DqTable};
use crate::core::imaging::star_mask::{generate_star_mask_from_detection, StarMaskConfig};
use crate::core::stacking::memory::{check_memory, effective_memory, format_bytes, header_size, MemoryInfo, MemoryJob};
use crate::core::stacking::{stop_if_cancelled, CancelCheck};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::asdf_bridge::try_extract_rgb_from_asdf;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::try_extract_rgb_mmap;
use crate::infra::fits::writer::is_wcs_card;
use crate::infra::image_source::{load_plane, load_plane_header, LoadedPlane};
use crate::infra::wcs_source::{header_wcs, load_wcs};
use crate::math::exact_median_f64;
use crate::types::constants::{HEADER_COMBINE_METHOD, HEADER_NCOMBINE};
use crate::types::header::HduHeader;
use crate::types::image_ref::{source_stem, ImageRef, OutputStems};

pub use crate::types::stacking::{CanvasSpec, LevelModel, MosaicConfig, RejectSpec};

pub const EDGE_SAMPLES: usize = 64;

const F32_BYTES: u64 = 4;
const ACCUMULATOR_PLANES: u64 = 2;
const EDGE_SNAP_PX: f64 = 1e-6;
const JACOBIAN_STEP_PX: f64 = 1.0;
const ARCSEC_PER_DEG: f64 = 3600.0;
const MIN_MEAN_DIRECTION: f64 = 1e-6;

const CANVAS_WCS_KEYS: [&str; 26] = [
    "CTYPE1", "CTYPE2", "CUNIT1", "CUNIT2", "CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2", "CDELT1", "CDELT2", "CROTA1",
    "CROTA2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "PC1_1", "PC1_2", "PC2_1", "PC2_2", "LONPOLE", "LATPOLE", "RADESYS",
    "RADECSYS", "EQUINOX", "EPOCH",
];
const CANVAS_WCS_PREFIXES: [&str; 6] = ["A_", "B_", "AP_", "BP_", "PV1_", "PV2_"];

#[derive(Debug, Clone)]
pub struct TileGeometry {
    pub name: String,
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub planes: usize,
    pub wcs: Arc<WcsTransform>,
    pub bunit: Option<String>,
    pub single_exposure: bool,
    pub solve: Option<SolveResult>,
}

pub struct UnsolvedTile<'a> {
    pub index: usize,
    pub name: &'a str,
    pub path: &'a str,
}

pub type TileSolver<'a> = &'a dyn Fn(&UnsolvedTile<'_>) -> Result<SolveResult>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CanvasWindow {
    pub x0: usize,
    pub y0: usize,
    pub cols: usize,
    pub rows: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TileEstimate {
    pub name: String,
    pub coverage_frac: f64,
    pub scale_ratio: f64,
    pub rotation_deg: f64,
    pub wcs_kind: WcsKind,
    pub window: CanvasWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct OverlapPair {
    pub i: usize,
    pub j: usize,
    pub frac: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Component {
    pub tiles: Vec<usize>,
    pub reference: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MosaicEstimate {
    pub cols: usize,
    pub rows: usize,
    pub megapixels: f64,
    pub ram_bytes: u64,
    pub canvas_cards: Vec<(String, String)>,
    pub tiles: Vec<TileEstimate>,
    pub pairs: Vec<OverlapPair>,
    pub components: Vec<Component>,
    pub isolated: Vec<usize>,
    pub warnings: Vec<String>,
    pub refusal: Option<String>,
    pub default_reject: Option<RejectSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct OverlapStat {
    pub i: usize,
    pub j: usize,
    pub d: f64,
    pub w: f64,
    pub log_ratio: f64,
    pub log_w: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelSolution {
    pub offsets: Vec<f64>,
    pub scales: Vec<f64>,
    pub unmatched: Vec<usize>,
    pub unscaled: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct LevelWindow {
    pub block_x0: usize,
    pub block_y0: usize,
    pub planes: Vec<Array2<f32>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelReport {
    pub planes: Vec<LevelSolution>,
    pub stats: Vec<Vec<OverlapStat>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MosaicStage {
    Overlaps,
    Levels,
    Warp,
    Reject,
    Residuals,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PairResidual {
    pub i: usize,
    pub j: usize,
    pub dx: f64,
    pub dy: f64,
    pub matched: usize,
    pub rms_px: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TileRejection {
    pub tested_px: u64,
    pub rejected_px: u64,
    pub sigma: f64,
    #[serde(skip)]
    pub pixels: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub struct MosaicOutput {
    pub planes: Vec<Array2<f32>>,
    pub coverage: Array2<i32>,
    pub cards: Vec<(String, String)>,
    pub history: Vec<String>,
    pub levels: Vec<LevelSolution>,
    pub pair_residuals: Vec<PairResidual>,
    pub rejected_px: u64,
    pub rejections: Vec<TileRejection>,
    pub warnings: Vec<String>,
}

fn header_from_cards(cards: &[(String, String)]) -> HduHeader {
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key, value.clone());
    }
    header
}

fn shifted_header(frame: &HduHeader, dx: i64, dy: i64, cols: usize, rows: usize) -> HduHeader {
    let mut header = frame.clone();
    header.set("NAXIS1", cols.to_string());
    header.set("NAXIS2", rows.to_string());
    for (key, shift) in [("CRPIX1", dx), ("CRPIX2", dy)] {
        if let Some(crpix) = header.get_f64(key) {
            header.set_f64(key, crpix - shift as f64);
        }
    }
    header
}

pub fn canvas_wcs(cards: &[(String, String)]) -> Result<WcsTransform> {
    WcsTransform::from_header(&header_from_cards(cards)).context("The canvas cards hold no celestial WCS")
}

pub fn window_cards(canvas_cards: &[(String, String)], window: &CanvasWindow) -> Vec<(String, String)> {
    shifted_header(&header_from_cards(canvas_cards), window.x0 as i64, window.y0 as i64, window.cols, window.rows).cards
}

fn colour_planes(header: &HduHeader) -> Result<usize> {
    let compressed = header.get("ZIMAGE").is_some_and(|v| v.trim() == "T") && header.get("ZNAXIS1").is_some();
    let prefix = if compressed { "Z" } else { "" };
    let axes = header.get_i64(&format!("{prefix}NAXIS")).unwrap_or(2);
    let depth = header.get_i64(&format!("{prefix}NAXIS3")).unwrap_or(1);
    match (axes, depth) {
        (..=2, _) | (_, 1) => Ok(1),
        (3, 3) => Ok(3),
        _ => bail!("it has {depth} planes along NAXIS3, and a mosaic takes mono or RGB panels"),
    }
}

struct TileHeader {
    name: String,
    path: String,
    source: String,
    header: HduHeader,
    planes: usize,
    width: usize,
    height: usize,
    wcs: Option<WcsTransform>,
}

fn read_tile_header(path: &str, stems: &mut OutputStems) -> Result<TileHeader> {
    let reference = ImageRef::parse(path);
    let name = reference.output_stem_in(stems);
    let header = load_plane_header(&reference).with_context(|| format!("Tile '{name}': the header of {path} cannot be read"))?;
    let planes = colour_planes(&header).with_context(|| format!("Tile '{name}' ({path})"))?;
    let size = header_size(&header);
    let wcs = load_wcs(path, &header).ok();
    Ok(TileHeader {
        name,
        path: path.to_string(),
        source: reference.path.clone(),
        header,
        planes,
        width: size.width as usize,
        height: size.height as usize,
        wcs,
    })
}

fn refuse_without_hints(tile: &TileHeader) -> Result<()> {
    if tile.wcs.is_some() {
        return Ok(());
    }
    let hints = solve_hints(&tile.header, tile.width, tile.height);
    let missing = match (&hints.centre, &hints.scale) {
        (None, _) => "no pointing in the header",
        (_, None) => "no pixel scale in the header (a pixel size with a focal length)",
        _ => return Ok(()),
    };
    bail!(
        "Tile '{}' ({}) has no celestial WCS and {missing}: solve it in Plate Solve with a hint, then Write WCS",
        tile.name,
        tile.path
    )
}

fn solved_header(header: &HduHeader, cards: &[(String, String)]) -> HduHeader {
    let mut merged = header.clone();
    let stale: Vec<String> =
        merged.cards.iter().map(|(key, _)| key.clone()).filter(|key| is_wcs_card(key.trim()) || key.trim() == "RADECSYS").collect();
    for key in stale {
        merged.remove(&key);
    }
    for (key, value) in cards {
        merged.set(key.trim(), value.clone());
    }
    merged
}

fn tile_geometry(index: usize, tile: TileHeader, solve: TileSolver) -> Result<TileGeometry> {
    let TileHeader { name, path, source, header, planes, width, height, wcs } = tile;
    let (wcs, solved) = match wcs {
        Some(wcs) => (wcs, None),
        None => {
            let solved = solve(&UnsolvedTile { index, name: &name, path: &path })?;
            let wcs = header_wcs(&solved_header(&header, &solved.wcs_cards))
                .with_context(|| format!("Tile '{name}' ({path}): the solved WCS cannot be used"))?;
            (wcs, Some(solved))
        }
    };
    Ok(TileGeometry {
        bunit: card_text(&header, "BUNIT"),
        single_exposure: is_single_exposure(&source, &header),
        name,
        path,
        width,
        height,
        planes,
        wcs: Arc::new(wcs),
        solve: solved,
    })
}

pub fn load_tiles_with(paths: &[String], solve: TileSolver) -> Result<Vec<TileGeometry>> {
    let mut stems = OutputStems::default();
    let headers = paths.iter().map(|path| read_tile_header(path, &mut stems)).collect::<Result<Vec<_>>>()?;
    headers.iter().try_for_each(refuse_without_hints)?;
    headers.into_iter().enumerate().map(|(index, tile)| tile_geometry(index, tile, solve)).collect()
}

pub fn load_tiles(paths: &[String]) -> Result<Vec<TileGeometry>> {
    load_tiles_with(paths, &|tile: &UnsolvedTile<'_>| {
        bail!("Tile '{}' has no celestial WCS ({}): plate solve it in Astrometry, then click Write WCS", tile.name, tile.path)
    })
}

fn is_level_2_product(stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    LEVEL_2_SUFFIXES.iter().any(|suffix| stem.ends_with(suffix))
}

fn is_single_exposure(path: &str, header: &HduHeader) -> bool {
    let named = is_level_2_product(&source_stem(path))
        || card_text(header, FILENAME_CARD).is_some_and(|name| is_level_2_product(&source_stem(&name)));
    let stacked = header.get(HEADER_COMBINE_METHOD).is_some() || header.get_f64(HEADER_NCOMBINE).is_some_and(|n| n > 1.0);
    named && !stacked
}

pub fn default_rejection(tiles: &[TileGeometry]) -> Option<RejectSpec> {
    (!tiles.is_empty() && tiles.iter().all(|t| t.single_exposure)).then(RejectSpec::default)
}

type Matrix = [[f64; 2]; 2];

fn det(m: &Matrix) -> f64 {
    m[0][0] * m[1][1] - m[0][1] * m[1][0]
}

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

fn rotation_deg(jacobian: &Matrix) -> f64 {
    let deg = (jacobian[1][0] - jacobian[0][1]).atan2(jacobian[0][0] + jacobian[1][1]).to_degrees();
    if deg <= -180.0 {
        deg + 360.0
    } else {
        deg
    }
}

fn axes_aligned_with(jacobian: &Matrix) -> Matrix {
    if det(jacobian) > 0.0 {
        let (sin, cos) = (jacobian[1][0] - jacobian[0][1]).atan2(jacobian[0][0] + jacobian[1][1]).sin_cos();
        [[cos, -sin], [sin, cos]]
    } else {
        let (sin, cos) = (-jacobian[1][0] - jacobian[0][1]).atan2(jacobian[1][1] - jacobian[0][0]).sin_cos();
        [[-cos, -sin], [-sin, cos]]
    }
}

fn tile_jacobian(tile: &TileGeometry, frame: &WcsTransform) -> Option<Matrix> {
    let (cx, cy) = pixel_center(tile.width, tile.height);
    let h = JACOBIAN_STEP_PX;
    let probe = [(cx + h, cy), (cx - h, cy), (cx, cy + h), (cx, cy - h)].map(|(x, y)| {
        let sky = tile.wcs.pixel_to_world(x, y);
        frame.world_to_pixel(sky.ra, sky.dec)
    });
    let step = 2.0 * h;
    let jacobian = [
        [(probe[0].0 - probe[1].0) / step, (probe[2].0 - probe[3].0) / step],
        [(probe[0].1 - probe[1].1) / step, (probe[2].1 - probe[3].1) / step],
    ];
    let usable = jacobian.iter().flatten().all(|v| v.is_finite()) && det(&jacobian).abs() > 0.0;
    usable.then_some(jacobian)
}

fn edge_samples(width: usize, height: usize) -> Vec<(f64, f64)> {
    let corners = pixel_edge_corners(width, height);
    (0..4)
        .flat_map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            (0..EDGE_SAMPLES).map(move |m| {
                let t = m as f64 / EDGE_SAMPLES as f64;
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            })
        })
        .collect()
}

fn shoelace(points: &[(f64, f64)]) -> f64 {
    let Some(&(ox, oy)) = points.first() else {
        return 0.0;
    };
    let n = points.len();
    (0..n)
        .map(|k| {
            let (a, b) = (points[k], points[(k + 1) % n]);
            (a.0 - ox) * (b.1 - oy) - (b.0 - ox) * (a.1 - oy)
        })
        .sum::<f64>()
        / 2.0
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn convex_hull(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let mut hull: Vec<(f64, f64)> = Vec::with_capacity(2 * sorted.len());
    for pass in [&sorted[..], &sorted.iter().rev().copied().collect::<Vec<_>>()[..]] {
        let start = hull.len();
        for &p in pass {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

fn clipped_area(subject: &[(f64, f64)], convex: &[(f64, f64)]) -> f64 {
    if convex.len() < 3 {
        return 0.0;
    }
    let mut polygon = subject.to_vec();
    for k in 0..convex.len() {
        let (a, b) = (convex[k], convex[(k + 1) % convex.len()]);
        let input = std::mem::take(&mut polygon);
        let Some(&last) = input.last() else {
            return 0.0;
        };
        let mut previous = last;
        for &current in &input {
            let (side_previous, side_current) = (cross(a, b, previous), cross(a, b, current));
            if (side_current >= 0.0) != (side_previous >= 0.0) {
                let t = side_previous / (side_previous - side_current);
                polygon.push((previous.0 + (current.0 - previous.0) * t, previous.1 + (current.1 - previous.1) * t));
            }
            if side_current >= 0.0 {
                polygon.push(current);
            }
            previous = current;
        }
    }
    shoelace(&polygon).abs()
}

struct Outline {
    points: Vec<(f64, f64)>,
    min: (f64, f64),
    max: (f64, f64),
    area: f64,
}

impl Outline {
    fn new(points: Vec<(f64, f64)>) -> Self {
        let (mut min, mut max) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
        for &(x, y) in &points {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        let area = shoelace(&points).abs();
        Outline { points, min, max, area }
    }

    fn meets(&self, other: &Outline) -> bool {
        self.min.0 <= other.max.0 && other.min.0 <= self.max.0 && self.min.1 <= other.max.1 && other.min.1 <= self.max.1
    }
}

fn project_outline(tile: &TileGeometry, frame: &WcsTransform) -> Option<Outline> {
    let sky: Vec<(f64, f64)> =
        tile.wcs.pixel_to_world_batch(&edge_samples(tile.width, tile.height)).into_iter().map(|c| (c.ra, c.dec)).collect();
    let points = frame.world_to_pixel_batch(&sky);
    points.iter().all(|p| p.0.is_finite() && p.1.is_finite()).then(|| Outline::new(points))
}

fn pixel_span(min: f64, max: f64) -> (i64, i64) {
    ((min + 0.5 + EDGE_SNAP_PX).floor() as i64, (max - 0.5 - EDGE_SNAP_PX).ceil() as i64)
}

fn span_len(first: i64, last: i64) -> usize {
    usize::try_from(last.saturating_sub(first).saturating_add(1).max(0)).unwrap_or(usize::MAX)
}

fn offset_from(origin: i64, value: i64) -> usize {
    usize::try_from(value.saturating_sub(origin).max(0)).unwrap_or(usize::MAX)
}

fn tan_header((ra, dec): (f64, f64), cd: &Matrix) -> HduHeader {
    let mut header = HduHeader::empty();
    for (key, value) in [
        ("NAXIS1", "1"),
        ("NAXIS2", "1"),
        ("CTYPE1", "'RA---TAN'"),
        ("CTYPE2", "'DEC--TAN'"),
        ("CUNIT1", "'deg'"),
        ("CUNIT2", "'deg'"),
    ] {
        header.set(key, value.to_string());
    }
    for (key, value) in [
        ("CRPIX1", 1.0),
        ("CRPIX2", 1.0),
        ("CRVAL1", ra),
        ("CRVAL2", dec),
        ("CD1_1", cd[0][0]),
        ("CD1_2", cd[0][1]),
        ("CD2_1", cd[1][0]),
        ("CD2_2", cd[1][1]),
    ] {
        header.set_f64(key, value);
    }
    header.set("RADESYS", "'ICRS'".to_string());
    header
}

fn mean_direction(tiles: &[TileGeometry]) -> Result<(f64, f64), String> {
    let mut sum = [0.0f64; 3];
    for tile in tiles {
        let (cx, cy) = pixel_center(tile.width, tile.height);
        let sky = tile.wcs.pixel_to_world(cx, cy);
        if !(sky.ra.is_finite() && sky.dec.is_finite()) {
            return Err(format!("Tile '{}' has no sky position at its centre: check its WCS.", tile.name));
        }
        let (sin_ra, cos_ra) = sky.ra.to_radians().sin_cos();
        let (sin_dec, cos_dec) = sky.dec.to_radians().sin_cos();
        sum = [sum[0] + cos_dec * cos_ra, sum[1] + cos_dec * sin_ra, sum[2] + sin_dec];
    }
    let norm = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    if !(norm > MIN_MEAN_DIRECTION * tiles.len() as f64) {
        return Err("The tiles are spread over the whole sky, so no TAN canvas holds them: check their WCS.".to_string());
    }
    Ok((sum[1].atan2(sum[0]).to_degrees().rem_euclid(360.0), (sum[2] / norm).clamp(-1.0, 1.0).asin().to_degrees()))
}

fn finest_scale_deg(tiles: &[TileGeometry]) -> Result<f64, String> {
    let mut finest = f64::INFINITY;
    for tile in tiles {
        let scale = tile.wcs.pixel_scale_arcsec();
        if !(scale.is_finite() && scale > 0.0) {
            return Err(format!("Tile '{}' has no usable pixel scale in its WCS.", tile.name));
        }
        finest = finest.min(scale);
    }
    Ok(finest / ARCSEC_PER_DEG)
}

fn wcs_of(header: &HduHeader) -> Result<WcsTransform, String> {
    WcsTransform::from_header(header).map_err(|e| format!("The canvas WCS cannot be built: {e:#}"))
}

fn auto_frame(tiles: &[TileGeometry], north_up: bool, reference: usize) -> Result<HduHeader, String> {
    let centre = mean_direction(tiles)?;
    let scale = finest_scale_deg(tiles)?;
    let north: Matrix = [[-scale, 0.0], [0.0, scale]];
    let provisional = tan_header(centre, &north);
    if north_up {
        return Ok(provisional);
    }
    let tile = &tiles[reference];
    let jacobian = tile_jacobian(tile, &wcs_of(&provisional)?)
        .ok_or_else(|| format!("Tile '{}' gives the canvas no orientation: its WCS is degenerate at its centre.", tile.name))?;
    Ok(tan_header(centre, &multiply(&north, &axes_aligned_with(&jacobian))))
}

fn is_canvas_wcs_key(key: &str) -> bool {
    CANVAS_WCS_KEYS.contains(&key) || CANVAS_WCS_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
}

fn like_frame(cards: &[(String, String)]) -> Result<HduHeader, String> {
    let mut header = HduHeader::empty();
    header.set("NAXIS1", "1".to_string());
    header.set("NAXIS2", "1".to_string());
    for (key, value) in cards {
        let key = key.trim().to_ascii_uppercase();
        if is_canvas_wcs_key(&key) {
            header.set(&key, value.clone());
        }
    }
    WcsTransform::from_header(&header).map_err(|e| format!("The canvas to reuse has no celestial WCS: {e:#}"))?;
    Ok(header)
}

struct Canvas {
    header: HduHeader,
    frame: WcsTransform,
    origin: (i64, i64),
    cols: usize,
    rows: usize,
    outlines: Vec<Outline>,
}

fn build_canvas(tiles: &[TileGeometry], spec: &CanvasSpec) -> Result<Canvas, String> {
    let header = match spec {
        CanvasSpec::Auto { north_up, reference } => auto_frame(tiles, *north_up, reference.unwrap_or(0))?,
        CanvasSpec::Like { cards } => like_frame(cards)?,
    };
    let frame = wcs_of(&header)?;
    let mut outlines = Vec::with_capacity(tiles.len());
    for tile in tiles {
        let outline = project_outline(tile, &frame).ok_or_else(|| {
            format!("Tile '{}' does not project onto the canvas: it lies too far from the canvas centre, so check its WCS.", tile.name)
        })?;
        outlines.push(outline);
    }
    let min = outlines.iter().fold((f64::INFINITY, f64::INFINITY), |m, o| (m.0.min(o.min.0), m.1.min(o.min.1)));
    let max = outlines.iter().fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |m, o| (m.0.max(o.max.0), m.1.max(o.max.1)));
    let (x_first, x_last) = pixel_span(min.0, max.0);
    let (y_first, y_last) = pixel_span(min.1, max.1);
    let (cols, rows) = (span_len(x_first, x_last), span_len(y_first, y_last));
    Ok(Canvas { header: shifted_header(&header, x_first, y_first, cols, rows), frame, origin: (x_first, y_first), cols, rows, outlines })
}

impl Canvas {
    fn window(&self, outline: &Outline) -> CanvasWindow {
        let (x_first, x_last) = pixel_span(outline.min.0, outline.max.0);
        let (y_first, y_last) = pixel_span(outline.min.1, outline.max.1);
        CanvasWindow {
            x0: offset_from(self.origin.0, x_first),
            y0: offset_from(self.origin.1, y_first),
            cols: span_len(x_first, x_last),
            rows: span_len(y_first, y_last),
        }
    }

    fn pixels(&self) -> u64 {
        (self.cols as u64).saturating_mul(self.rows as u64)
    }
}

fn overlap_pairs(outlines: &[Outline], min_frac: f64) -> Vec<OverlapPair> {
    let hulls: Vec<Vec<(f64, f64)>> = outlines.iter().map(|o| convex_hull(&o.points)).collect();
    let mut pairs = Vec::new();
    for i in 0..outlines.len() {
        for j in i + 1..outlines.len() {
            let smaller = outlines[i].area.min(outlines[j].area);
            if !outlines[i].meets(&outlines[j]) || smaller <= 0.0 {
                continue;
            }
            let frac = (clipped_area(&outlines[i].points, &hulls[j]) / smaller).min(1.0);
            if frac >= min_frac {
                pairs.push(OverlapPair { i, j, frac });
            }
        }
    }
    pairs
}

fn find_root(parent: &mut [usize], mut tile: usize) -> usize {
    while parent[tile] != tile {
        parent[tile] = parent[parent[tile]];
        tile = parent[tile];
    }
    tile
}

fn connected_groups(n: usize, pairs: &[OverlapPair]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    for pair in pairs {
        let (a, b) = (find_root(&mut parent, pair.i), find_root(&mut parent, pair.j));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut slot = vec![usize::MAX; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for tile in 0..n {
        let root = find_root(&mut parent, tile);
        if slot[root] == usize::MAX {
            slot[root] = groups.len();
            groups.push(Vec::new());
        }
        groups[slot[root]].push(tile);
    }
    groups
}

fn assign_references(groups: Vec<Vec<usize>>, chosen: &[Option<usize>], tiles: &[TileGeometry]) -> Result<Vec<Component>> {
    if let Some(extra) = chosen.iter().enumerate().skip(groups.len()).find_map(|(k, r)| r.map(|_| k)) {
        bail!("A reference is chosen for group {} but the tiles form {} groups", extra + 1, groups.len());
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(k, members)| {
            let reference = match chosen.get(k).copied().flatten() {
                Some(r) if members.contains(&r) => r,
                Some(r) => bail!("Tile '{}' is not in group {}: the reference of a group is one of its own tiles", tiles[r].name, k + 1),
                None => members[0],
            };
            Ok(Component { tiles: members, reference })
        })
        .collect()
}

fn check_request(tiles: &[TileGeometry], cfg: &MosaicConfig) -> Result<()> {
    if tiles.is_empty() {
        bail!("A mosaic needs at least one tile");
    }
    if let Some(tile) = tiles.iter().find(|t| t.width == 0 || t.height == 0) {
        bail!("Tile '{}' has no pixels", tile.name);
    }
    if let CanvasSpec::Auto { reference: Some(r), .. } = cfg.canvas {
        if r >= tiles.len() {
            bail!("Canvas reference tile {} is out of range for {} tiles", r, tiles.len());
        }
    }
    if let Some(r) = cfg.references.iter().flatten().find(|&&r| r >= tiles.len()) {
        bail!("Reference tile {} is out of range for {} tiles", r, tiles.len());
    }
    Ok(())
}

fn mixed_planes(tiles: &[TileGeometry]) -> Option<String> {
    let colour = tiles.iter().find(|t| t.planes > 1)?;
    let mono = tiles.iter().find(|t| t.planes == 1)?;
    Some(format!(
        "Tile '{}' is RGB and tile '{}' is mono: a mosaic takes tiles of one kind, so build the RGB and the mono mosaics separately.",
        colour.name, mono.name
    ))
}

fn normalized_unit(unit: &str) -> String {
    unit.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

fn is_surface_brightness(unit: &str) -> bool {
    let unit = normalized_unit(unit);
    SURFACE_BRIGHTNESS_MARKERS.iter().any(|marker| unit.contains(marker))
}

fn unit_label(tile: &TileGeometry) -> &str {
    tile.bunit.as_deref().unwrap_or("not set")
}

fn shared_unit(tiles: &[TileGeometry]) -> Result<Option<String>, String> {
    let Some(first) = tiles.first() else {
        return Ok(None);
    };
    let unit_of = |tile: &TileGeometry| tile.bunit.as_deref().map(normalized_unit);
    match tiles.iter().find(|t| unit_of(t) != unit_of(first)) {
        None => Ok(first.bunit.clone()),
        Some(other) => Err(format!(
            "Tiles '{}' and '{}' hold different units (BUNIT {} and {}): the mosaic blends their values as they are and is written without BUNIT.",
            first.name,
            other.name,
            unit_label(first),
            unit_label(other)
        )),
    }
}

fn unit_warnings(tiles: &[TileGeometry], rows: &[TileEstimate]) -> Vec<String> {
    let mut warnings: Vec<String> = shared_unit(tiles).err().into_iter().collect();
    let unscaled: Vec<String> = tiles
        .iter()
        .zip(rows)
        .filter(|(tile, row)| {
            (row.scale_ratio - 1.0).abs() > UNSCALED_SCALE_TOLERANCE && !tile.bunit.as_deref().is_some_and(is_surface_brightness)
        })
        .map(|(tile, row)| format!("'{}' ({:.3} times the canvas scale, BUNIT {})", tile.name, row.scale_ratio, unit_label(tile)))
        .collect();
    if !unscaled.is_empty() {
        let (subject, verb, owner) = if unscaled.len() == 1 { ("Tile", "has", "its") } else { ("Tiles", "have", "their") };
        warnings.push(format!(
            "{subject} {} {verb} a pixel scale more than {} % away from the canvas and a unit that is not a surface brightness: {owner} values are copied unscaled, so a value per pixel does not keep its flux on the canvas pixels.",
            unscaled.join(", "),
            UNSCALED_SCALE_TOLERANCE * 100.0
        ));
    }
    warnings
}

fn area_without(outlines: &[Outline], skipped: usize) -> f64 {
    let (mut min, mut max) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    for o in outlines.iter().enumerate().filter(|(k, _)| *k != skipped).map(|(_, o)| o) {
        min = (min.0.min(o.min.0), min.1.min(o.min.1));
        max = (max.0.max(o.max.0), max.1.max(o.max.1));
    }
    if min.0 > max.0 {
        0.0
    } else {
        (max.0 - min.0) * (max.1 - min.1)
    }
}

fn widest_tile(outlines: &[Outline]) -> usize {
    (0..outlines.len()).min_by(|&a, &b| area_without(outlines, a).total_cmp(&area_without(outlines, b))).unwrap_or(0)
}

fn three_windows_meet(windows: &[CanvasWindow]) -> bool {
    (0..windows.len()).any(|i| {
        (i + 1..windows.len()).any(|j| {
            overlap_of(&windows[i], &windows[j]).is_some_and(|pair| windows[j + 1..].iter().any(|w| overlap_of(&pair, w).is_some()))
        })
    })
}

fn canvas_planes(planes: u64, rejecting: bool) -> u64 {
    ACCUMULATOR_PLANES + planes + u64::from(rejecting)
}

fn mosaic_ram_bytes(canvas: &Canvas, planes: u64, tiles: &[TileGeometry], windows: &[CanvasWindow], rejecting: bool) -> u64 {
    let accumulators = canvas.pixels().saturating_mul(canvas_planes(planes, rejecting));
    let tile_planes = planes + if rejecting { REJECTION_TILE_PLANES } else { 0 };
    let in_flight = tiles
        .iter()
        .zip(windows)
        .map(|(tile, window)| {
            let tile_px = (tile.width as u64).saturating_mul(tile.height as u64);
            let window_px = (window.cols as u64).saturating_mul(window.rows as u64);
            tile_px.saturating_mul(tile_planes).saturating_add(window_px.saturating_mul(planes + 1))
        })
        .max()
        .unwrap_or(0);
    F32_BYTES.saturating_mul(accumulators.saturating_add(in_flight))
}

impl MosaicEstimate {
    fn refused(reason: String) -> Self {
        MosaicEstimate {
            cols: 0,
            rows: 0,
            megapixels: 0.0,
            ram_bytes: 0,
            canvas_cards: Vec::new(),
            tiles: Vec::new(),
            pairs: Vec::new(),
            components: Vec::new(),
            isolated: Vec::new(),
            warnings: Vec::new(),
            refusal: Some(reason),
            default_reject: None,
        }
    }
}

pub fn mosaic_estimate(tiles: &[TileGeometry], cfg: &MosaicConfig, mem: Option<MemoryInfo>) -> Result<MosaicEstimate> {
    check_request(tiles, cfg)?;
    let canvas = match build_canvas(tiles, &cfg.canvas) {
        Ok(canvas) => canvas,
        Err(reason) => return Ok(MosaicEstimate::refused(reason)),
    };
    let mut reasons = Vec::new();
    let over_tile_limit = tiles.len() > cfg.max_tiles;
    if over_tile_limit {
        reasons.push(format!(
            "The mosaic has {} tiles and takes at most {}: tile '{}' and the tiles after it are over the limit.",
            tiles.len(),
            cfg.max_tiles,
            tiles[cfg.max_tiles].name
        ));
    }
    reasons.extend(mixed_planes(tiles));
    let (pairs, components, isolated) = if over_tile_limit {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        let pairs = overlap_pairs(&canvas.outlines, cfg.min_overlap_frac);
        let components = assign_references(connected_groups(tiles.len(), &pairs), &cfg.references, tiles)?;
        let isolated: Vec<usize> =
            if tiles.len() < 2 { Vec::new() } else { components.iter().filter(|c| c.tiles.len() == 1).map(|c| c.tiles[0]).collect() };
        (pairs, components, isolated)
    };
    let windows: Vec<CanvasWindow> = canvas.outlines.iter().map(|o| canvas.window(o)).collect();
    let planes = tiles.iter().map(|t| t.planes as u64).max().unwrap_or(1);
    let can_reject = three_windows_meet(&windows);
    let rejecting = cfg.reject.is_some() && can_reject;
    let ram_bytes = mosaic_ram_bytes(&canvas, planes, tiles, &windows, rejecting);
    let canvas_px = canvas.pixels() as f64;
    let summed_area: f64 = canvas.outlines.iter().map(|o| o.area).sum();
    let culprit = &tiles[widest_tile(&canvas.outlines)].name;
    if canvas.cols > cfg.max_side_px || canvas.rows > cfg.max_side_px {
        reasons.push(format!(
            "The canvas would be {}x{} px, above the limit of {} px per side. Leaving out tile '{}' shrinks it the most: check its WCS.",
            canvas.cols, canvas.rows, cfg.max_side_px, culprit
        ));
    }
    if canvas_px > cfg.max_area_ratio * summed_area {
        reasons.push(format!(
            "The canvas would be {:.1} times the summed area of the tiles (the limit is {}). Leaving out tile '{}' shrinks it the most: check its WCS.",
            canvas_px / summed_area,
            cfg.max_area_ratio,
            culprit
        ));
    }
    let mut warnings = Vec::new();
    if ram_bytes > cfg.max_ram_bytes {
        reasons.push(format!(
            "The mosaic needs about {} of memory ({} planes per canvas pixel and one tile in flight), above its {} budget. Leaving out tile '{}' shrinks the canvas the most.",
            format_bytes(ram_bytes),
            canvas_planes(planes, rejecting),
            format_bytes(cfg.max_ram_bytes),
            culprit
        ));
    } else {
        match check_memory(ram_bytes, mem, &MemoryJob::Mosaic { cols: canvas.cols, rows: canvas.rows }) {
            Ok(warning) => warnings.extend(warning),
            Err(e) => reasons.push(format!("{e} Leaving out tile '{culprit}' shrinks the canvas the most.")),
        }
    }
    let canvas_scale = canvas.frame.pixel_scale_arcsec();
    let tile_rows: Vec<TileEstimate> = tiles
        .iter()
        .zip(&canvas.outlines)
        .zip(&windows)
        .map(|((tile, outline), window)| TileEstimate {
            name: tile.name.clone(),
            coverage_frac: outline.area / canvas_px,
            scale_ratio: tile.wcs.pixel_scale_arcsec() / canvas_scale,
            rotation_deg: tile_jacobian(tile, &canvas.frame).map_or(f64::NAN, |j| rotation_deg(&j)),
            wcs_kind: tile.wcs.kind(),
            window: *window,
        })
        .collect();
    warnings.extend(unit_warnings(tiles, &tile_rows));
    Ok(MosaicEstimate {
        cols: canvas.cols,
        rows: canvas.rows,
        megapixels: canvas_px / 1e6,
        ram_bytes,
        canvas_cards: canvas.header.cards.clone(),
        tiles: tile_rows,
        pairs,
        components,
        isolated,
        warnings,
        refusal: (!reasons.is_empty()).then(|| reasons.join(" ")),
        default_reject: default_rejection(tiles).filter(|_| can_reject),
    })
}

pub const LEVEL_DECIMATION: usize = 4;

const MIN_LEVEL_SAMPLES: usize = 16;
const MIN_SCALE_LAG_PAIRS: usize = 64;
const MAX_LOG_RATIO_SIGMA: f64 = 0.02;
const MIN_LOG_RATIO_SIGMA: f64 = 1e-6;
const SCALE_CLIP_SIGMA: f64 = 5.0;
const LAG_NEIGHBOURS: f64 = 4.0;
const MAD_TO_SIGMA: f64 = 1.4826;
const BICUBIC_REACH_PX: f64 = 2.0;
const PANEL_NOISE_REACH_PX: f64 = 2.0;

pub fn window_frame(canvas_cards: &[(String, String)], window: &CanvasWindow) -> Result<WcsTransform> {
    canvas_wcs(&window_cards(canvas_cards, window))
}

pub fn warp_to_window(plane: &Array2<f32>, tile: &TileGeometry, frame: &WcsTransform, window: &CanvasWindow) -> Array2<f32> {
    reproject_to_grid(plane, &tile.wcs, frame, window.rows, window.cols).0
}

struct BlockGrid {
    x0: usize,
    y0: usize,
    cols: usize,
    rows: usize,
}

fn block_span(start: usize, len: usize) -> (usize, usize) {
    let first = start / LEVEL_DECIMATION;
    match len {
        0 => (first, 0),
        _ => (first, (start + len - 1) / LEVEL_DECIMATION - first + 1),
    }
}

impl BlockGrid {
    fn of(window: &CanvasWindow) -> Self {
        let (x0, cols) = block_span(window.x0, window.cols);
        let (y0, rows) = block_span(window.y0, window.rows);
        BlockGrid { x0, y0, cols, rows }
    }

    fn block_of(&self, window: &CanvasWindow, (y, x): (usize, usize)) -> (usize, usize) {
        ((window.y0 + y) / LEVEL_DECIMATION - self.y0, (window.x0 + x) / LEVEL_DECIMATION - self.x0)
    }
}

fn block_means(warped: &Array2<f32>, window: &CanvasWindow, grid: &BlockGrid) -> Array2<f32> {
    let mut sums = Array2::<f64>::zeros((grid.rows, grid.cols));
    let mut counts = Array2::<usize>::zeros((grid.rows, grid.cols));
    for (pixel, &value) in warped.indexed_iter() {
        if value.is_finite() {
            let block = grid.block_of(window, pixel);
            sums[block] += value as f64;
            counts[block] += 1;
        }
    }
    let full = LEVEL_DECIMATION * LEVEL_DECIMATION;
    Array2::from_shape_fn((grid.rows, grid.cols), |b| if counts[b] == full { (sums[b] / full as f64) as f32 } else { f32::NAN })
}

fn star_blocks(luminance: &Array2<f32>, window: &CanvasWindow, grid: &BlockGrid) -> Result<Array2<bool>> {
    let config = StarMaskConfig::default();
    let stars = detect_star_extents(luminance, config.detection_sigma);
    let mask = generate_star_mask_from_detection(luminance, &stars, &config).map_err(anyhow::Error::msg)?.mask;
    let mut touched = Array2::from_elem((grid.rows, grid.cols), false);
    for (pixel, &weight) in mask.indexed_iter() {
        if weight > 0.0 {
            touched[grid.block_of(window, pixel)] = true;
        }
    }
    Ok(touched)
}

pub fn level_window(
    planes: &[Array2<f32>],
    tile: &TileGeometry,
    canvas_cards: &[(String, String)],
    window: &CanvasWindow,
) -> Result<LevelWindow> {
    warp_levels_and_validity(planes, tile, canvas_cards, window).map(|(level, _)| level)
}

fn warp_levels_and_validity(
    planes: &[Array2<f32>],
    tile: &TileGeometry,
    canvas_cards: &[(String, String)],
    window: &CanvasWindow,
) -> Result<(LevelWindow, Array2<bool>)> {
    if let Some(plane) = planes.iter().find(|p| p.dim() != (tile.height, tile.width)) {
        bail!("Tile '{}' has a {}x{} plane where its header gives {}x{}", tile.name, plane.ncols(), plane.nrows(), tile.width, tile.height);
    }
    let frame = window_frame(canvas_cards, window)?;
    let grid = BlockGrid::of(window);
    let mut means = Vec::with_capacity(planes.len());
    let mut luminance: Option<Array2<f32>> = None;
    for plane in planes {
        let warped = warp_to_window(plane, tile, &frame, window);
        means.push(block_means(&warped, window, &grid));
        luminance = Some(match luminance.take() {
            Some(sum) => sum + &warped,
            None => warped,
        });
    }
    let Some(luminance) = luminance else {
        bail!("Tile '{}' has no pixel planes", tile.name);
    };
    let filled = Array2::from_shape_fn((grid.rows, grid.cols), |block| means.iter().all(|m| m[block].is_finite()));
    let stars = star_blocks(&luminance, window, &grid)?;
    for (block, &star) in stars.indexed_iter() {
        if star || !filled[block] {
            means.iter_mut().for_each(|m| m[block] = f32::NAN);
        }
    }
    Ok((LevelWindow { block_x0: grid.x0, block_y0: grid.y0, planes: means }, filled))
}

pub fn level_windows(
    tiles: &[TileGeometry],
    estimate: &MosaicEstimate,
    load: &dyn Fn(usize) -> Result<Vec<Array2<f32>>>,
    cancel: CancelCheck,
    progress: &(dyn Fn(MosaicStage, usize, usize) + Sync),
) -> Result<Vec<LevelWindow>> {
    level_and_feather_windows(tiles, estimate, load, cancel, progress).map(|(windows, _)| windows)
}

fn level_and_feather_windows(
    tiles: &[TileGeometry],
    estimate: &MosaicEstimate,
    load: &dyn Fn(usize) -> Result<Vec<Array2<f32>>>,
    cancel: CancelCheck,
    progress: &(dyn Fn(MosaicStage, usize, usize) + Sync),
) -> Result<(Vec<LevelWindow>, Vec<FeatherMap>)> {
    if estimate.tiles.len() != tiles.len() {
        bail!("The estimate covers {} tiles and the mosaic has {}", estimate.tiles.len(), tiles.len());
    }
    let mut windows = Vec::with_capacity(tiles.len());
    let mut maps = Vec::with_capacity(tiles.len());
    for (k, (tile, row)) in tiles.iter().zip(&estimate.tiles).enumerate() {
        stop_if_cancelled(cancel)?;
        let planes = load(k).with_context(|| format!("Tile '{}' ({})", tile.name, tile.path))?;
        let (window, filled) = warp_levels_and_validity(&planes, tile, &estimate.canvas_cards, &row.window)?;
        maps.push(FeatherMap::of_blocks(&filled, window.block_x0, window.block_y0));
        windows.push(window);
        progress(MosaicStage::Overlaps, k + 1, tiles.len());
    }
    Ok((windows, maps))
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    i: usize,
    j: usize,
    value: f64,
    weight: f64,
}

fn offset_edges(stats: &[OverlapStat]) -> Vec<Edge> {
    stats.iter().map(|s| Edge { i: s.i, j: s.j, value: s.d, weight: s.w }).collect()
}

fn log_edges(stats: &[OverlapStat]) -> Vec<Edge> {
    stats.iter().map(|s| Edge { i: s.i, j: s.j, value: s.log_ratio, weight: s.log_w }).collect()
}

fn reached_from(reference: usize, members: &[usize], edges: &[Edge]) -> Vec<usize> {
    let mut reached = vec![reference];
    let mut next = 0;
    while let Some(&tile) = reached.get(next) {
        next += 1;
        for edge in edges {
            let other = match (edge.i == tile, edge.j == tile) {
                (true, _) => edge.j,
                (_, true) => edge.i,
                _ => continue,
            };
            if members.contains(&other) && !reached.contains(&other) {
                reached.push(other);
            }
        }
    }
    reached.sort_unstable();
    reached
}

fn solve_relative(n: usize, edges: &[Edge], components: &[Component]) -> (Vec<f64>, Vec<usize>) {
    let usable: Vec<Edge> = edges
        .iter()
        .copied()
        .filter(|e| e.i < n && e.j < n && e.i != e.j && e.value.is_finite() && e.weight.is_finite() && e.weight > 0.0)
        .collect();
    let mut values = vec![0.0; n];
    let mut unreached = Vec::new();
    for component in components {
        let reached = reached_from(component.reference, &component.tiles, &usable);
        unreached.extend(component.tiles.iter().copied().filter(|t| !reached.contains(t)));
        let unknowns: Vec<usize> = reached.iter().copied().filter(|&t| t != component.reference).collect();
        if unknowns.is_empty() {
            continue;
        }
        let row = |tile: usize| unknowns.iter().position(|&u| u == tile);
        let mut normal = vec![vec![0.0; unknowns.len()]; unknowns.len()];
        let mut rhs = vec![0.0; unknowns.len()];
        for edge in usable.iter().filter(|e| reached.contains(&e.i) && reached.contains(&e.j)) {
            let (ri, rj) = (row(edge.i), row(edge.j));
            if let Some(r) = ri {
                normal[r][r] += edge.weight;
                rhs[r] += edge.weight * edge.value;
            }
            if let Some(r) = rj {
                normal[r][r] += edge.weight;
                rhs[r] -= edge.weight * edge.value;
            }
            if let (Some(a), Some(b)) = (ri, rj) {
                normal[a][b] -= edge.weight;
                normal[b][a] -= edge.weight;
            }
        }
        match lu_solve(&mut normal, &mut rhs) {
            Some(()) => unknowns.iter().zip(&rhs).for_each(|(&tile, &value)| values[tile] = value),
            None => unreached.extend(unknowns),
        }
    }
    unreached.sort_unstable();
    unreached.dedup();
    (values, unreached)
}

pub fn solve_levels(n: usize, stats: &[OverlapStat], components: &[Component], model: LevelModel) -> LevelSolution {
    let (mut offsets, mut unmatched) = solve_relative(n, &offset_edges(stats), components);
    if n > 1 {
        unmatched.extend(components.iter().filter(|c| c.tiles.len() == 1).map(|c| c.tiles[0]));
        unmatched.sort_unstable();
        unmatched.dedup();
    }
    let (mut scales, unscaled) = match model {
        LevelModel::Offset => (vec![1.0; n], Vec::new()),
        LevelModel::OffsetScale => {
            let (logs, unreached) = solve_relative(n, &log_edges(stats), components);
            (logs.into_iter().map(f64::exp).collect(), unreached.into_iter().filter(|t| !unmatched.contains(t)).collect())
        }
    };
    for &tile in &unmatched {
        if let (Some(offset), Some(scale)) = (offsets.get_mut(tile), scales.get_mut(tile)) {
            (*offset, *scale) = (0.0, 1.0);
        }
    }
    LevelSolution { offsets, scales, unmatched, unscaled }
}

struct PairSamples {
    cols: usize,
    rows: usize,
    first: Vec<f64>,
    second: Vec<f64>,
}

fn pair_samples(a: &LevelWindow, b: &LevelWindow, plane: usize) -> Option<PairSamples> {
    let (pa, pb) = (a.planes.get(plane)?, b.planes.get(plane)?);
    let (x0, y0) = (a.block_x0.max(b.block_x0), a.block_y0.max(b.block_y0));
    let x1 = (a.block_x0 + pa.ncols()).min(b.block_x0 + pb.ncols());
    let y1 = (a.block_y0 + pa.nrows()).min(b.block_y0 + pb.nrows());
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let (cols, rows) = (x1 - x0, y1 - y0);
    let mut first = Vec::with_capacity(cols * rows);
    let mut second = Vec::with_capacity(cols * rows);
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = (pa[[y - a.block_y0, x - a.block_x0]] as f64, pb[[y - b.block_y0, x - b.block_x0]] as f64);
            let both = u.is_finite() && v.is_finite();
            first.push(if both { u } else { f64::NAN });
            second.push(if both { v } else { f64::NAN });
        }
    }
    Some(PairSamples { cols, rows, first, second })
}

fn level_difference(samples: &PairSamples, (scale_first, scale_second): (f64, f64)) -> Option<(f64, usize)> {
    let differences: Vec<f64> = samples
        .first
        .iter()
        .zip(&samples.second)
        .filter(|(u, _)| u.is_finite())
        .map(|(u, v)| u / scale_first - v / scale_second)
        .collect();
    (differences.len() >= MIN_LEVEL_SAMPLES).then(|| (exact_median_f64(&differences), differences.len()))
}

struct LaggedFit {
    mean_first: f64,
    mean_second: f64,
    ratio: f64,
    var_second: f64,
    cov_second: f64,
    pairs: usize,
}

impl LaggedFit {
    fn residual(&self, samples: &PairSamples, k: usize) -> f64 {
        (samples.first[k] - self.mean_first) - self.ratio * (samples.second[k] - self.mean_second)
    }

    fn residuals(&self, samples: &PairSamples, keep: &[bool]) -> Vec<f64> {
        (0..keep.len()).filter(|&k| keep[k]).map(|k| self.residual(samples, k)).collect()
    }
}

fn robust_centre_and_sigma(values: &[f64]) -> (f64, f64) {
    let centre = exact_median_f64(values);
    let deviations: Vec<f64> = values.iter().map(|v| (v - centre).abs()).collect();
    (centre, MAD_TO_SIGMA * exact_median_f64(&deviations))
}

fn lagged_fit(samples: &PairSamples, keep: &[bool], lag: usize) -> Option<LaggedFit> {
    let kept: Vec<usize> = (0..keep.len()).filter(|&k| keep[k]).collect();
    if kept.len() < 2 || lag == 0 {
        return None;
    }
    let n = kept.len() as f64;
    let mean = |values: &[f64]| kept.iter().map(|&k| values[k]).sum::<f64>() / n;
    let (mean_first, mean_second) = (mean(&samples.first), mean(&samples.second));
    let var_second = kept.iter().map(|&k| (samples.second[k] - mean_second).powi(2)).sum::<f64>() / n;
    let (mut cov_first, mut cov_second, mut pairs) = (0.0f64, 0.0f64, 0usize);
    for &k in &kept {
        let (x, y) = (k % samples.cols, k / samples.cols);
        let right = (lag < samples.cols - x).then(|| k + lag);
        let below = (lag < samples.rows - y).then(|| k + lag * samples.cols);
        for q in [right, below].into_iter().flatten().filter(|&q| keep[q]) {
            cov_first += (samples.first[k] - mean_first) * (samples.first[q] - mean_first);
            cov_second += (samples.second[k] - mean_second) * (samples.second[q] - mean_second);
            pairs += 1;
        }
    }
    if pairs < MIN_SCALE_LAG_PAIRS {
        return None;
    }
    let (cov_first, cov_second) = (cov_first / pairs as f64, cov_second / pairs as f64);
    if !(cov_first > 0.0 && cov_second > 0.0) {
        return None;
    }
    Some(LaggedFit { mean_first, mean_second, ratio: (cov_first / cov_second).sqrt(), var_second, cov_second, pairs })
}

fn log_scale_ratio(samples: &PairSamples, lag: usize) -> Option<(f64, f64)> {
    let mut keep: Vec<bool> = samples.first.iter().map(|v| v.is_finite()).collect();
    let provisional = lagged_fit(samples, &keep, lag)?;
    let (centre, spread) = robust_centre_and_sigma(&provisional.residuals(samples, &keep));
    if spread > 0.0 {
        for k in 0..keep.len() {
            if keep[k] && (provisional.residual(samples, k) - centre).abs() > SCALE_CLIP_SIGMA * spread {
                keep[k] = false;
            }
        }
    }
    let fit = lagged_fit(samples, &keep, lag)?;
    let (_, noise) = robust_centre_and_sigma(&fit.residuals(samples, &keep));
    let noise_in_second = noise * noise / (fit.ratio * fit.ratio);
    let leverage = 2.0 * LAG_NEIGHBOURS * fit.var_second * noise_in_second;
    let log_sigma = 0.5 * ((leverage + noise_in_second * noise_in_second) / (fit.pairs as f64 * fit.cov_second * fit.cov_second)).sqrt();
    (log_sigma <= MAX_LOG_RATIO_SIGMA).then(|| {
        let sigma = log_sigma.max(MIN_LOG_RATIO_SIGMA);
        (fit.ratio.ln(), 1.0 / (sigma * sigma))
    })
}

fn scale_lag(scale_ratio: f64) -> usize {
    let reach = scale_ratio.max(1.0) * (BICUBIC_REACH_PX + PANEL_NOISE_REACH_PX);
    let span = ((LEVEL_DECIMATION - 1) as f64 + 2.0 * reach) / LEVEL_DECIMATION as f64;
    (span.floor() as usize).saturating_add(1)
}

fn pair_lag(estimate: &MosaicEstimate, pair: &OverlapPair) -> usize {
    let ratio = |k: usize| estimate.tiles.get(k).map_or(1.0, |t| t.scale_ratio);
    scale_lag(ratio(pair.i).max(ratio(pair.j)))
}

fn level_warnings(tiles: &[TileGeometry], estimate: &MosaicEstimate, solutions: &[LevelSolution]) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let groups: Vec<&Component> = estimate.components.iter().filter(|c| c.tiles.len() > 1).collect();
    if groups.len() > 1 {
        let references: Vec<String> = groups.iter().map(|c| format!("'{}'", tiles[c.reference].name)).collect();
        warnings.push(format!(
            "The tiles form {} groups that do not overlap: levels are matched within each group against its reference ({}), not between the groups.",
            groups.len(),
            references.join(", ")
        ));
    }
    for (plane, solution) in solutions.iter().enumerate() {
        let in_plane = if solutions.len() > 1 { format!(" in plane {}", plane + 1) } else { String::new() };
        let unmatched = solution.unmatched.iter().map(|&k| {
            if estimate.isolated.contains(&k) {
                format!("Tile '{}' overlaps no other tile, so its level is left as it is (offset 0, scale 1).", tiles[k].name)
            } else {
                format!(
                    "Tile '{}' has no usable pixels where it overlaps the other tiles of its group (they are blank, masked by stars or too few), so its level is left as it is (offset 0, scale 1).",
                    tiles[k].name
                )
            }
        });
        let unscaled = solution.unscaled.iter().map(|&k| {
            format!(
                "The overlaps of tile '{}' hold too little structure{in_plane} to measure a scale, so its scale is left at 1 and only its offset is matched.",
                tiles[k].name
            )
        });
        for text in unmatched.chain(unscaled) {
            if !warnings.contains(&text) {
                warnings.push(text);
            }
        }
    }
    warnings
}

pub fn match_levels(tiles: &[TileGeometry], estimate: &MosaicEstimate, windows: &[LevelWindow], model: LevelModel) -> Result<LevelReport> {
    let n = tiles.len();
    if windows.len() != n || estimate.tiles.len() != n {
        bail!("Levels need one window and one estimate row per tile: {} tiles, {} windows, {} rows", n, windows.len(), estimate.tiles.len());
    }
    let planes = windows.first().map_or(0, |w| w.planes.len());
    if let Some(k) = windows.iter().position(|w| w.planes.len() != planes) {
        bail!("Tile '{}' has {} planes and tile '{}' has {}: a mosaic takes tiles of one kind", tiles[k].name, windows[k].planes.len(), tiles[0].name, planes);
    }
    let out_of_range = |k: usize| k >= n;
    if estimate.pairs.iter().any(|p| out_of_range(p.i) || out_of_range(p.j))
        || estimate.components.iter().any(|c| out_of_range(c.reference) || c.tiles.iter().copied().any(out_of_range))
    {
        bail!("The estimate names a tile beyond the {} tiles of the mosaic", n);
    }
    let mut solutions = Vec::with_capacity(planes);
    let mut stats = Vec::with_capacity(planes);
    for plane in 0..planes {
        let samples: Vec<Option<PairSamples>> = estimate.pairs.iter().map(|p| pair_samples(&windows[p.i], &windows[p.j], plane)).collect();
        let ratios: Vec<OverlapStat> = estimate
            .pairs
            .iter()
            .zip(&samples)
            .map(|(p, s)| {
                let measured = match model {
                    LevelModel::Offset => None,
                    LevelModel::OffsetScale => s.as_ref().and_then(|s| log_scale_ratio(s, pair_lag(estimate, p))),
                };
                let (log_ratio, log_w) = measured.unwrap_or((0.0, 0.0));
                OverlapStat { i: p.i, j: p.j, d: 0.0, w: 0.0, log_ratio, log_w }
            })
            .collect();
        let scales: Vec<f64> = match model {
            LevelModel::Offset => vec![1.0; n],
            LevelModel::OffsetScale => solve_relative(n, &log_edges(&ratios), &estimate.components).0.into_iter().map(f64::exp).collect(),
        };
        let plane_stats: Vec<OverlapStat> = ratios
            .iter()
            .zip(&samples)
            .map(|(r, s)| {
                let (d, count) = s.as_ref().and_then(|s| level_difference(s, (scales[r.i], scales[r.j]))).unwrap_or((0.0, 0));
                OverlapStat { d, w: count as f64, ..*r }
            })
            .collect();
        solutions.push(solve_levels(n, &plane_stats, &estimate.components, model));
        stats.push(plane_stats);
    }
    let warnings = level_warnings(tiles, estimate, &solutions);
    Ok(LevelReport { planes: solutions, stats, warnings })
}

const RESIDUAL_WARNING_PX: f64 = 0.5;
const RESIDUAL_CROP_PX: usize = 1536;
const RESIDUAL_SEEDS: [(f64, f64); 1] = [(0.0, 0.0)];
const UNSCALED_SCALE_TOLERANCE: f64 = 0.02;
const MIN_FEATHER_WEIGHT: f32 = 1e-6;
const REJECTION_COVERAGE: i32 = 3;
const REJECTION_TILE_PLANES: u64 = 3;
const COVERED_SHARE: f32 = 0.5;
const MIN_REJECTION_SAMPLES: usize = 256;
const REJECTION_SAMPLE_CAP: usize = 1 << 20;
const NOISE_LAG_PX: usize = 2;
const SECOND_DIFFERENCE_VARIANCE: f64 = 6.0;
const LEVEL_2_SUFFIXES: [&str; 4] = ["_cal", "_crf", "_flt", "_flc"];
const FILENAME_CARD: &str = "FILENAME";
const NO_TRIPLE_COVERAGE: &str =
    "Rejection needs 3 or more tiles over a pixel and no pixel of this mosaic has that many, so nothing was rejected.";
const SURFACE_BRIGHTNESS_MARKERS: [&str; 11] =
    ["/SR", "SR-1", "SR^-1", "SR**-1", "/STERADIAN", "/ARCSEC2", "/ARCSEC^2", "/ARCSEC**2", "ARCSEC-2", "ARCSEC^-2", "ARCSEC**-2"];

#[derive(Debug, Clone)]
pub struct TilePixels {
    pub planes: Vec<Array2<f32>>,
    pub excluded: Option<Array2<u8>>,
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string()).filter(|v| !v.is_empty())
}

fn smootherstep(t: f64) -> f64 {
    t * t * t * (t * (6.0 * t - 15.0) + 10.0)
}

fn feather_weight(distance: f64, other: f64, feather_px: f64) -> f64 {
    let reach = feather_px.min(distance + other);
    if reach > 0.0 {
        smootherstep((distance / reach).clamp(0.0, 1.0))
    } else {
        1.0
    }
}

fn border_connected(plane: &Array2<f32>, padded: impl Fn(f32) -> bool) -> Array2<bool> {
    let (rows, cols) = plane.dim();
    let mut padding = Array2::from_elem((rows, cols), false);
    if rows == 0 || cols == 0 {
        return padding;
    }
    let open = |padding: &Array2<bool>, y: usize, x: usize| padded(plane[[y, x]]) && !padding[[y, x]];
    let mut seeds: Vec<(usize, usize)> =
        (0..cols).flat_map(|x| [(0, x), (rows - 1, x)]).chain((0..rows).flat_map(|y| [(y, 0), (y, cols - 1)])).collect();
    while let Some((y, x)) = seeds.pop() {
        if !open(&padding, y, x) {
            continue;
        }
        let (mut left, mut right) = (x, x);
        while left > 0 && open(&padding, y, left - 1) {
            left -= 1;
        }
        while right + 1 < cols && open(&padding, y, right + 1) {
            right += 1;
        }
        padding.slice_mut(s![y, left..=right]).fill(true);
        for row in [y.checked_sub(1), (y + 1 < rows).then_some(y + 1)].into_iter().flatten() {
            let mut k = left;
            while k <= right {
                if open(&padding, row, k) {
                    seeds.push((row, k));
                    while k <= right && open(&padding, row, k) {
                        k += 1;
                    }
                } else {
                    k += 1;
                }
            }
        }
    }
    padding
}

fn tile_validity(planes: &[Array2<f32>], excluded: Option<&Array2<u8>>) -> Array2<bool> {
    let mut valid = Array2::from_elem(planes.first().map_or((0, 0), |p| p.dim()), true);
    for plane in planes {
        let padding = border_connected(plane, |value| value == 0.0);
        Zip::from(&mut valid).and(plane).and(&padding).for_each(|ok, &value, &pad| *ok &= value.is_finite() && !pad);
    }
    if let Some(excluded) = excluded {
        Zip::from(&mut valid).and(excluded).for_each(|ok, &flag| *ok &= flag == 0);
    }
    valid
}

fn masked_planes(pixels: TilePixels, tile: &TileGeometry) -> Result<Vec<Array2<f32>>> {
    let TilePixels { mut planes, excluded } = pixels;
    if planes.len() != tile.planes {
        bail!("Tile '{}' has {} pixel planes where its header gives {}", tile.name, planes.len(), tile.planes);
    }
    let shape = (tile.height, tile.width);
    if let Some(dim) = planes.iter().map(|p| p.dim()).chain(excluded.iter().map(|e| e.dim())).find(|&dim| dim != shape) {
        bail!("Tile '{}' has a {}x{} plane where its header gives {}x{}", tile.name, dim.1, dim.0, tile.width, tile.height);
    }
    let valid = tile_validity(&planes, excluded.as_ref());
    for plane in &mut planes {
        Zip::from(plane).and(&valid).par_for_each(|value, &ok| {
            if !ok {
                *value = f32::NAN;
            }
        });
    }
    Ok(planes)
}

fn rgb_planes(reference: &ImageRef) -> Result<Vec<Array2<f32>>> {
    let (source, _tmp) = resolve_single_image(&reference.path)?;
    let planes = if is_asdf_file(&source) {
        try_extract_rgb_from_asdf(&source)?.map(|rgb| vec![rgb.r, rgb.g, rgb.b])
    } else {
        let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
        try_extract_rgb_mmap(&file)?.map(|rgb| vec![rgb.r, rgb.g, rgb.b])
    };
    planes.with_context(|| format!("{} holds no RGB cube", reference.path))
}

fn dq_exclusion(loaded: &LoadedPlane) -> Option<Array2<u8>> {
    let dq_ref = loaded.companions.dq.as_ref().filter(|_| !loaded.info.is_dq)?;
    let dq = load_plane(dq_ref).map_err(|e| log::warn!("The DQ plane {} cannot be read: {e:#}", dq_ref.cache_key())).ok()?;
    let bits = dq.int_plane.as_ref().filter(|plane| plane.bits.dim() == loaded.arr.dim())?;
    let table = DqTable::select(&loaded.header).or_else(|| DqTable::select(&dq.header));
    Some(exclusion_map(&bits.bits, table.exclusion_mask()))
}

pub fn load_tile_pixels(tile: &TileGeometry, dq_mask: bool) -> Result<TilePixels> {
    let reference = ImageRef::parse(&tile.path);
    let context = || format!("Tile '{}': the pixels of {} cannot be read", tile.name, tile.path);
    if tile.planes > 1 {
        return Ok(TilePixels { planes: rgb_planes(&reference).with_context(context)?, excluded: None });
    }
    let loaded = load_plane(&reference).with_context(context)?;
    let excluded = dq_mask.then(|| dq_exclusion(&loaded)).flatten();
    Ok(TilePixels { planes: vec![loaded.arr], excluded })
}

fn lower_envelope(values: &[f64], out: &mut [f64]) {
    let mut vertices: Vec<usize> = Vec::with_capacity(values.len());
    let mut starts: Vec<f64> = Vec::with_capacity(values.len());
    for (q, &fq) in values.iter().enumerate() {
        if !fq.is_finite() {
            continue;
        }
        let qf = q as f64;
        let mut start = f64::NEG_INFINITY;
        while let (Some(&p), Some(&from)) = (vertices.last(), starts.last()) {
            let pf = p as f64;
            let crossing = ((fq + qf * qf) - (values[p] + pf * pf)) / (2.0 * (qf - pf));
            if crossing > from {
                start = crossing;
                break;
            }
            vertices.pop();
            starts.pop();
        }
        vertices.push(q);
        starts.push(start);
    }
    if vertices.is_empty() {
        out.fill(f64::INFINITY);
        return;
    }
    let mut k = 0;
    for (p, slot) in out.iter_mut().enumerate() {
        let pf = p as f64;
        while k + 1 < vertices.len() && starts[k + 1] < pf {
            k += 1;
        }
        let q = vertices[k];
        *slot = (pf - q as f64).powi(2) + values[q];
    }
}

fn squared_distance_to(features: &Array2<bool>) -> Array2<f64> {
    let mut grid = features.mapv(|feature| if feature { 0.0 } else { f64::INFINITY });
    for axis in [Axis(0), Axis(1)] {
        Zip::from(grid.lanes_mut(axis)).par_for_each(|mut lane| {
            let values = lane.to_vec();
            let mut envelope = vec![0.0; values.len()];
            lower_envelope(&values, &mut envelope);
            lane.iter_mut().zip(envelope).for_each(|(slot, value)| *slot = value);
        });
    }
    grid
}

#[derive(Debug, Clone)]
struct FeatherMap {
    origin: (f64, f64),
    signed: Array2<f32>,
}

impl FeatherMap {
    fn of_blocks(filled: &Array2<bool>, block_x0: usize, block_y0: usize) -> FeatherMap {
        let (rows, cols) = filled.dim();
        let padded =
            Array2::from_shape_fn((rows + 2, cols + 2), |(y, x)| (1..=rows).contains(&y) && (1..=cols).contains(&x) && filled[[y - 1, x - 1]]);
        let to_gap = squared_distance_to(&padded.mapv(|inside| !inside));
        let to_data = squared_distance_to(&padded);
        let block = LEVEL_DECIMATION as f64;
        let farthest = (rows + cols + 2) as f64;
        let signed = Array2::from_shape_fn(padded.dim(), |cell| {
            let cells = if padded[cell] { to_gap[cell].sqrt() } else { 1.0 - to_data[cell].sqrt().min(farthest) };
            (block * cells - (block - 1.0) / 2.0) as f32
        });
        let centre = |first: usize| (first as f64 - 1.0) * block + (block - 1.0) / 2.0;
        FeatherMap { origin: (centre(block_x0), centre(block_y0)), signed }
    }

    fn distance_at(&self, x: f64, y: f64) -> f64 {
        let (rows, cols) = self.signed.dim();
        let block = LEVEL_DECIMATION as f64;
        let u = ((x - self.origin.0) / block).clamp(0.0, (cols - 1) as f64);
        let v = ((y - self.origin.1) / block).clamp(0.0, (rows - 1) as f64);
        let (i0, j0) = (u.floor() as usize, v.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(cols - 1), (j0 + 1).min(rows - 1));
        let (fx, fy) = (u - i0 as f64, v - j0 as f64);
        let at = |j: usize, i: usize| self.signed[[j, i]] as f64;
        (1.0 - fy) * ((1.0 - fx) * at(j0, i0) + fx * at(j0, i1)) + fy * ((1.0 - fx) * at(j1, i0) + fx * at(j1, i1))
    }
}

fn feather_weights(
    maps: &[FeatherMap],
    tile: usize,
    neighbours: &[usize],
    window: &CanvasWindow,
    valid: &Array2<bool>,
    feather_px: f64,
) -> Array2<f32> {
    let mut weights = Array2::<f32>::zeros((window.rows, window.cols));
    Zip::indexed(&mut weights).and(valid).par_for_each(|(y, x), weight, &inside| {
        if inside {
            let (cx, cy) = ((window.x0 + x) as f64, (window.y0 + y) as f64);
            let own = maps[tile].distance_at(cx, cy).max(0.0);
            let other = neighbours.iter().map(|&j| maps[j].distance_at(cx, cy)).fold(0.0, f64::max);
            *weight = (feather_weight(own, other, feather_px) as f32).max(MIN_FEATHER_WEIGHT);
        }
    });
    weights
}

fn overlap_of(a: &CanvasWindow, b: &CanvasWindow) -> Option<CanvasWindow> {
    let (x0, y0) = (a.x0.max(b.x0), a.y0.max(b.y0));
    let (x1, y1) = ((a.x0 + a.cols).min(b.x0 + b.cols), (a.y0 + a.rows).min(b.y0 + b.rows));
    (x0 < x1 && y0 < y1).then(|| CanvasWindow { x0, y0, cols: x1 - x0, rows: y1 - y0 })
}

fn centred_crop(window: CanvasWindow, side: usize) -> CanvasWindow {
    let crop = |start: usize, len: usize| {
        let kept = len.min(side);
        (start + (len - kept) / 2, kept)
    };
    let ((x0, cols), (y0, rows)) = (crop(window.x0, window.cols), crop(window.y0, window.rows));
    CanvasWindow { x0, y0, cols, rows }
}

struct CanvasSums {
    sums: Vec<Array2<f32>>,
    weight: Array2<f32>,
    coverage: Array2<i32>,
}

impl CanvasSums {
    fn new(rows: usize, cols: usize, planes: usize) -> Self {
        CanvasSums {
            sums: (0..planes).map(|_| Array2::zeros((rows, cols))).collect(),
            weight: Array2::zeros((rows, cols)),
            coverage: Array2::zeros((rows, cols)),
        }
    }

    fn add(&mut self, window: &CanvasWindow, values: &[Array2<f32>], weights: &Array2<f32>) {
        let (y0, x0) = (window.y0, window.x0);
        let (y1, x1) = (y0 + window.rows, x0 + window.cols);
        for (sum, plane) in self.sums.iter_mut().zip(values) {
            Zip::from(sum.slice_mut(s![y0..y1, x0..x1])).and(plane).and(weights).par_for_each(|total, &value, &w| {
                if w > 0.0 {
                    *total += w * value;
                }
            });
        }
        Zip::from(self.weight.slice_mut(s![y0..y1, x0..x1])).and(self.coverage.slice_mut(s![y0..y1, x0..x1])).and(weights).par_for_each(
            |total, count, &w| {
                if w > 0.0 {
                    *total += w;
                    *count += 1;
                }
            },
        );
    }

    fn luminance(&self, y: usize, x: usize) -> f32 {
        self.sums.iter().map(|sum| sum[[y, x]] / self.weight[[y, x]]).sum()
    }

    fn finish(self) -> (Vec<Array2<f32>>, Array2<i32>) {
        let CanvasSums { mut sums, weight, coverage } = self;
        for sum in &mut sums {
            Zip::from(sum).and(&weight).par_for_each(|value, &w| *value = if w > 0.0 { *value / w } else { f32::NAN });
        }
        (sums, coverage)
    }
}

fn level_corrected(mut planes: Vec<Array2<f32>>, levels: &[LevelSolution], tile: usize) -> Vec<Array2<f32>> {
    for (plane, solution) in planes.iter_mut().zip(levels) {
        let (scale, offset) = (solution.scales[tile], solution.offsets[tile]);
        plane.par_mapv_inplace(|value| (value as f64 / scale - offset) as f32);
    }
    planes
}

fn finite_in_every_plane(planes: &[Array2<f32>], window: &CanvasWindow) -> Array2<bool> {
    let mut valid = Array2::from_elem((window.rows, window.cols), true);
    for plane in planes {
        Zip::from(&mut valid).and(plane).par_for_each(|ok, &value| *ok &= value.is_finite());
    }
    valid
}

fn pair_residual(
    canvas: &CanvasSums,
    earlier: &FeatherMap,
    overlap: &CanvasWindow,
    window: &CanvasWindow,
    values: &[Array2<f32>],
    valid: &Array2<bool>,
) -> Option<StarResidual> {
    let mut reference = Array2::from_elem((overlap.rows, overlap.cols), f32::NAN);
    let mut target = reference.clone();
    Zip::indexed(&mut reference).and(&mut target).par_for_each(|(y, x), earlier_value, later_value| {
        let (cy, cx) = (overlap.y0 + y, overlap.x0 + x);
        let (wy, wx) = (cy - window.y0, cx - window.x0);
        if canvas.coverage[[cy, cx]] == 1 && valid[[wy, wx]] && earlier.distance_at(cx as f64, cy as f64) > 0.0 {
            *earlier_value = canvas.luminance(cy, cx);
            *later_value = values.iter().map(|plane| plane[[wy, wx]]).sum();
        }
    });
    point_source_residual(&reference, &target, &RESIDUAL_SEEDS)
}

fn residual_warning(tiles: &[TileGeometry], residual: &PairResidual) -> Option<String> {
    let shift = residual.dx.hypot(residual.dy);
    (shift > RESIDUAL_WARNING_PX).then(|| {
        format!(
            "Tiles '{}' and '{}' disagree by {:.2} px on the positions of {} shared stars after the warp: check the WCS of both, for example by plate solving them again.",
            tiles[residual.i].name, tiles[residual.j].name, shift, residual.matched
        )
    })
}

fn output_cards(canvas_cards: &[(String, String)], bunit: Option<&str>) -> Result<Vec<(String, String)>> {
    let square_degrees = det(&canvas_wcs(canvas_cards)?.raw_params().4).abs();
    let mut header = header_from_cards(canvas_cards);
    header.set_f64("PIXAR_SR", square_degrees * 1f64.to_radians().powi(2));
    header.set_f64("PIXAR_A2", square_degrees * ARCSEC_PER_DEG * ARCSEC_PER_DEG);
    if let Some(unit) = bunit {
        header.set("BUNIT", format!("'{}'", unit.replace('\'', "''")));
    }
    Ok(header.cards)
}

fn file_name(path: &str) -> String {
    let source = ImageRef::parse(path).path;
    Path::new(&source).file_name().map_or_else(|| source.clone(), |name| name.to_string_lossy().into_owned())
}

fn wcs_origin(tile: &TileGeometry) -> String {
    let Some(solve) = &tile.solve else {
        return match tile.wcs.kind() {
            WcsKind::Header => "header WCS".to_string(),
            WcsKind::Gwcs => "gWCS".to_string(),
        };
    };
    let catalog = solve.catalog.as_deref().unwrap_or("an unnamed catalogue");
    let place = if catalog == GAIA_ONLINE_TIER.label { "online" } else { "offline" };
    format!(
        "WCS solved {place} from its header hints against {catalog}, {} stars, rms {:.3} px",
        solve.matched.unwrap_or(0),
        solve.rms_px.unwrap_or(f64::NAN)
    )
}

fn group_history(tiles: &[TileGeometry], components: &[Component]) -> Vec<String> {
    components
        .iter()
        .enumerate()
        .map(|(g, component)| {
            let members: Vec<String> = component.tiles.iter().map(|t| (t + 1).to_string()).collect();
            let noun = if members.len() == 1 { "tile" } else { "tiles" };
            let reference = component.reference;
            format!("Group {} of {noun} {}: reference tile {} '{}'", g + 1, members.join(", "), reference + 1, tiles[reference].name)
        })
        .collect()
}

fn mosaic_history(
    tiles: &[TileGeometry],
    estimate: &MosaicEstimate,
    levels: &[LevelSolution],
    left_out: &[u64],
    cfg: &MosaicConfig,
) -> Vec<String> {
    let model = match cfg.levels {
        LevelModel::Offset => "offsets",
        LevelModel::OffsetScale => "offsets and scales",
    };
    let mut history = vec![format!("Mosaic of {} tiles on a {}x{} canvas", tiles.len(), estimate.cols, estimate.rows)];
    for (k, tile) in tiles.iter().enumerate() {
        history.push(format!("Tile {} '{}' from {}: {}", k + 1, tile.name, file_name(&tile.path), wcs_origin(tile)));
    }
    history.extend([
        "Each tile reprojected once with bicubic interpolation, values unscaled".to_string(),
        format!("Feather {} px smootherstep, weights normalised per pixel", cfg.feather_px),
        if cfg.dq_mask { "DQ flags applied: pixels flagged in a tile's DQ plane are left out" } else { "DQ flags not applied" }.to_string(),
        format!("Levels matched as {model} against each group reference"),
    ]);
    history.extend(group_history(tiles, &estimate.components));
    for (k, tile) in tiles.iter().enumerate() {
        for (c, solution) in levels.iter().enumerate() {
            let plane = if levels.len() > 1 { format!(" plane {}", c + 1) } else { String::new() };
            history.push(format!("Tile {} '{}'{plane}: offset {:.6E} scale {:.6}", k + 1, tile.name, solution.offsets[k], solution.scales[k]));
        }
    }
    let causes = if cfg.dq_mask { "NaN, border zero padding or DQ flags" } else { "NaN or border zero padding" };
    for ((k, tile), left) in tiles.iter().enumerate().zip(left_out) {
        let pixels = (tile.width as u64).saturating_mul(tile.height as u64);
        history.push(format!("Tile {} '{}': {left} of {pixels} px left out ({causes})", k + 1, tile.name));
    }
    history
}

fn rejection_history(tiles: &[TileGeometry], spec: Option<&RejectSpec>, rejections: &[TileRejection]) -> Vec<String> {
    let Some(spec) = spec else {
        return vec!["Outlier rejection off".to_string()];
    };
    let mut history = vec![format!("Outliers rejected where {REJECTION_COVERAGE} or more tiles overlap, snr {} scale {}", spec.snr, spec.scale)];
    history.extend(tiles.iter().zip(rejections).enumerate().map(|(k, (tile, r))| match r.tested_px {
        0 => format!("Tile {} '{}': no pixel tested against {REJECTION_COVERAGE} or more tiles", k + 1, tile.name),
        tested => format!("Tile {} '{}': {} of {tested} px rejected, noise {:.4E}", k + 1, tile.name, r.rejected_px, r.sigma),
    }));
    history
}

impl TileRejection {
    fn untested() -> Self {
        TileRejection { tested_px: 0, rejected_px: 0, sigma: f64::NAN, pixels: Vec::new() }
    }
}

struct OutlierSums {
    sum: Array2<f32>,
    low: Array2<f32>,
    high: Array2<f32>,
    count: Array2<i32>,
}

impl OutlierSums {
    fn new(rows: usize, cols: usize) -> Self {
        OutlierSums {
            sum: Array2::zeros((rows, cols)),
            low: Array2::from_elem((rows, cols), f32::INFINITY),
            high: Array2::from_elem((rows, cols), f32::NEG_INFINITY),
            count: Array2::zeros((rows, cols)),
        }
    }

    fn add(&mut self, window: &CanvasWindow, luminance: &Array2<f32>) {
        let (y0, x0) = (window.y0, window.x0);
        let (y1, x1) = (y0 + window.rows, x0 + window.cols);
        Zip::from(self.sum.slice_mut(s![y0..y1, x0..x1]))
            .and(self.low.slice_mut(s![y0..y1, x0..x1]))
            .and(self.high.slice_mut(s![y0..y1, x0..x1]))
            .and(self.count.slice_mut(s![y0..y1, x0..x1]))
            .and(luminance)
            .par_for_each(|sum, low, high, count, &value| {
                if value.is_finite() {
                    *sum += value;
                    *low = low.min(value);
                    *high = high.max(value);
                    *count += 1;
                }
            });
    }

    fn trimmed_mean(self) -> Option<Array2<f32>> {
        let OutlierSums { mut sum, low, high, count } = self;
        Zip::from(&mut sum).and(&low).and(&high).and(&count).par_for_each(|value, &lowest, &highest, &covering| {
            *value = if covering >= REJECTION_COVERAGE { (*value - lowest - highest) / (covering - 2) as f32 } else { f32::NAN };
        });
        sum.iter().any(|v| v.is_finite()).then_some(sum)
    }
}

fn corrected_luminance(planes: &[Array2<f32>], levels: &[LevelSolution], tile: usize) -> Array2<f32> {
    let mut luminance = Array2::<f32>::zeros(planes.first().map_or((0, 0), |p| p.dim()));
    for (plane, solution) in planes.iter().zip(levels) {
        let (scale, offset) = (solution.scales[tile], solution.offsets[tile]);
        Zip::from(&mut luminance).and(plane).par_for_each(|sum, &value| *sum += (value as f64 / scale - offset) as f32);
    }
    luminance
}

fn outlier_reference(
    tiles: &[TileGeometry],
    estimate: &MosaicEstimate,
    levels: &[LevelSolution],
    masked: &dyn Fn(usize) -> Result<Vec<Array2<f32>>>,
    cancel: CancelCheck,
    progress: &(dyn Fn(MosaicStage, usize, usize) + Sync),
) -> Result<Option<Array2<f32>>> {
    let mut sums = OutlierSums::new(estimate.rows, estimate.cols);
    for (k, (tile, row)) in tiles.iter().zip(&estimate.tiles).enumerate() {
        stop_if_cancelled(cancel)?;
        let planes = masked(k).with_context(|| format!("Tile '{}' ({})", tile.name, tile.path))?;
        let luminance = corrected_luminance(&planes, levels, k);
        drop(planes);
        let frame = window_frame(&estimate.canvas_cards, &row.window)?;
        sums.add(&row.window, &warp_to_window(&luminance, tile, &frame, &row.window));
        progress(MosaicStage::Reject, k + 1, tiles.len());
    }
    Ok(sums.trimmed_mean())
}

fn reference_slope(blotted: &Array2<f32>, y: usize, x: usize) -> f64 {
    let (rows, cols) = blotted.dim();
    let centre = f64::from(blotted[[y, x]]);
    let step = |pixel: (usize, usize)| {
        let value = f64::from(blotted[pixel]);
        if value.is_finite() {
            (value - centre).abs()
        } else {
            0.0
        }
    };
    let steepest = |neighbours: [Option<(usize, usize)>; 2]| neighbours.into_iter().flatten().map(step).fold(0.0, f64::max);
    let along_x = steepest([x.checked_sub(1).map(|left| (y, left)), (x + 1 < cols).then_some((y, x + 1))]);
    let along_y = steepest([y.checked_sub(1).map(|up| (up, x)), (y + 1 < rows).then_some((y + 1, x))]);
    along_x.hypot(along_y)
}

fn residual_centre_and_noise(luminance: &Array2<f32>, blotted: &Array2<f32>, testable: &Array2<bool>, tested: usize) -> Option<(f64, f64)> {
    let stride = tested.div_ceil(REJECTION_SAMPLE_CAP).max(1);
    let cols = luminance.ncols();
    let mut residuals = Vec::with_capacity(tested / stride + 1);
    let mut curvatures = Vec::with_capacity(tested / stride + 1);
    for ((y, x), _) in testable.indexed_iter().filter(|(_, ok)| **ok).step_by(stride) {
        let value = f64::from(luminance[[y, x]]);
        residuals.push(value - f64::from(blotted[[y, x]]));
        if x >= NOISE_LAG_PX && x + NOISE_LAG_PX < cols {
            let (left, right) = (luminance[[y, x - NOISE_LAG_PX]], luminance[[y, x + NOISE_LAG_PX]]);
            if left.is_finite() && right.is_finite() {
                curvatures.push(f64::from(left) + f64::from(right) - 2.0 * value);
            }
        }
    }
    if curvatures.len() < MIN_REJECTION_SAMPLES {
        return None;
    }
    let sigma = robust_centre_and_sigma(&curvatures).1 / SECOND_DIFFERENCE_VARIANCE.sqrt();
    (sigma.is_finite() && sigma > 0.0).then(|| (exact_median_f64(&residuals), sigma))
}

fn tile_rejection(
    luminance: &Array2<f32>,
    reference: &Array2<f32>,
    tile: &TileGeometry,
    frame: &WcsTransform,
    window: &CanvasWindow,
    spec: &RejectSpec,
) -> TileRejection {
    let crop = reference.slice(s![window.y0..window.y0 + window.rows, window.x0..window.x0 + window.cols]).to_owned();
    if !crop.iter().any(|v| v.is_finite()) {
        return TileRejection::untested();
    }
    let covered = crop.mapv(|v| if v.is_finite() { 1.0f32 } else { 0.0 });
    let share = reproject_to_grid(&covered, frame, &tile.wcs, tile.height, tile.width).0;
    drop(covered);
    let blotted = reproject_to_grid(&crop, frame, &tile.wcs, tile.height, tile.width).0;
    drop(crop);
    let testable =
        Zip::from(&share).and(&blotted).and(luminance).par_map_collect(|&s, &b, &v| s > COVERED_SHARE && b.is_finite() && v.is_finite());
    drop(share);
    let tested = testable.iter().filter(|&&ok| ok).count();
    let Some((centre, sigma)) =
        (tested >= MIN_REJECTION_SAMPLES).then(|| residual_centre_and_noise(luminance, &blotted, &testable, tested)).flatten()
    else {
        return TileRejection::untested();
    };
    let (snr, scale) = (f64::from(spec.snr), f64::from(spec.scale));
    let flagged = Zip::indexed(&testable).par_map_collect(|(y, x), &ok| {
        ok && f64::from(luminance[[y, x]]) - f64::from(blotted[[y, x]]) - centre > snr * sigma + scale * reference_slope(&blotted, y, x)
    });
    let pixels: Vec<(usize, usize)> = flagged.indexed_iter().filter(|(_, hit)| **hit).map(|(pixel, _)| pixel).collect();
    TileRejection { tested_px: tested as u64, rejected_px: pixels.len() as u64, sigma, pixels }
}

fn exclude_pixels(planes: &mut [Array2<f32>], pixels: &[(usize, usize)]) {
    for plane in planes {
        for &pixel in pixels {
            plane[pixel] = f32::NAN;
        }
    }
}

fn footprint_share(planes: &[Array2<f32>], tile: &TileGeometry, frame: &WcsTransform, window: &CanvasWindow) -> Option<Array2<f32>> {
    let padding = border_connected(planes.first()?, |value| !value.is_finite());
    if !padding.iter().any(|&pad| pad) {
        return None;
    }
    let inside = padding.mapv(|pad| if pad { 0.0f32 } else { 1.0 });
    drop(padding);
    Some(warp_to_window(&inside, tile, frame, window))
}

fn keep_inside_footprint(valid: &mut Array2<bool>, share: &Array2<f32>) {
    Zip::from(valid).and(share).par_for_each(|ok, &s| *ok &= s > COVERED_SHARE);
}

pub fn run_mosaic_with(
    tiles: &[TileGeometry],
    cfg: &MosaicConfig,
    load: &dyn Fn(usize) -> Result<TilePixels>,
    mem: Option<MemoryInfo>,
    cancel: CancelCheck,
    progress: &(dyn Fn(MosaicStage, usize, usize) + Sync),
) -> Result<MosaicOutput> {
    let feather_px = f64::from(cfg.feather_px);
    if !(feather_px.is_finite() && feather_px >= 0.0) {
        bail!("The feather is {} px: it takes a width of 0 px or more", cfg.feather_px);
    }
    if let Some(spec) = cfg.reject.filter(|s| !(s.snr.is_finite() && s.snr > 0.0 && s.scale.is_finite() && s.scale >= 0.0)) {
        bail!("Rejection takes an snr above 0 and a gradient scale of 0 or more, not snr {} and scale {}", spec.snr, spec.scale);
    }
    let estimate = mosaic_estimate(tiles, cfg, mem)?;
    if let Some(reason) = &estimate.refusal {
        bail!("{reason}");
    }
    let masked = |k: usize| -> Result<Vec<Array2<f32>>> { masked_planes(load(k)?, &tiles[k]) };
    let (level_windows, maps) = level_and_feather_windows(tiles, &estimate, &masked, cancel, progress)?;
    let levels = match_levels(tiles, &estimate, &level_windows, cfg.levels)?;
    drop(level_windows);
    progress(MosaicStage::Levels, 1, 1);

    let mut warnings = estimate.warnings.clone();
    warnings.extend(levels.warnings.iter().cloned());
    let windows: Vec<CanvasWindow> = estimate.tiles.iter().map(|row| row.window).collect();
    let reference = match cfg.reject {
        Some(_) if three_windows_meet(&windows) => outlier_reference(tiles, &estimate, &levels.planes, &masked, cancel, progress)?,
        _ => None,
    };
    if cfg.reject.is_some() && reference.is_none() {
        warnings.push(NO_TRIPLE_COVERAGE.to_string());
    }
    let mut canvas = CanvasSums::new(estimate.rows, estimate.cols, tiles[0].planes);
    let mut residuals = Vec::with_capacity(estimate.pairs.len());
    let mut rejections = Vec::new();
    let mut left_out = Vec::with_capacity(tiles.len());
    for (k, (tile, row)) in tiles.iter().zip(&estimate.tiles).enumerate() {
        stop_if_cancelled(cancel)?;
        let window = row.window;
        let mut planes = masked(k).with_context(|| format!("Tile '{}' ({})", tile.name, tile.path))?;
        left_out.push(planes.first().map_or(0, |plane| plane.iter().filter(|v| !v.is_finite()).count() as u64));
        let frame = window_frame(&estimate.canvas_cards, &window)?;
        let footprint = footprint_share(&planes, tile, &frame, &window);
        if let Some(spec) = &cfg.reject {
            let rejection = match &reference {
                Some(reference) => tile_rejection(&corrected_luminance(&planes, &levels.planes, k), reference, tile, &frame, &window, spec),
                None => TileRejection::untested(),
            };
            exclude_pixels(&mut planes, &rejection.pixels);
            rejections.push(rejection);
        }
        let warped: Vec<Array2<f32>> = planes.iter().map(|plane| warp_to_window(plane, tile, &frame, &window)).collect();
        drop(planes);
        let values = level_corrected(warped, &levels.planes, k);
        let mut valid = finite_in_every_plane(&values, &window);
        if let Some(share) = footprint {
            keep_inside_footprint(&mut valid, &share);
        }
        if !valid.iter().any(|&ok| ok) {
            warnings.push(format!(
                "Tile '{}' has no valid pixels on the canvas (they are NaN, zero padding or flagged in its DQ plane), so it adds nothing to the mosaic.",
                tile.name
            ));
        }
        for pair in estimate.pairs.iter().filter(|p| p.j == k) {
            let measured = overlap_of(&estimate.tiles[pair.i].window, &window).and_then(|overlap| {
                pair_residual(&canvas, &maps[pair.i], &centred_crop(overlap, RESIDUAL_CROP_PX), &window, &values, &valid)
            });
            residuals.push(PairResidual {
                i: pair.i,
                j: k,
                dx: measured.map_or(f64::NAN, |r| r.dx),
                dy: measured.map_or(f64::NAN, |r| r.dy),
                matched: measured.map_or(0, |r| r.agreeing),
                rms_px: measured.map_or(f64::NAN, |r| r.rms_px),
            });
            progress(MosaicStage::Residuals, residuals.len(), estimate.pairs.len());
        }
        let neighbours: Vec<usize> =
            (0..tiles.len()).filter(|&j| j != k && overlap_of(&estimate.tiles[j].window, &window).is_some()).collect();
        let weights = feather_weights(&maps, k, &neighbours, &window, &valid, feather_px);
        canvas.add(&window, &values, &weights);
        progress(MosaicStage::Warp, k + 1, tiles.len());
    }
    residuals.sort_by_key(|r| (r.i, r.j));
    warnings.extend(residuals.iter().filter_map(|r| residual_warning(tiles, r)));
    let (planes, coverage) = canvas.finish();
    let cards = output_cards(&estimate.canvas_cards, shared_unit(tiles).ok().flatten().as_deref())?;
    let mut history = mosaic_history(tiles, &estimate, &levels.planes, &left_out, cfg);
    history.extend(rejection_history(tiles, cfg.reject.as_ref(), &rejections));
    Ok(MosaicOutput {
        planes,
        coverage,
        cards,
        history,
        levels: levels.planes,
        pair_residuals: residuals,
        rejected_px: rejections.iter().map(|r| r.rejected_px).sum(),
        rejections,
        warnings,
    })
}

pub fn run_mosaic(
    tiles: &[TileGeometry],
    cfg: &MosaicConfig,
    cancel: CancelCheck,
    progress: &(dyn Fn(MosaicStage, usize, usize) + Sync),
) -> Result<MosaicOutput> {
    let load = |k: usize| load_tile_pixels(&tiles[k], cfg.dq_mask);
    run_mosaic_with(tiles, cfg, &load, effective_memory(), cancel, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::wcs::{pixel_center, pixel_edge_corners, read_effective_cd};
    use crate::core::stacking::never_cancelled;
    use crate::core::synth::psf::{render_stars, GaussianPsf};
    use crate::core::synth::star_field::Star;
    use crate::infra::fits::writer::{write_fits_mono, write_fits_rgb, write_mef_images, HduData, ImageHdu};
    use ndarray::s;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn header(cards: &[(&str, String)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, value.clone());
        }
        h
    }

    fn tan_cards(width: usize, height: usize, crpix: (f64, f64), crval: (f64, f64), cd: [[f64; 2]; 2]) -> Vec<(&'static str, String)> {
        vec![
            ("NAXIS1", width.to_string()),
            ("NAXIS2", height.to_string()),
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", crpix.0.to_string()),
            ("CRPIX2", crpix.1.to_string()),
            ("CRVAL1", crval.0.to_string()),
            ("CRVAL2", crval.1.to_string()),
            ("CD1_1", cd[0][0].to_string()),
            ("CD1_2", cd[0][1].to_string()),
            ("CD2_1", cd[1][0].to_string()),
            ("CD2_2", cd[1][1].to_string()),
        ]
    }

    fn rotated_cd(scale_arcsec: f64, rotation_deg: f64) -> [[f64; 2]; 2] {
        let s = scale_arcsec / 3600.0;
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        [[-s * cos, -s * sin], [-s * sin, s * cos]]
    }

    fn tile(name: &str, cards: &[(&str, String)]) -> TileGeometry {
        let header = header(cards);
        TileGeometry {
            name: name.to_string(),
            path: format!("{name}.fits"),
            width: header.get_i64("NAXIS1").unwrap() as usize,
            height: header.get_i64("NAXIS2").unwrap() as usize,
            planes: 1,
            wcs: Arc::new(WcsTransform::from_header(&header).unwrap()),
            bunit: card_text(&header, "BUNIT"),
            single_exposure: false,
            solve: None,
        }
    }

    fn centred_tile(name: &str, width: usize, height: usize, crval: (f64, f64), scale_arcsec: f64, rotation_deg: f64) -> TileGeometry {
        let crpix = ((width as f64 + 1.0) / 2.0, (height as f64 + 1.0) / 2.0);
        tile(name, &tan_cards(width, height, crpix, crval, rotated_cd(scale_arcsec, rotation_deg)))
    }

    fn grid_window(name: &str, x0: f64, y0: f64, width: usize, height: usize) -> TileGeometry {
        tile(name, &tan_cards(width, height, (1.0 - x0, 1.0 - y0), (150.0, 30.0), rotated_cd(1.0, 0.0)))
    }

    fn edge_points(width: usize, height: usize) -> Vec<(f64, f64)> {
        let corners = pixel_edge_corners(width, height);
        let mut points = Vec::new();
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            for m in 0..64 {
                let t = m as f64 / 64.0;
                points.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        points
    }

    fn on_frame(tile: &TileGeometry, frame: &WcsTransform, points: &[(f64, f64)]) -> Vec<(f64, f64)> {
        points
            .iter()
            .map(|&(x, y)| {
                let sky = tile.wcs.pixel_to_world(x, y);
                frame.world_to_pixel(sky.ra, sky.dec)
            })
            .collect()
    }

    fn extent(points: &[(f64, f64)]) -> ((f64, f64), (f64, f64)) {
        points.iter().fold(
            ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY)),
            |((x0, y0), (x1, y1)), &(x, y)| ((x0.min(x), y0.min(y)), (x1.max(x), y1.max(y))),
        )
    }

    fn assert_tight(label: &str, min: f64, max: f64, first: usize, len: usize) {
        let (lo, hi) = (first as f64 - 0.5, (first + len) as f64 - 0.5);
        assert!(min >= lo - 1e-6 && min < lo + 1.0, "{label}: lowest sample {min} against the window edge {lo}");
        assert!(max <= hi + 1e-6 && max > hi - 1.0, "{label}: highest sample {max} against the window edge {hi}");
    }

    fn expected_ram(estimate: &MosaicEstimate, tiles: &[TileGeometry]) -> u64 {
        let planes = tiles.iter().map(|t| t.planes as u64).max().unwrap();
        let canvas = (estimate.cols * estimate.rows) as u64 * (2 + planes);
        let in_flight = tiles
            .iter()
            .zip(&estimate.tiles)
            .map(|(t, row)| (t.width * t.height) as u64 * planes + (row.window.cols * row.window.rows) as u64 * (planes + 1))
            .max()
            .unwrap();
        4 * (canvas + in_flight)
    }

    fn with_canvas(canvas: CanvasSpec) -> MosaicConfig {
        MosaicConfig { canvas, ..MosaicConfig::default() }
    }

    #[test]
    fn canvas_bbox_matches_tile_edge_samples() {
        let k = (-2e-8f64).to_string();
        let mut barrel = tan_cards(1000, 1000, (500.5, 500.5), (150.25, 30.05), rotated_cd(1.0, 90.0));
        barrel[2].1 = "'RA---TAN-SIP'".into();
        barrel[3].1 = "'DEC--TAN-SIP'".into();
        barrel.extend([
            ("A_ORDER", "3".to_string()),
            ("A_3_0", k.clone()),
            ("A_1_2", k.clone()),
            ("B_ORDER", "3".to_string()),
            ("B_0_3", k.clone()),
            ("B_2_1", k),
        ]);
        let tiles = vec![
            centred_tile("plain", 1200, 800, (150.0, 30.0), 1.0, 7.0),
            tile("barrel", &barrel),
            centred_tile("coarse", 900, 900, (150.1, 29.85), 1.01, 0.0),
        ];
        let aligned_to_barrel = with_canvas(CanvasSpec::Auto { north_up: false, reference: Some(1) });
        let estimate = mosaic_estimate(&tiles, &aligned_to_barrel, None).unwrap();
        assert_eq!(estimate.refusal, None);
        assert_eq!(estimate.tiles.len(), 3);
        assert!(estimate.tiles[1].rotation_deg.abs() < 1e-6, "{}", estimate.tiles[1].rotation_deg);
        let canvas = canvas_wcs(&estimate.canvas_cards).unwrap();
        let mut everything = Vec::new();
        for (tile, row) in tiles.iter().zip(&estimate.tiles) {
            let samples = on_frame(tile, &canvas, &edge_points(tile.width, tile.height));
            let ((min_x, min_y), (max_x, max_y)) = extent(&samples);
            let w = row.window;
            assert_tight(&format!("{} x", tile.name), min_x, max_x, w.x0, w.cols);
            assert_tight(&format!("{} y", tile.name), min_y, max_y, w.y0, w.rows);
            let window = canvas_wcs(&window_cards(&estimate.canvas_cards, &w)).unwrap();
            let local = on_frame(tile, &window, &edge_points(tile.width, tile.height));
            for (a, b) in samples.iter().zip(&local) {
                assert!((a.0 - w.x0 as f64 - b.0).abs() < 1e-6 && (a.1 - w.y0 as f64 - b.1).abs() < 1e-6, "{}: {a:?} {b:?}", tile.name);
            }
            everything.extend(samples);
        }
        let ((min_x, min_y), (max_x, max_y)) = extent(&everything);
        assert_tight("canvas x", min_x, max_x, 0, estimate.cols);
        assert_tight("canvas y", min_y, max_y, 0, estimate.rows);
        let corners = extent(&on_frame(&tiles[1], &canvas, &pixel_edge_corners(1000, 1000)));
        let edges = extent(&on_frame(&tiles[1], &canvas, &edge_points(1000, 1000)));
        let bulge_x = (edges.1 .0 - edges.0 .0) - (corners.1 .0 - corners.0 .0);
        let bulge_y = (edges.1 .1 - edges.0 .1) - (corners.1 .1 - corners.0 .1);
        assert!(bulge_x > 4.0 && bulge_y > 4.0, "the barrel tile bulges {bulge_x} x {bulge_y} px beyond its corners");
        assert_eq!(estimate.megapixels, (estimate.cols * estimate.rows) as f64 / 1e6);
    }

    #[test]
    fn refuses_canvas_over_40000_px_or_3x_tile_area_naming_the_tile() {
        let mut tiles = vec![
            grid_window("a", 0.0, 0.0, 1000, 1000),
            grid_window("b", 900.0, 0.0, 1000, 1000),
            grid_window("c", 0.0, 900.0, 1000, 1000),
        ];
        assert_eq!(mosaic_estimate(&tiles, &MosaicConfig::default(), None).unwrap().refusal, None);

        tiles.push(centred_tile("stray", 1000, 1000, (165.0, 30.0), 1.0, 0.0));
        let far = mosaic_estimate(&tiles, &MosaicConfig::default(), None).unwrap();
        assert!(far.cols > 40_000 && far.rows < 40_000, "{} x {}", far.cols, far.rows);
        let refusal = far.refusal.expect("a tile 15 degrees away stretches the canvas past 40000 px");
        assert!(refusal.contains("40000 px per side") && refusal.contains("'stray'"), "{refusal}");

        let sparse = vec![
            grid_window("a", 0.0, 0.0, 1000, 1000),
            grid_window("b", 900.0, 0.0, 1000, 1000),
            grid_window("corner", 5000.0, 5000.0, 1000, 1000),
        ];
        let est = mosaic_estimate(&sparse, &MosaicConfig::default(), None).unwrap();
        assert!(est.cols < 40_000 && est.rows < 40_000);
        let refusal = est.refusal.expect("a corner tile leaves most of the canvas empty");
        assert!(refusal.contains("times the summed area") && refusal.contains("'corner'"), "{refusal}");
        assert!(!refusal.contains("px per side"), "{refusal}");

        let opposite = vec![
            centred_tile("east", 500, 500, (10.0, 0.0), 1.0, 0.0),
            centred_tile("near", 500, 500, (10.1, 0.0), 1.0, 0.0),
            centred_tile("antipode", 500, 500, (160.0, 0.0), 1.0, 0.0),
        ];
        let refusal = mosaic_estimate(&opposite, &MosaicConfig::default(), None).unwrap().refusal;
        assert!(refusal.as_deref().is_some_and(|r| r.contains("'antipode'") && r.contains("does not project")), "{refusal:?}");

        let many: Vec<TileGeometry> = (0..51).map(|k| grid_window(&format!("t{k}"), 9.0 * k as f64, 0.0, 10, 10)).collect();
        let est = mosaic_estimate(&many, &MosaicConfig::default(), None).unwrap();
        let refusal = est.refusal.expect("51 tiles are over the limit");
        assert!(refusal.contains("51 tiles") && refusal.contains("at most 50") && refusal.contains("'t50'"), "{refusal}");
        assert!(est.pairs.is_empty() && est.components.is_empty() && est.isolated.is_empty());
        assert_eq!(mosaic_estimate(&many[..50], &MosaicConfig::default(), None).unwrap().refusal, None);

        let mut colour = grid_window("colour", 900.0, 0.0, 1000, 1000);
        colour.planes = 3;
        let mixed = vec![grid_window("mono", 0.0, 0.0, 1000, 1000), colour];
        let refusal = mosaic_estimate(&mixed, &MosaicConfig::default(), None).unwrap().refusal.unwrap();
        assert!(refusal.contains("'colour'") && refusal.contains("'mono'"), "{refusal}");

        let narrow = MosaicConfig { max_side_px: 500, ..MosaicConfig::default() };
        let refusal = mosaic_estimate(&tiles[..1], &narrow, None).unwrap().refusal.unwrap();
        assert!(refusal.contains("500 px per side") && refusal.contains("'a'"), "{refusal}");
    }

    #[test]
    fn refuses_a_canvas_whose_ram_exceeds_the_budget() {
        let wide = vec![centred_tile("wide", 30_000, 20_000, (150.0, 30.0), 1.0, 0.0)];
        let est = mosaic_estimate(&wide, &MosaicConfig::default(), None).unwrap();
        assert!(est.cols <= 40_000 && est.rows <= 40_000);
        assert_eq!(est.ram_bytes, expected_ram(&est, &wide));
        let refusal = est.refusal.expect("about 13 GB is over the 4 GB budget");
        assert!(refusal.contains("above its 4.0 GB budget") && refusal.contains("'wide'"), "{refusal}");
        assert!(!refusal.contains("px per side") && !refusal.contains("times the summed area"), "{refusal}");
        let roomy = MosaicConfig { max_ram_bytes: u64::MAX, ..MosaicConfig::default() };
        assert_eq!(mosaic_estimate(&wide, &roomy, None).unwrap().refusal, None);

        let small = vec![centred_tile("small", 1000, 1000, (150.0, 30.0), 1.0, 0.0)];
        let free = mosaic_estimate(&small, &MosaicConfig::default(), None).unwrap();
        let ram = free.ram_bytes;
        assert_eq!(ram, expected_ram(&free, &small));
        assert!((20_000_000..30_000_000).contains(&ram), "three planes per canvas pixel and one tile in flight: {ram}");
        let memory = |physical: u64, commit: u64| Some(MemoryInfo { physical_available: physical, commit_available: commit });
        let short = mosaic_estimate(&small, &MosaicConfig::default(), memory(ram - 1, ram - 1)).unwrap();
        let refusal = short.refusal.expect("one byte short of the estimate");
        assert!(refusal.starts_with(&format!("A mosaic canvas of {}x{} needs about", free.cols, free.rows)), "{refusal}");
        assert!(refusal.contains("fewer or smaller panels") && refusal.contains("'small'"), "{refusal}");
        let tight = mosaic_estimate(&small, &MosaicConfig::default(), memory(ram, 64 * ram)).unwrap();
        assert_eq!(tight.refusal, None);
        assert!(tight.warnings.iter().any(|w| w.contains("may page to disk")), "{:?}", tight.warnings);
        let ample = mosaic_estimate(&small, &MosaicConfig::default(), memory(64 * ram, 64 * ram)).unwrap();
        assert_eq!((ample.refusal, ample.warnings.len()), (None, 0));

        let mut colour = small.clone();
        colour[0].planes = 3;
        let rgb = mosaic_estimate(&colour, &MosaicConfig::default(), None).unwrap();
        assert_eq!(rgb.ram_bytes, expected_ram(&rgb, &colour));
        assert!(rgb.ram_bytes > ram);
    }

    fn like_cards() -> Vec<(&'static str, String)> {
        let (sin, cos) = 88.0f64.to_radians().sin_cos();
        vec![
            ("NAXIS1", "14344".into()),
            ("NAXIS2", "8589".into()),
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CUNIT1", "'deg'".into()),
            ("CUNIT2", "'deg'".into()),
            ("CRPIX1", "7185.307253487335".into()),
            ("CRPIX2", "4294.478201664114".into()),
            ("CRVAL1", "274.7298929".into()),
            ("CRVAL2", "-13.8517283".into()),
            ("CDELT1", "8.674E-06".into()),
            ("CDELT2", "8.674E-06".into()),
            ("PC1_1", (-cos).to_string()),
            ("PC1_2", sin.to_string()),
            ("PC2_1", sin.to_string()),
            ("PC2_2", cos.to_string()),
            ("RADESYS", "'ICRS'".into()),
            ("BUNIT", "'MJy/sr'".into()),
            ("PIXAR_SR", "2.29E-14".into()),
            ("CTYPE3", "'RGB'".into()),
        ]
    }

    fn like_window(name: &str, like: &[(&'static str, String)], x0: f64, y0: f64, width: usize, height: usize) -> TileGeometry {
        let source = header(like);
        let mut cards = like.to_vec();
        for (key, value) in cards.iter_mut() {
            match *key {
                "NAXIS1" => *value = width.to_string(),
                "NAXIS2" => *value = height.to_string(),
                "CRPIX1" => *value = (source.get_f64("CRPIX1").unwrap() - x0).to_string(),
                "CRPIX2" => *value = (source.get_f64("CRPIX2").unwrap() - y0).to_string(),
                _ => {}
            }
        }
        tile(name, &cards)
    }

    fn owned(cards: &[(&str, String)]) -> Vec<(String, String)> {
        cards.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn canvas_override_keeps_crval_and_cd_and_shifts_crpix_by_an_integer() {
        let mut sip = like_cards();
        sip[2].1 = "'RA---TAN-SIP'".into();
        sip[3].1 = "'DEC--TAN-SIP'".into();
        sip.extend([
            ("A_ORDER", "2".to_string()),
            ("A_2_0", "1.0E-7".to_string()),
            ("B_ORDER", "2".to_string()),
            ("B_0_2", "-1.0E-7".to_string()),
        ]);
        for like in [like_cards(), sip] {
            let tiles = vec![
                like_window("left", &like, 100.0, 200.0, 3000, 2500),
                like_window("right", &like, 2800.0, 200.0, 3000, 2500),
                like_window("overhang", &like, -40.0, 2600.0, 2000, 2000),
            ];
            let est = mosaic_estimate(&tiles, &with_canvas(CanvasSpec::Like { cards: owned(&like) }), None).unwrap();
            assert_eq!(est.refusal, None);
            let canvas = header_from_cards(&est.canvas_cards);
            let source = header(&like);
            for key in ["CTYPE1", "CTYPE2", "CUNIT1", "CUNIT2", "CRVAL1", "CRVAL2", "CDELT1", "CDELT2", "PC1_1", "PC1_2", "PC2_1", "PC2_2", "RADESYS"] {
                assert_eq!(canvas.get(key), source.get(key), "{key}");
            }
            for key in ["A_ORDER", "A_2_0", "B_ORDER", "B_0_2"] {
                assert_eq!(canvas.get(key), source.get(key), "{key}");
            }
            for key in ["BUNIT", "PIXAR_SR", "CTYPE3"] {
                assert_eq!(canvas.get(key), None, "{key} is not a canvas card");
            }
            assert_eq!(read_effective_cd(&canvas).unwrap(), read_effective_cd(&source).unwrap());
            let shift = |key: &str| canvas.get_f64(key).unwrap() - source.get_f64(key).unwrap();
            assert!((shift("CRPIX1") - 40.0).abs() < 1e-9 && (shift("CRPIX2") + 200.0).abs() < 1e-9, "{} {}", shift("CRPIX1"), shift("CRPIX2"));
            assert_eq!((est.cols, est.rows), (5840, 4400));
            let windows: Vec<(usize, usize, usize, usize)> =
                est.tiles.iter().map(|t| (t.window.x0, t.window.y0, t.window.cols, t.window.rows)).collect();
            assert_eq!(windows, vec![(140, 0, 3000, 2500), (2840, 0, 3000, 2500), (0, 2400, 2000, 2000)]);
            let frame = canvas_wcs(&est.canvas_cards).unwrap();
            for (tile, (x0, y0)) in tiles.iter().zip([(140.0, 0.0), (2840.0, 0.0), (0.0, 2400.0)]) {
                for (x, y) in [(0.0, 0.0), (1234.0, 567.0), (1999.0, 1999.0)] {
                    let sky = tile.wcs.pixel_to_world(x, y);
                    let (cx, cy) = frame.world_to_pixel(sky.ra, sky.dec);
                    assert!((cx - x - x0).abs() < 1e-6 && (cy - y - y0).abs() < 1e-6, "{} ({x}, {y}) -> ({cx}, {cy})", tile.name);
                }
            }
        }
        let tiles = vec![grid_window("a", 0.0, 0.0, 100, 100)];
        let blank = with_canvas(CanvasSpec::Like { cards: vec![("BUNIT".to_string(), "'MJy/sr'".to_string())] });
        let refusal = mosaic_estimate(&tiles, &blank, None).unwrap().refusal.unwrap();
        assert!(refusal.contains("no celestial WCS"), "{refusal}");
    }

    #[test]
    fn overlap_graph_reports_components_references_and_isolated_tiles() {
        let tiles = vec![
            grid_window("row-a", 0.0, 0.0, 1000, 1000),
            grid_window("row-b", 800.0, 0.0, 1000, 1000),
            grid_window("row-c", 1700.0, 0.0, 1000, 1000),
            grid_window("big", 0.0, 1100.0, 1000, 1000),
            grid_window("small", 980.0, 1100.0, 300, 300),
            grid_window("sliver", 1277.0, 1100.0, 300, 300),
            grid_window("alone", 2900.0, 1100.0, 1000, 1000),
        ];
        let est = mosaic_estimate(&tiles, &MosaicConfig::default(), None).unwrap();
        assert_eq!(est.refusal, None);
        let pairs: Vec<(usize, usize)> = est.pairs.iter().map(|p| (p.i, p.j)).collect();
        assert_eq!(pairs, vec![(0, 1), (1, 2), (3, 4)]);
        for (pair, expected) in est.pairs.iter().zip([0.2, 0.1, 6000.0 / 90000.0]) {
            assert!((pair.frac - expected).abs() < 2e-3, "{pair:?} against {expected}");
        }
        assert_eq!(
            est.components,
            vec![
                Component { tiles: vec![0, 1, 2], reference: 0 },
                Component { tiles: vec![3, 4], reference: 3 },
                Component { tiles: vec![5], reference: 5 },
                Component { tiles: vec![6], reference: 6 },
            ]
        );
        assert_eq!(est.isolated, vec![5, 6]);

        let chosen = MosaicConfig { references: vec![Some(2), Some(4)], ..MosaicConfig::default() };
        let references: Vec<usize> = mosaic_estimate(&tiles, &chosen, None).unwrap().components.iter().map(|c| c.reference).collect();
        assert_eq!(references, vec![2, 4, 5, 6]);

        let finer = MosaicConfig { min_overlap_frac: 0.005, ..MosaicConfig::default() };
        let est = mosaic_estimate(&tiles, &finer, None).unwrap();
        assert!(est.pairs.iter().any(|p| (p.i, p.j) == (4, 5) && (p.frac - 0.01).abs() < 2e-3), "{:?}", est.pairs);
        assert_eq!(est.components[1].tiles, vec![3, 4, 5]);
        assert_eq!(est.isolated, vec![6]);

        let wrong_group = MosaicConfig { references: vec![Some(3)], ..MosaicConfig::default() };
        let err = format!("{:#}", mosaic_estimate(&tiles, &wrong_group, None).unwrap_err());
        assert!(err.contains("'big'") && err.contains("group 1"), "{err}");
        let out_of_range = MosaicConfig { references: vec![None, Some(99)], ..MosaicConfig::default() };
        assert!(format!("{:#}", mosaic_estimate(&tiles, &out_of_range, None).unwrap_err()).contains("99"));
        let extra_group = MosaicConfig { references: vec![None, None, None, None, Some(6)], ..MosaicConfig::default() };
        assert!(format!("{:#}", mosaic_estimate(&tiles, &extra_group, None).unwrap_err()).contains("group 5"));

        let single = mosaic_estimate(&tiles[..1], &MosaicConfig::default(), None).unwrap();
        assert_eq!((single.components.len(), single.isolated.len()), (1, 0));
    }

    #[test]
    fn default_canvas_is_tan_at_the_mean_tile_centre_with_the_finest_scale_and_the_reference_rotation() {
        let tiles = vec![
            centred_tile("first", 800, 600, (150.0, 30.0), 1.0, 7.0),
            centred_tile("finest", 800, 600, (150.2, 30.1), 0.99, 0.0),
            centred_tile("turned", 800, 600, (150.1, 29.9), 1.01, 90.0),
        ];
        let est = mosaic_estimate(&tiles, &MosaicConfig::default(), None).unwrap();
        let cards = header_from_cards(&est.canvas_cards);
        assert_eq!((cards.get("CTYPE1"), cards.get("CTYPE2")), (Some("'RA---TAN'"), Some("'DEC--TAN'")));
        let mut sum = [0.0f64; 3];
        for t in &tiles {
            let (cx, cy) = pixel_center(t.width, t.height);
            let sky = t.wcs.pixel_to_world(cx, cy);
            let (ra, dec) = (sky.ra.to_radians(), sky.dec.to_radians());
            sum[0] += dec.cos() * ra.cos();
            sum[1] += dec.cos() * ra.sin();
            sum[2] += dec.sin();
        }
        let norm = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        let (ra, dec) = (sum[1].atan2(sum[0]).to_degrees(), (sum[2] / norm).asin().to_degrees());
        assert!((cards.get_f64("CRVAL1").unwrap() - ra).abs() < 1e-9, "{:?} against {ra}", cards.get("CRVAL1"));
        assert!((cards.get_f64("CRVAL2").unwrap() - dec).abs() < 1e-9, "{:?} against {dec}", cards.get("CRVAL2"));
        let crpix = (cards.get_f64("CRPIX1").unwrap(), cards.get_f64("CRPIX2").unwrap());
        assert_eq!((crpix.0.fract(), crpix.1.fract()), (0.0, 0.0), "the tangent point sits on a pixel centre: {crpix:?}");
        let canvas = canvas_wcs(&est.canvas_cards).unwrap();
        assert!((canvas.pixel_scale_arcsec() - 0.99).abs() < 1e-9, "{}", canvas.pixel_scale_arcsec());
        assert!(est.tiles[0].rotation_deg.abs() < 1e-6, "the canvas takes the rotation of the first tile: {}", est.tiles[0].rotation_deg);
        assert!((est.tiles[2].rotation_deg + 83.0).abs() < 0.2, "{}", est.tiles[2].rotation_deg);
        assert!((est.tiles[1].scale_ratio - 1.0).abs() < 1e-9 && (est.tiles[0].scale_ratio - 1.0 / 0.99).abs() < 1e-9);
        assert!(est.tiles.iter().all(|t| t.wcs_kind == WcsKind::Header));

        let north = mosaic_estimate(&tiles, &with_canvas(CanvasSpec::Auto { north_up: true, reference: None }), None).unwrap();
        let cd = read_effective_cd(&header_from_cards(&north.canvas_cards)).unwrap();
        assert_eq!((cd[0][1], cd[1][0]), (0.0, 0.0));
        assert!(cd[0][0] < 0.0 && (cd[1][1] * 3600.0 - 0.99).abs() < 1e-12, "{cd:?}");

        let turned = mosaic_estimate(&tiles, &with_canvas(CanvasSpec::Auto { north_up: false, reference: Some(2) }), None).unwrap();
        assert!(turned.tiles[2].rotation_deg.abs() < 1e-6, "{}", turned.tiles[2].rotation_deg);

        let mut mirrored = rotated_cd(1.0, 20.0);
        mirrored[0][0] = -mirrored[0][0];
        mirrored[1][0] = -mirrored[1][0];
        let flipped = vec![
            tile("mirror", &tan_cards(500, 500, (250.5, 250.5), (150.0, 30.0), mirrored)),
            centred_tile("plain", 500, 500, (150.05, 30.0), 1.0, 0.0),
        ];
        let est = mosaic_estimate(&flipped, &MosaicConfig::default(), None).unwrap();
        let cd = read_effective_cd(&header_from_cards(&est.canvas_cards)).unwrap();
        assert!(cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0] > 0.0, "the canvas keeps the parity of its reference tile: {cd:?}");
        assert!(est.tiles[0].rotation_deg.abs() < 1e-6, "{}", est.tiles[0].rotation_deg);

        assert!(mosaic_estimate(&tiles, &with_canvas(CanvasSpec::Auto { north_up: false, reference: Some(3) }), None).is_err());
        assert!(mosaic_estimate(&[], &MosaicConfig::default(), None).is_err());
    }

    #[test]
    fn load_tiles_reads_size_planes_and_wcs_through_the_tile_loader() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("second")).unwrap();
        let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
        let mono = path(dir.path().join("panel.fits"));
        let colour = path(dir.path().join("second").join("panel.fits"));
        let bare = path(dir.path().join("bare.fits"));
        let wcs = header(&tan_cards(40, 30, (20.5, 15.5), (150.0, 30.0), rotated_cd(1.0, 0.0)));
        let mut with_unit = wcs.clone();
        with_unit.set("BUNIT", "MJy/sr".to_string());
        let plane = Array2::from_elem((30, 40), 1.0f32);
        write_fits_mono(&mono, &plane, Some(&with_unit)).unwrap();
        write_fits_rgb(&colour, &plane, &plane, &plane, Some(&wcs)).unwrap();
        write_fits_mono(&bare, &plane, None).unwrap();

        let tiles = load_tiles(&[mono.clone(), colour.clone()]).unwrap();
        let names: Vec<&str> = tiles.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["panel", "panel_2"]);
        assert_eq!((tiles[0].path.as_str(), tiles[1].path.as_str()), (mono.as_str(), colour.as_str()));
        for (t, planes) in tiles.iter().zip([1, 3]) {
            assert_eq!((t.width, t.height, t.planes), (40, 30, planes), "{}", t.name);
            assert_eq!(t.wcs.kind(), WcsKind::Header);
            let sky = t.wcs.pixel_to_world(19.5, 14.5);
            assert!((sky.ra - 150.0).abs() < 1e-9 && (sky.dec - 30.0).abs() < 1e-9, "{}: {sky:?}", t.name);
        }
        assert_eq!((tiles[0].bunit.as_deref(), tiles[1].bunit.as_deref()), (Some("MJy/sr"), None));
        let err = format!("{:#}", load_tiles(&[mono, bare]).unwrap_err());
        assert!(err.contains("'bare'") && err.contains("celestial WCS"), "{err}");
    }

    const NOISE: f64 = 2.0;

    struct Blob {
        x: f64,
        y: f64,
        amplitude: f64,
        width: f64,
    }

    struct Sky {
        background: f64,
        blobs: Vec<Blob>,
        stars: Vec<(f64, f64, f64)>,
    }

    impl Sky {
        fn nebula(&self, x: f64, y: f64) -> f64 {
            self.blobs
                .iter()
                .map(|b| b.amplitude * (-((x - b.x).powi(2) + (y - b.y).powi(2)) / (2.0 * b.width * b.width)).exp())
                .sum()
        }
    }

    struct Panel {
        name: &'static str,
        x0: f64,
        y0: f64,
        width: usize,
        height: usize,
        scale: f64,
        offset: f64,
        noise: f64,
    }

    fn panel(name: &'static str, x0: f64, y0: f64, offset: f64) -> Panel {
        Panel { name, x0, y0, width: 300, height: 240, scale: 1.0, offset, noise: NOISE }
    }

    fn gaussian(rng: &mut StdRng) -> f64 {
        let u1: f64 = rng.gen_range(f64::EPSILON..1.0);
        let u2: f64 = rng.gen();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    fn scattered_stars(seed: u64, extent: (f64, f64), count: usize) -> Vec<(f64, f64, f64)> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..count).map(|_| (rng.gen_range(0.0..extent.0), rng.gen_range(0.0..extent.1), rng.gen_range(500.0..20_000.0))).collect()
    }

    fn render(sky: &Sky, p: &Panel, seed: u64) -> Array2<f32> {
        let local: Vec<Star> =
            sky.stars.iter().map(|&(x, y, flux)| Star { x: x - p.x0, y: y - p.y0, z: 0.0, flux, temperature: 5800.0 }).collect();
        let starlight = render_stars(&local, &GaussianPsf::from_fwhm(3.0), p.width as u32, p.height as u32);
        let mut rng = StdRng::seed_from_u64(seed);
        Array2::from_shape_fn((p.height, p.width), |(y, x)| {
            let truth = sky.background + sky.nebula(x as f64 + p.x0, y as f64 + p.y0) + starlight[[y, x]] as f64;
            (p.scale * truth + p.offset + p.noise * gaussian(&mut rng)) as f32
        })
    }

    fn build(sky: &Sky, panels: &[Panel]) -> (Vec<TileGeometry>, Vec<Vec<Array2<f32>>>) {
        let tiles = panels.iter().map(|p| grid_window(p.name, p.x0, p.y0, p.width, p.height)).collect();
        let planes = panels.iter().enumerate().map(|(k, p)| vec![render(sky, p, 1000 + k as u64)]).collect();
        (tiles, planes)
    }

    fn levels_of(tiles: &[TileGeometry], planes: &[Vec<Array2<f32>>], cfg: &MosaicConfig) -> (MosaicEstimate, Vec<LevelWindow>, LevelReport) {
        let estimate = mosaic_estimate(tiles, cfg, None).unwrap();
        assert_eq!(estimate.refusal, None);
        let load = |k: usize| -> Result<Vec<Array2<f32>>> { Ok(planes[k].clone()) };
        let windows = level_windows(tiles, &estimate, &load, &never_cancelled, &|_: MosaicStage, _: usize, _: usize| {}).unwrap();
        let report = match_levels(tiles, &estimate, &windows, cfg.levels).unwrap();
        (estimate, windows, report)
    }

    #[test]
    fn injected_offsets_recovered_within_0_2_sigma_per_component() {
        let mut stars = scattered_stars(11, (1240.0, 432.0), 140);
        stars.push((150.0, 120.0, 30_000.0));
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars };
        let panels = [
            panel("a0", 0.0, 0.0, 0.0),
            panel("a1", 240.0, 0.0, 5.0 * NOISE),
            panel("a2", 0.0, 192.0, -3.0 * NOISE),
            panel("a3", 240.0, 192.0, 2.0 * NOISE),
            panel("b0", 700.0, 0.0, 27.0 * NOISE),
            panel("b1", 940.0, 0.0, 30.0 * NOISE),
        ];
        let (tiles, planes) = build(&sky, &panels);
        let cfg = MosaicConfig { references: vec![None, Some(5)], ..MosaicConfig::default() };
        let (estimate, windows, report) = levels_of(&tiles, &planes, &cfg);
        assert_eq!(
            estimate.components,
            vec![Component { tiles: vec![0, 1, 2, 3], reference: 0 }, Component { tiles: vec![4, 5], reference: 5 }]
        );
        assert!(estimate.isolated.is_empty());
        assert_eq!(report.planes.len(), 1);
        let solution = &report.planes[0];
        let expected = [0.0, 5.0, -3.0, 2.0, -3.0, 0.0].map(|s| s * NOISE);
        for (k, (&solved, &injected)) in solution.offsets.iter().zip(&expected).enumerate() {
            assert!((solved - injected).abs() < 0.2 * NOISE, "{}: solved {solved} against {injected}", tiles[k].name);
        }
        assert_eq!((solution.offsets[0], solution.offsets[5]), (0.0, 0.0), "each reference is fixed");
        assert!(solution.scales.iter().all(|&s| s == 1.0), "{:?}", solution.scales);
        assert!(solution.unmatched.is_empty() && solution.unscaled.is_empty(), "{solution:?}");
        assert_eq!(report.stats[0].len(), estimate.pairs.len());
        assert!(report.stats[0].iter().all(|s| s.w >= 50.0), "{:?}", report.stats[0]);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("'a0'") && report.warnings[0].contains("'b1'"), "{}", report.warnings[0]);

        let window = estimate.tiles[0].window;
        let blocks = windows[0].planes[0].dim();
        let span = |start: usize, len: usize| (start + len - 1) / LEVEL_DECIMATION - start / LEVEL_DECIMATION + 1;
        assert_eq!(blocks, (span(window.y0, window.rows), span(window.x0, window.cols)));
        let canvas = canvas_wcs(&estimate.canvas_cards).unwrap();
        let star = tiles[0].wcs.pixel_to_world(150.0, 120.0);
        let (cx, cy) = canvas.world_to_pixel(star.ra, star.dec);
        let block = (cy.round() as usize / LEVEL_DECIMATION - windows[0].block_y0, cx.round() as usize / LEVEL_DECIMATION - windows[0].block_x0);
        assert!(windows[0].planes[0][block].is_nan(), "the block under the bright star is masked");
        let kept = windows[0].planes[0].iter().filter(|v| v.is_finite()).count() as f64 / (blocks.0 * blocks.1) as f64;
        assert!((0.3..0.98).contains(&kept), "{kept} of the blocks are kept");
    }

    #[test]
    fn isolated_tile_is_warned_and_left_at_zero() {
        let blob = Blob { x: 270.0, y: 120.0, amplitude: 300.0, width: 60.0 };
        let sky = Sky { background: 100.0, blobs: vec![blob], stars: scattered_stars(12, (1300.0, 240.0), 60) };
        let panels = [panel("t0", 0.0, 0.0, 0.0), panel("t1", 240.0, 0.0, 3.0 * NOISE), panel("alone", 1000.0, 0.0, 10.0 * NOISE)];
        let (tiles, planes) = build(&sky, &panels);
        for model in [LevelModel::Offset, LevelModel::OffsetScale] {
            let (estimate, _, report) = levels_of(&tiles, &planes, &MosaicConfig { levels: model, ..MosaicConfig::default() });
            assert_eq!(estimate.isolated, vec![2]);
            let solution = &report.planes[0];
            assert_eq!((solution.offsets[2], solution.scales[2]), (0.0, 1.0), "{model:?}");
            assert_eq!(solution.unmatched, vec![2], "{model:?}");
            assert!((solution.offsets[1] - 3.0 * NOISE).abs() < 0.2 * NOISE, "{model:?}: {}", solution.offsets[1]);
            assert_eq!(report.warnings.len(), 1, "{model:?}: {:?}", report.warnings);
            assert!(report.warnings[0].contains("'alone'") && report.warnings[0].contains("overlaps no other tile"), "{}", report.warnings[0]);
        }

        let panels = [panel("t0", 0.0, 0.0, 0.0), panel("t1", 240.0, 0.0, 3.0 * NOISE), panel("blank", 480.0, 0.0, 4.0 * NOISE)];
        let (tiles, mut planes) = build(&sky, &panels);
        planes[2][0].slice_mut(s![.., ..60]).fill(f32::NAN);
        let (estimate, _, report) = levels_of(&tiles, &planes, &MosaicConfig::default());
        assert!(estimate.isolated.is_empty() && estimate.components.len() == 1, "{:?}", estimate.components);
        let solution = &report.planes[0];
        assert_eq!((solution.offsets[2], solution.unmatched.clone()), (0.0, vec![2]));
        assert!((solution.offsets[1] - 3.0 * NOISE).abs() < 0.2 * NOISE, "{}", solution.offsets[1]);
        let blank_pair = report.stats[0].iter().find(|s| (s.i, s.j) == (1, 2)).expect("the geometry pairs t1 with blank");
        assert_eq!(blank_pair.w, 0.0);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("'blank'") && report.warnings[0].contains("no usable pixels"), "{}", report.warnings[0]);
    }

    #[test]
    fn log_scale_term_recovers_0_98_within_0_5_percent() {
        let blobs = vec![
            Blob { x: 290.0, y: 120.0, amplitude: 300.0, width: 70.0 },
            Blob { x: 150.0, y: 230.0, amplitude: 250.0, width: 60.0 },
            Blob { x: 300.0, y: 235.0, amplitude: 200.0, width: 50.0 },
            Blob { x: 200.0, y: 150.0, amplitude: 150.0, width: 200.0 },
        ];
        let sky = Sky { background: 100.0, blobs, stars: scattered_stars(13, (576.0, 461.0), 80) };
        let shot = |name, x0, y0, scale, offset| Panel { name, x0, y0, width: 320, height: 256, scale, offset, noise: NOISE };
        let panels = [shot("s0", 0.0, 0.0, 1.0, 0.0), shot("s1", 256.0, 0.0, 0.98, 4.0 * NOISE), shot("s2", 0.0, 205.0, 1.0, -2.0 * NOISE)];
        let (tiles, planes) = build(&sky, &panels);
        let cfg = MosaicConfig { levels: LevelModel::OffsetScale, ..MosaicConfig::default() };
        let (estimate, _, report) = levels_of(&tiles, &planes, &cfg);
        assert_eq!(estimate.pairs.len(), 3);
        let solution = &report.planes[0];
        assert_eq!((solution.scales[0], solution.offsets[0]), (1.0, 0.0));
        for (k, truth) in [(1, 0.98), (2, 1.0)] {
            assert!((solution.scales[k] / truth - 1.0).abs() < 0.005, "{}: scale {} against {truth}", tiles[k].name, solution.scales[k]);
        }
        for (k, truth) in [(1, 4.0 * NOISE / 0.98), (2, -2.0 * NOISE)] {
            assert!((solution.offsets[k] - truth).abs() < 0.2 * NOISE, "{}: offset {} against {truth}", tiles[k].name, solution.offsets[k]);
        }
        assert!(solution.unscaled.is_empty() && report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(report.stats[0].iter().all(|s| s.log_w > 0.0), "{:?}", report.stats[0]);

        let offset_only = MosaicConfig { levels: LevelModel::Offset, ..MosaicConfig::default() };
        let (_, _, report) = levels_of(&tiles, &planes, &offset_only);
        assert!(report.planes[0].scales.iter().all(|&s| s == 1.0), "{:?}", report.planes[0].scales);

        let flat = Sky { background: 100.0, blobs: Vec::new(), stars: Vec::new() };
        let noisier = Panel { noise: 2.0 * NOISE, ..shot("s1", 256.0, 0.0, 1.0, 4.0 * NOISE) };
        let (tiles, planes) = build(&flat, &[shot("s0", 0.0, 0.0, 1.0, 0.0), noisier, shot("s2", 0.0, 205.0, 1.0, -2.0 * NOISE)]);
        let (_, _, report) = levels_of(&tiles, &planes, &cfg);
        let solution = &report.planes[0];
        assert_eq!(solution.scales, vec![1.0, 1.0, 1.0], "a flat overlap measures no scale, whatever the noise of each tile");
        assert_eq!(solution.unscaled, vec![1, 2]);
        for (k, truth) in [(1, 4.0 * NOISE), (2, -2.0 * NOISE)] {
            assert!((solution.offsets[k] - truth).abs() < 0.2 * NOISE, "{}: offset {} against {truth}", tiles[k].name, solution.offsets[k]);
        }
        for name in ["'s1'", "'s2'"] {
            assert!(report.warnings.iter().any(|w| w.contains(name) && w.contains("too little structure")), "{name}: {:?}", report.warnings);
        }
    }

    #[test]
    fn rgb_tiles_get_levels_per_plane_from_one_star_mask() {
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: scattered_stars(14, (540.0, 240.0), 50) };
        let base = [panel("left", 0.0, 0.0, 0.0), panel("right", 240.0, 0.0, 0.0)];
        let mut tiles: Vec<TileGeometry> = base.iter().map(|p| grid_window(p.name, p.x0, p.y0, p.width, p.height)).collect();
        tiles.iter_mut().for_each(|t| t.planes = 3);
        let offsets = [2.0, -1.0, 4.0].map(|s| s * NOISE);
        let planes: Vec<Vec<Array2<f32>>> = base
            .iter()
            .enumerate()
            .map(|(k, p)| {
                (0..3)
                    .map(|c| render(&sky, &Panel { offset: if k == 1 { offsets[c] } else { 0.0 }, ..*p }, 2000 + 10 * k as u64 + c as u64))
                    .collect()
            })
            .collect();
        let (_, windows, report) = levels_of(&tiles, &planes, &MosaicConfig::default());
        assert_eq!(report.planes.len(), 3);
        for (c, solution) in report.planes.iter().enumerate() {
            assert_eq!(solution.offsets[0], 0.0);
            assert!((solution.offsets[1] - offsets[c]).abs() < 0.2 * NOISE, "plane {}: {} against {}", c + 1, solution.offsets[1], offsets[c]);
        }
        for window in &windows {
            let masked = |p: usize| window.planes[p].mapv(|v| v.is_nan());
            assert!(masked(0) == masked(1) && masked(1) == masked(2), "the planes of a tile share one validity");
            assert!(masked(0).iter().any(|&m| m), "the stars of the summed planes are masked");
        }
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn solve_levels_is_the_weighted_least_squares_fit_with_each_reference_fixed() {
        let stat = |i, j, d, w, log_ratio, log_w| OverlapStat { i, j, d, w, log_ratio, log_w };
        let (g1, g2) = (0.98f64, 0.95f64);
        let stats = vec![
            stat(0, 1, 1.0, 1.0, (1.0 / g1).ln(), 1.0),
            stat(1, 2, 2.0, 1.0, (g1 / g2).ln(), 1.0),
            stat(2, 0, -3.3, 2.0, g2.ln(), 3.0),
            stat(0, 1, 50.0, 0.0, 9.0, 0.0),
            stat(1, 2, f64::NAN, 5.0, f64::NAN, 5.0),
            stat(3, 4, -0.5, 1.0, 1.1f64.ln(), 0.0),
        ];
        let components = vec![
            Component { tiles: vec![0, 1, 2], reference: 0 },
            Component { tiles: vec![3, 4], reference: 4 },
            Component { tiles: vec![5], reference: 5 },
            Component { tiles: vec![6, 7], reference: 6 },
        ];
        let solved = solve_levels(8, &stats, &components, LevelModel::OffsetScale);
        let offsets = [0.0, -1.12, -3.24, -0.5, 0.0, 0.0, 0.0, 0.0];
        let scales = [1.0, g1, g2, 1.0, 1.0, 1.0, 1.0, 1.0];
        for k in 0..8 {
            assert!((solved.offsets[k] - offsets[k]).abs() < 1e-12, "offset {k}: {} against {}", solved.offsets[k], offsets[k]);
            assert!((solved.scales[k] - scales[k]).abs() < 1e-12, "scale {k}: {} against {}", solved.scales[k], scales[k]);
        }
        assert_eq!((solved.unmatched.clone(), solved.unscaled.clone()), (vec![5, 7], vec![3]));

        let offsets_only = solve_levels(8, &stats, &components, LevelModel::Offset);
        assert_eq!(offsets_only.offsets, solved.offsets);
        assert_eq!((offsets_only.scales, offsets_only.unscaled), (vec![1.0; 8], Vec::new()));
        let single = solve_levels(1, &[], &[Component { tiles: vec![0], reference: 0 }], LevelModel::OffsetScale);
        assert!(single.unmatched.is_empty() && single.offsets == vec![0.0] && single.scales == vec![1.0]);
    }

    fn grid_canvas() -> MosaicConfig {
        with_canvas(CanvasSpec::Like { cards: owned(&tan_cards(1, 1, (1.0, 1.0), (150.0, 30.0), rotated_cd(1.0, 0.0))) })
    }

    fn no_progress(_: MosaicStage, _: usize, _: usize) {}

    fn mosaic_of(tiles: &[TileGeometry], planes: &[Vec<Array2<f32>>], cfg: &MosaicConfig) -> MosaicOutput {
        let load = |k: usize| -> Result<TilePixels> { Ok(TilePixels { planes: planes[k].clone(), excluded: None }) };
        run_mosaic_with(tiles, cfg, &load, None, &never_cancelled, &no_progress).unwrap()
    }

    fn sip_cards(side: usize, crval: (f64, f64), scale_arcsec: f64, rotation_deg: f64) -> Vec<(&'static str, String)> {
        let centre = (side as f64 + 1.0) / 2.0;
        let mut cards = tan_cards(side, side, (centre, centre), crval, rotated_cd(scale_arcsec, rotation_deg));
        cards[2].1 = "'RA---TAN-SIP'".into();
        cards[3].1 = "'DEC--TAN-SIP'".into();
        cards.extend([
            ("A_ORDER", "2".to_string()),
            ("A_2_0", "3.0E-6".to_string()),
            ("A_1_1", "-2.0E-6".to_string()),
            ("A_0_2", "1.0E-6".to_string()),
            ("B_ORDER", "2".to_string()),
            ("B_2_0", "-1.0E-6".to_string()),
            ("B_1_1", "2.0E-6".to_string()),
            ("B_0_2", "3.0E-6".to_string()),
        ]);
        cards
    }

    fn centroid(image: &Array2<f32>, (x0, y0): (f64, f64)) -> Option<(f64, f64)> {
        const APERTURE_PX: f64 = 6.0;
        const SKY_PX: (f64, f64) = (8.0, 11.0);
        let (rows, cols) = image.dim();
        let reach = SKY_PX.1.ceil() as i64 + 1;
        let (ix, iy) = (x0.round() as i64, y0.round() as i64);
        if ix < reach || iy < reach || ix + reach >= cols as i64 || iy + reach >= rows as i64 {
            return None;
        }
        let pixels: Vec<(i64, i64)> = (iy - reach..=iy + reach).flat_map(|y| (ix - reach..=ix + reach).map(move |x| (x, y))).collect();
        let at = |&(x, y): &(i64, i64)| image[[y as usize, x as usize]] as f64;
        if pixels.iter().any(|p| !at(p).is_finite()) {
            return None;
        }
        let sky: Vec<f64> =
            pixels.iter().filter(|&&(x, y)| (SKY_PX.0..=SKY_PX.1).contains(&(x as f64 - x0).hypot(y as f64 - y0))).map(at).collect();
        let background = exact_median_f64(&sky);
        let (mut cx, mut cy) = (x0, y0);
        for _ in 0..50 {
            let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
            for p in pixels.iter().filter(|&&(x, y)| (x as f64 - cx).hypot(y as f64 - cy) <= APERTURE_PX) {
                let v = at(p) - background;
                sum += v;
                sx += v * p.0 as f64;
                sy += v * p.1 as f64;
            }
            let next = (sx / sum, sy / sum);
            let moved = (next.0 - cx).hypot(next.1 - cy);
            (cx, cy) = next;
            if moved < 1e-10 {
                break;
            }
        }
        Some((cx, cy))
    }

    #[test]
    fn dq_excluded_and_border_zero_runs_are_invalid_interior_zeros_kept() {
        let (rows, cols) = (12, 16);
        let mut plane = Array2::from_elem((rows, cols), 5.0f32);
        let mut expected = Array2::from_elem((rows, cols), true);
        plane.slice_mut(s![.., ..2]).fill(0.0);
        expected.slice_mut(s![.., ..2]).fill(false);
        let snake: [(usize, usize); 11] = [(0, 8), (1, 8), (2, 8), (2, 9), (2, 10), (3, 10), (4, 10), (4, 11), (4, 12), (3, 12), (2, 12)];
        for p in snake {
            plane[p] = 0.0;
            expected[p] = false;
        }
        let interior: [(usize, usize); 5] = [(8, 6), (6, 12), (6, 13), (7, 12), (7, 13)];
        for p in interior {
            plane[p] = 0.0;
        }
        plane[[5, 5]] = f32::NAN;
        expected[[5, 5]] = false;
        let mut excluded = Array2::<u8>::zeros((rows, cols));
        excluded[[9, 9]] = 1;
        excluded[[10, 13]] = 1;
        let unflagged = expected.clone();
        expected[[9, 9]] = false;
        expected[[10, 13]] = false;
        assert_eq!(tile_validity(std::slice::from_ref(&plane), Some(&excluded)), expected);
        assert_eq!(tile_validity(std::slice::from_ref(&plane), None), unflagged);

        let mut colour = vec![plane.clone(), plane.clone(), plane.clone()];
        colour[1][[9, 4]] = 0.0;
        colour[2].slice_mut(s![11, 4..8]).fill(0.0);
        let mut shared = expected.clone();
        shared.slice_mut(s![11, 4..8]).fill(false);
        assert_eq!(tile_validity(&colour, Some(&excluded)), shared, "the three planes share one validity");

        let dir = tempfile::tempdir().unwrap();
        let wcs = header(&tan_cards(16, 12, (8.5, 6.5), (150.0, 30.0), rotated_cd(1.0, 0.0)));
        let sci = Array2::from_elem((12, 16), 7.0f32);
        let mut dq = Array2::<u32>::zeros((12, 16));
        for (p, bits) in [((2usize, 3usize), 1u32), ((4, 5), 2), ((6, 7), 0x4000), ((8, 9), 16)] {
            dq[p] = bits;
        }
        let flagged = |telescope: &str, dq_mask: bool| -> Vec<(usize, usize)> {
            let path = dir.path().join(format!("{telescope}_{dq_mask}.fits")).to_string_lossy().into_owned();
            let mut primary = HduHeader::empty();
            primary.set("TELESCOP", telescope.to_string());
            let hdus = [
                ImageHdu { data: HduData::F32(&sci), header: Some(&wcs), extname: "SCI", extver: 1 },
                ImageHdu { data: HduData::U32(&dq), header: None, extname: "DQ", extver: 1 },
            ];
            write_mef_images(&path, Some(&primary), &hdus).unwrap();
            let tiles = load_tiles(&[path]).unwrap();
            let pixels = load_tile_pixels(&tiles[0], dq_mask).unwrap();
            assert_eq!(pixels.planes, vec![sci.clone()], "{telescope}");
            let valid = tile_validity(&pixels.planes, pixels.excluded.as_ref());
            valid.indexed_iter().filter(|(_, v)| !**v).map(|(p, _)| p).collect()
        };
        assert_eq!(flagged("JWST", true), vec![(2, 3)]);
        assert_eq!(flagged("ROMAN", true), vec![(2, 3)]);
        assert_eq!(flagged("HST", true), vec![(2, 3), (4, 5), (8, 9)]);
        assert!(flagged("SYNTH", true).is_empty());
        assert!(flagged("JWST", false).is_empty());

        let padded_path = dir.path().join("padded.fits").to_string_lossy().into_owned();
        let mut padded = Array2::from_elem((31, 41), 9.0f32);
        padded.slice_mut(s![.., ..6]).fill(0.0);
        padded[[15, 25]] = 0.0;
        let padded_wcs = header(&tan_cards(41, 31, (21.0, 16.0), (150.0, 30.0), rotated_cd(1.0, 0.0)));
        write_fits_mono(&padded_path, &padded, Some(&padded_wcs)).unwrap();
        let tiles = load_tiles(&[padded_path]).unwrap();
        let out = run_mosaic(&tiles, &MosaicConfig::default(), &never_cancelled, &no_progress).unwrap();
        assert_eq!(out.coverage.dim(), (31, 41));
        for y in 0..31 {
            for x in 0..5 {
                assert!(out.coverage[[y, x]] == 0 && out.planes[0][[y, x]].is_nan(), "({y}, {x}) is zero padding");
            }
            for x in 6..41 {
                assert_eq!(out.coverage[[y, x]], 1, "({y}, {x})");
            }
        }
        assert!(out.planes[0][[15, 25]].abs() < 1e-4, "the interior zero is kept: {}", out.planes[0][[15, 25]]);
        assert!((out.planes[0][[20, 30]] - 9.0).abs() < 1e-4, "{}", out.planes[0][[20, 30]]);
    }

    #[test]
    fn feather_weights_sum_to_one_across_a_straight_seam() {
        let feather = 64.0;
        for (start, narrow) in [(260usize, true), (100, false)] {
            let tiles = vec![grid_window("left", 0.0, 0.0, 300, 240), grid_window("right", start as f64, 0.0, 300, 240)];
            let planes: Vec<Vec<Array2<f32>>> = tiles.iter().map(|t| vec![Array2::from_elem((t.height, t.width), 100.0f32)]).collect();
            let estimate = mosaic_estimate(&tiles, &grid_canvas(), None).unwrap();
            assert_eq!(estimate.refusal, None);
            let windows: Vec<CanvasWindow> = estimate.tiles.iter().map(|t| t.window).collect();
            assert_eq!((windows[0].x0, windows[1].x0, estimate.cols, estimate.rows), (0, start, start + 300, 240));
            let load = |k: usize| -> Result<Vec<Array2<f32>>> { Ok(planes[k].clone()) };
            let (_, maps) = level_and_feather_windows(&tiles, &estimate, &load, &never_cancelled, &no_progress).unwrap();
            let weights: Vec<Array2<f32>> = (0..2)
                .map(|k| {
                    let valid = Array2::from_elem((windows[k].rows, windows[k].cols), true);
                    feather_weights(&maps, k, &[1 - k], &windows[k], &valid, feather)
                })
                .collect();
            let at = |k: usize, x: usize, y: usize| weights[k][[y - windows[k].y0, x - windows[k].x0]] as f64;
            let share = |x: usize, y: usize| at(0, x, y) / (at(0, x, y) + at(1, x, y));
            for y in 80..160 {
                assert!((0..start).all(|x| at(0, x, y) == 1.0), "row {y}: the left tile alone keeps its full weight");
                assert!((300..start + 300).all(|x| at(1, x, y) == 1.0), "row {y}: the right tile alone keeps its full weight");
                for x in start..300 {
                    let (left, right) = ((300 - x) as f64, (x + 1 - start) as f64);
                    let reach = feather.min(left + right);
                    let expected = (smootherstep((left / reach).min(1.0)), smootherstep((right / reach).min(1.0)));
                    let (w_left, w_right) = (at(0, x, y), at(1, x, y));
                    assert!(
                        (w_left - expected.0).abs() < 1e-6 && (w_right - expected.1).abs() < 1e-6,
                        "({x}, {y}): {w_left} and {w_right} against {expected:?}"
                    );
                    assert!((w_left - at(1, start + 299 - x, y)).abs() < 1e-6, "({x}, {y}): the seam is symmetric");
                    if narrow {
                        assert!((w_left + w_right - 1.0).abs() < 1e-6, "({x}, {y}): {w_left} + {w_right}");
                    }
                    if x > start {
                        assert!(share(x, y) <= share(x - 1, y) + 1e-9, "({x}, {y}): the left share falls across the seam");
                    }
                }
                assert!(share(start, y) > 0.99 && share(299, y) < 0.01, "row {y}: {} .. {}", share(start, y), share(299, y));
            }
            if !narrow {
                assert_eq!(share(200, 120), 0.5, "deep in a wide overlap both tiles weigh the same");
            }
        }
    }

    #[test]
    fn floored_weight_keeps_1e_17_values_beside_a_flagged_pixel() {
        let window = CanvasWindow { x0: 0, y0: 0, cols: 32, rows: 32 };
        let mut left_blocks = Array2::from_elem((8, 8), true);
        left_blocks.slice_mut(s![3..5, 3..5]).fill(false);
        let mut right_blocks = Array2::from_elem((8, 8), true);
        right_blocks[[3, 3]] = false;
        let maps = [FeatherMap::of_blocks(&left_blocks, 0, 0), FeatherMap::of_blocks(&right_blocks, 0, 0)];
        let pixel = (15usize, 15usize);
        let (own, other) = (maps[0].distance_at(15.0, 15.0), maps[1].distance_at(15.0, 15.0));
        assert!(own <= 0.0 && other > 0.0, "the corner pixel sits outside the left map and inside the right one: {own} {other}");
        let value = 2e-17f32;
        let left_valid = Array2::from_elem((32, 32), true);
        let mut right_valid = left_valid.clone();
        right_valid[pixel] = false;
        let mut canvas = CanvasSums::new(32, 32, 1);
        for (k, valid) in [left_valid, right_valid].iter().enumerate() {
            let weights = feather_weights(&maps, k, &[1 - k], &window, valid, 64.0);
            canvas.add(&window, &[Array2::from_elem((32, 32), value)], &weights);
        }
        let (planes, coverage) = canvas.finish();
        assert_eq!(coverage[pixel], 1);
        let found = planes[0][pixel];
        assert!((found / value - 1.0).abs() < 1e-3, "{found:e} against {value:e}");
    }

    #[test]
    fn ramp_is_smootherstep() {
        let feather = 64.0;
        for (t, expected) in [(0.0, 0.0), (0.25, 0.103515625), (0.5, 0.5), (0.75, 0.896484375), (1.0, 1.0)] {
            assert!((smootherstep(t) - expected).abs() < 1e-12, "s({t}) = {}", smootherstep(t));
            assert!((smootherstep(1.0 - t) + expected - 1.0).abs() < 1e-12, "s(1 - {t})");
            let weight = feather_weight(t * feather, 1000.0, feather);
            assert!((weight - expected).abs() < 1e-12, "a pixel {} px inside weighs {weight}", t * feather);
        }
        assert_eq!(feather_weight(100.0, 1000.0, feather), 1.0, "t stops at 1 past the feather");
        assert_eq!(feather_weight(0.0, 10.0, feather), 0.0, "the edge of a tile inside another tile weighs nothing");
        assert_eq!(feather_weight(20.0, 0.0, feather), 1.0, "a pixel no other tile covers keeps its full weight");
        let clamped = feather_weight(10.0, 30.0, feather);
        assert!((clamped - smootherstep(0.25)).abs() < 1e-12, "the feather is clamped to the 40 px overlap: {clamped}");
        assert_eq!(feather_weight(10.0, 30.0, 0.0), 1.0, "a zero feather blends the overlap evenly");
    }

    #[test]
    fn split_on_its_own_canvas_reassembles_within_1e_6_of_max() {
        let (width, height) = (520usize, 400usize);
        let mut cards = tan_cards(width, height, (260.5, 200.5), (150.0, 30.0), rotated_cd(1.0, 25.0));
        cards[2].1 = "'RA---TAN-SIP'".into();
        cards[3].1 = "'DEC--TAN-SIP'".into();
        cards.extend([
            ("A_ORDER", "2".to_string()),
            ("A_2_0", "2.0E-6".to_string()),
            ("B_ORDER", "2".to_string()),
            ("B_0_2", "-1.5E-6".to_string()),
        ]);
        let blobs = vec![Blob { x: 260.0, y: 200.0, amplitude: 400.0, width: 90.0 }];
        let sky = Sky { background: 100.0, blobs, stars: scattered_stars(41, (520.0, 400.0), 150) };
        let whole = Panel { name: "whole", x0: 0.0, y0: 0.0, width, height, scale: 1.0, offset: 0.0, noise: NOISE };
        let original = render(&sky, &whole, 4100);
        let spans = [(0usize, 0usize, 300usize, 230usize), (240, 0, 280, 230), (0, 170, 300, 230), (240, 170, 280, 230)];
        let tiles: Vec<TileGeometry> = spans
            .iter()
            .enumerate()
            .map(|(k, &(x0, y0, w, h))| like_window(&format!("part{k}"), &cards, x0 as f64, y0 as f64, w, h))
            .collect();
        let planes: Vec<Vec<Array2<f32>>> =
            spans.iter().map(|&(x0, y0, w, h)| vec![original.slice(s![y0..y0 + h, x0..x0 + w]).to_owned()]).collect();
        let calls = Mutex::new(Vec::new());
        let stages = Mutex::new(Vec::new());
        let load = |k: usize| -> Result<TilePixels> {
            calls.lock().unwrap().push(k);
            Ok(TilePixels { planes: planes[k].clone(), excluded: None })
        };
        let record = |stage: MosaicStage, done: usize, total: usize| stages.lock().unwrap().push((stage, done, total));
        let cfg = with_canvas(CanvasSpec::Like { cards: owned(&cards) });
        let out = run_mosaic_with(&tiles, &cfg, &load, None, &never_cancelled, &record).unwrap();
        assert_eq!((out.planes.len(), out.planes[0].dim()), (1, (height, width)));
        let peak = original.iter().fold(0.0f64, |m, &v| m.max((v as f64).abs()));
        let worst = out.planes[0].iter().zip(&original).map(|(&a, &b)| (a as f64 - b as f64).abs()).fold(0.0, f64::max);
        assert!(worst <= 1e-6 * peak, "the worst pixel is {worst} off against a peak of {peak}");
        for (y, x, count) in [(100usize, 100usize, 1), (100, 270, 2), (200, 100, 2), (200, 270, 4), (350, 450, 1)] {
            assert_eq!(out.coverage[[y, x]], count, "({y}, {x})");
        }
        assert!(out.coverage.iter().all(|&c| c >= 1));
        assert_eq!(*calls.lock().unwrap(), vec![0, 1, 2, 3, 0, 1, 2, 3], "each tile is loaded once per pass, one at a time");
        let seen = stages.lock().unwrap().clone();
        let main: Vec<(MosaicStage, usize, usize)> = seen.iter().copied().filter(|s| s.0 != MosaicStage::Residuals).collect();
        let expected: Vec<(MosaicStage, usize, usize)> = (1..=4)
            .map(|k| (MosaicStage::Overlaps, k, 4))
            .chain([(MosaicStage::Levels, 1, 1)])
            .chain((1..=4).map(|k| (MosaicStage::Warp, k, 4)))
            .collect();
        assert_eq!(main, expected);
        assert_eq!(out.pair_residuals.len(), 6);
        assert!(seen.iter().filter(|s| s.0 == MosaicStage::Residuals).all(|s| s.1 <= s.2 && s.2 == 6), "{seen:?}");
        assert!(out.levels[0].offsets.iter().all(|o| o.abs() < 1e-3), "{:?}", out.levels[0].offsets);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.rejected_px, 0);
    }

    #[test]
    fn nan_padding_stays_nan_with_zero_coverage_on_its_own_canvas() {
        let (width, height) = (240usize, 160usize);
        let cards = tan_cards(width, height, (120.5, 80.5), (150.0, 30.0), rotated_cd(1.0, 25.0));
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: scattered_stars(43, (240.0, 160.0), 40) };
        let whole = Panel { name: "whole", x0: 0.0, y0: 0.0, width, height, scale: 1.0, offset: 0.0, noise: NOISE };
        let padding = |y: usize, x: usize| x < 12 || y < 9 || y >= 150 || x + y < 40;
        let mut original = render(&sky, &whole, 4300);
        original.indexed_iter_mut().filter(|((y, x), _)| padding(*y, *x)).for_each(|(_, value)| *value = f32::NAN);
        let spans = [(0usize, 140usize), (100, 140)];
        let tiles: Vec<TileGeometry> = spans
            .iter()
            .enumerate()
            .map(|(k, &(x0, w))| like_window(&format!("part{k}"), &cards, x0 as f64, 0.0, w, height))
            .collect();
        let planes: Vec<Vec<Array2<f32>>> = spans.iter().map(|&(x0, w)| vec![original.slice(s![.., x0..x0 + w]).to_owned()]).collect();
        let out = mosaic_of(&tiles, &planes, &with_canvas(CanvasSpec::Like { cards: owned(&cards) }));
        assert_eq!(out.planes[0].dim(), (height, width));
        let peak = original.iter().filter(|v| v.is_finite()).fold(0.0f64, |m, &v| m.max((v as f64).abs()));
        let mut worst = 0.0f64;
        let mut leaked = Vec::new();
        for ((y, x), &value) in original.indexed_iter() {
            let (found, count) = (out.planes[0][[y, x]], out.coverage[[y, x]]);
            if value.is_finite() {
                assert!(found.is_finite() && count >= 1, "data pixel ({y}, {x}) came out {found} with coverage {count}");
                worst = worst.max((found as f64 - value as f64).abs());
            } else if found.is_finite() || count != 0 {
                leaked.push((y, x, found, count));
            }
        }
        let padded = original.iter().filter(|v| !v.is_finite()).count();
        assert!(leaked.is_empty(), "{} of {padded} padding pixels became data, first {:?}", leaked.len(), leaked.first());
        assert!(worst <= 1e-6 * peak, "the worst pixel is {worst} off against a peak of {peak}");
    }

    #[test]
    fn synthetic_star_centroids_within_0_02_px() {
        const SIDE: usize = 1024;
        let sky_frame = WcsTransform::from_header(&header(&tan_cards(1, 1, (1.0, 1.0), (150.0, 30.0), rotated_cd(1.0, 0.0)))).unwrap();
        let step = 0.85 * SIDE as f64;
        let extent = SIDE as f64 + 2.0 * step;
        let mut rng = StdRng::seed_from_u64(61);
        let field: Vec<(f64, f64, f64)> =
            (0..1200).map(|_| (rng.gen_range(0.0..extent), rng.gen_range(0.0..extent), rng.gen_range(6000.0..20_000.0))).collect();
        let stars: Vec<(f64, f64, f64)> = field
            .iter()
            .map(|&(x, y, flux)| {
                let sky = sky_frame.pixel_to_world(x, y);
                (sky.ra, sky.dec, flux)
            })
            .collect();
        let (rotations, scales) = ([0.0, 7.0, 90.0], [1.0, 0.99, 1.01]);
        let psf = GaussianPsf::from_fwhm(3.0);
        let margin = 10.0;
        let mut tiles = Vec::new();
        let mut planes = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                let k = 3 * r + c;
                let half = (SIDE as f64 - 1.0) / 2.0;
                let centre = sky_frame.pixel_to_world(half + c as f64 * step, half + r as f64 * step);
                let t = tile(&format!("v{k}"), &sip_cards(SIDE, (centre.ra, centre.dec), scales[(k + r) % 3], rotations[k % 3]));
                let local: Vec<Star> = stars
                    .iter()
                    .map(|&(ra, dec, flux)| {
                        let (x, y) = t.wcs.world_to_pixel(ra, dec);
                        Star { x, y, z: 0.0, flux, temperature: 5800.0 }
                    })
                    .filter(|s| (-margin..SIDE as f64 + margin).contains(&s.x) && (-margin..SIDE as f64 + margin).contains(&s.y))
                    .collect();
                let light = render_stars(&local, &psf, SIDE as u32, SIDE as u32);
                let mut noise = StdRng::seed_from_u64(6100 + k as u64);
                planes.push(vec![light.mapv(|v| (100.0 + v as f64 + 0.25 * gaussian(&mut noise)) as f32)]);
                tiles.push(t);
            }
        }
        let out = mosaic_of(&tiles, &planes, &MosaicConfig::default());
        let canvas = canvas_wcs(&out.cards).unwrap();
        let mut errors = Vec::new();
        let mut in_overlaps = 0;
        for (k, &(ra, dec, _)) in stars.iter().enumerate() {
            if field.iter().enumerate().any(|(j, s)| j != k && (s.0 - field[k].0).hypot(s.1 - field[k].1) < 14.0) {
                continue;
            }
            let expected = canvas.world_to_pixel(ra, dec);
            let Some(found) = centroid(&out.planes[0], expected) else {
                continue;
            };
            if out.coverage[[expected.1.round() as usize, expected.0.round() as usize]] > 1 {
                in_overlaps += 1;
            }
            errors.push((found.0 - expected.0).hypot(found.1 - expected.1));
        }
        let worst = errors.iter().copied().fold(0.0, f64::max);
        assert!(errors.len() >= 800 && in_overlaps >= 150, "{} stars measured, {in_overlaps} of them in overlaps", errors.len());
        assert!(worst < 0.02, "the worst centroid is {worst} px off over {} stars", errors.len());
        let measured: Vec<&PairResidual> = out.pair_residuals.iter().filter(|p| p.matched > 0).collect();
        assert!(measured.len() >= 10, "{:?}", out.pair_residuals);
        assert!(measured.iter().all(|p| p.dx.hypot(p.dy) < 0.1), "{measured:?}");
        assert!(!out.warnings.iter().any(|w| w.contains("disagree")), "{:?}", out.warnings);
    }

    #[test]
    fn rgb_tiles_share_geometry_and_feather() {
        let dir = tempfile::tempdir().unwrap();
        let blob = Blob { x: 270.0, y: 120.0, amplitude: 200.0, width: 50.0 };
        let sky = Sky { background: 100.0, blobs: vec![blob], stars: scattered_stars(51, (540.0, 240.0), 60) };
        let panels = [panel("left", 0.0, 0.0, 0.0), panel("right", 240.0, 0.0, 0.0)];
        let offsets = [2.0, -1.0, 4.0].map(|o| o * NOISE);
        let ramp = |x: usize| 0.1 * (x as f64 + 240.0 - 270.0);
        let mut inputs: Vec<Vec<Array2<f32>>> = Vec::new();
        for (k, p) in panels.iter().enumerate() {
            let base = render(&sky, p, 5100 + k as u64);
            let planes = (0..3)
                .map(|c| {
                    let gain = (c + 1) as f64;
                    Array2::from_shape_fn(base.dim(), |(y, x)| {
                        let (shift, offset) = if k == 1 { (ramp(x), offsets[c]) } else { (0.0, 0.0) };
                        (gain * (base[[y, x]] as f64 + shift) + offset) as f32
                    })
                })
                .collect();
            inputs.push(planes);
        }
        inputs[1][1].slice_mut(s![100..112, 20..32]).fill(f32::NAN);
        let mut paths = Vec::new();
        for (p, planes) in panels.iter().zip(&inputs) {
            let path = dir.path().join(format!("{}.fits", p.name)).to_string_lossy().into_owned();
            let wcs = header(&tan_cards(p.width, p.height, (1.0 - p.x0, 1.0 - p.y0), (150.0, 30.0), rotated_cd(1.0, 0.0)));
            write_fits_rgb(&path, &planes[0], &planes[1], &planes[2], Some(&wcs)).unwrap();
            paths.push(path);
        }
        let tiles = load_tiles(&paths).unwrap();
        assert!(tiles.iter().all(|t| t.planes == 3));
        let loaded = load_tile_pixels(&tiles[1], true).unwrap();
        assert!(loaded.excluded.is_none());
        assert_eq!(loaded.planes.len(), 3);
        for (read, written) in loaded.planes.iter().zip(&inputs[1]) {
            assert!(read.iter().zip(written).all(|(a, b)| a == b || (a.is_nan() && b.is_nan())));
        }
        let out = run_mosaic(&tiles, &grid_canvas(), &never_cancelled, &no_progress).unwrap();
        assert_eq!((out.planes.len(), out.levels.len()), (3, 3));
        let common = out.levels[0].offsets[1] - offsets[0];
        for (c, levels) in out.levels.iter().enumerate() {
            let shared = (levels.offsets[1] - offsets[c]) / (c + 1) as f64;
            assert!((shared - common).abs() < 1e-3, "plane {}: {shared} against {common}", c + 1);
        }
        assert!(common.abs() < 0.5, "the ramp has no median across the overlap: {common}");
        for ((y, x), &red) in out.planes[0].indexed_iter() {
            if !red.is_finite() {
                continue;
            }
            for c in 1..3 {
                let scaled = out.planes[c][[y, x]] / (c + 1) as f32;
                assert!((scaled - red).abs() <= 1e-4 * red.abs().max(1.0), "({y}, {x}) plane {}: {scaled} against {red}", c + 1);
            }
        }
        for y in 102..110 {
            for x in 262..270 {
                assert_eq!(out.coverage[[y, x]], 1, "({y}, {x}): the NaN in one plane of 'right' masks its three planes");
                for c in 0..3 {
                    let (value, left) = (out.planes[c][[y, x]], inputs[0][c][[y, x]]);
                    assert!((value - left).abs() < 1e-3, "({y}, {x}) plane {}: {value} against {left}", c + 1);
                }
            }
        }
        assert_eq!(out.coverage[[50, 270]], 2);
    }

    #[test]
    fn mixed_scales_keep_values_and_write_pixar() {
        let shot = |name: &str, ra: f64, scale: f64, bunit: &str| {
            let mut cards = tan_cards(300, 240, (150.5, 120.5), (ra, 30.0), rotated_cd(scale, 0.0));
            cards.push(("BUNIT", format!("'{bunit}'")));
            tile(name, &cards)
        };
        let run = |coarse_scale: f64, units: (&str, &str)| {
            let east = 150.0 + 250.0 / 3600.0 / 30f64.to_radians().cos();
            let tiles = vec![shot("fine", 150.0, 1.0, units.0), shot("coarse", east, coarse_scale, units.1)];
            let planes: Vec<Vec<Array2<f32>>> = tiles.iter().map(|t| vec![Array2::from_elem((t.height, t.width), 50.0f32)]).collect();
            let estimate = mosaic_estimate(&tiles, &MosaicConfig::default(), None).unwrap();
            (estimate, mosaic_of(&tiles, &planes, &MosaicConfig::default()))
        };
        let (estimate, out) = run(1.03, ("DN/s", "DN/s"));
        let warning = out.warnings.iter().find(|w| w.contains("2 %")).expect("the 3 % coarser tile is warned");
        assert!(warning.contains("'coarse'") && !warning.contains("'fine'") && warning.contains("DN/s"), "{warning}");
        assert!(estimate.warnings.contains(warning), "{:?}", estimate.warnings);
        assert_eq!(out.coverage.iter().filter(|&&c| c > 0).count(), out.planes[0].iter().filter(|v| v.is_finite()).count());
        assert!(out.planes[0].iter().filter(|v| v.is_finite()).all(|v| (v - 50.0).abs() < 1e-3), "values are copied unscaled");
        let cards = header_from_cards(&out.cards);
        let pixel_sr = (1.0f64 / 3600.0).to_radians().powi(2);
        assert!((cards.get_f64("PIXAR_SR").unwrap() / pixel_sr - 1.0).abs() < 1e-9, "{:?}", cards.get("PIXAR_SR"));
        assert!((cards.get_f64("PIXAR_A2").unwrap() - 1.0).abs() < 1e-9, "{:?}", cards.get("PIXAR_A2"));
        assert_eq!((cards.get("BUNIT"), cards.get("CTYPE1")), (Some("'DN/s'"), Some("'RA---TAN'")), "BUNIT is quoted like the canvas strings");
        let frame = canvas_wcs(&out.cards).unwrap();
        assert!((frame.pixel_scale_arcsec() - 1.0).abs() < 1e-9);

        for (scale, units) in [(1.03, ("MJy/sr", "MJy/sr")), (1.015, ("DN/s", "DN/s"))] {
            let (_, out) = run(scale, units);
            assert!(!out.warnings.iter().any(|w| w.contains("2 %")), "{scale} {units:?}: {:?}", out.warnings);
        }
        let (_, out) = run(1.0, ("MJy/sr", "DN/s"));
        assert_eq!(header_from_cards(&out.cards).get("BUNIT"), None);
        assert!(
            out.warnings.iter().any(|w| w.contains("'fine'") && w.contains("'coarse'") && w.contains("BUNIT")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn a_misregistered_pair_is_warned() {
        let stars = scattered_stars(71, (540.0, 240.0), 120);
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: stars.clone() };
        let panels = [panel("left", 0.0, 0.0, 0.0), panel("right", 240.0, 0.0, 0.0)];
        let (tiles, planes) = build(&sky, &panels);
        let aligned = mosaic_of(&tiles, &planes, &MosaicConfig::default());
        assert_eq!(aligned.pair_residuals.len(), 1);
        let pair = aligned.pair_residuals[0];
        assert!((pair.i, pair.j) == (0, 1) && pair.matched >= 4 && pair.dx.hypot(pair.dy) < 0.1, "{pair:?}");
        assert!(!aligned.warnings.iter().any(|w| w.contains("disagree")), "{:?}", aligned.warnings);

        let moved = Sky { background: 100.0, blobs: Vec::new(), stars: stars.iter().map(|&(x, y, f)| (x + 0.8, y, f)).collect() };
        let mut shifted = planes.clone();
        shifted[1] = vec![render(&moved, &panels[1], 1001)];
        let out = mosaic_of(&tiles, &shifted, &MosaicConfig::default());
        let pair = out.pair_residuals[0];
        assert!((pair.dx - 0.8).abs() < 0.1 && pair.dy.abs() < 0.1, "{pair:?}");
        let warning = out.warnings.iter().find(|w| w.contains("disagree")).expect("a 0.8 px residual is warned");
        assert!(warning.contains("'left'") && warning.contains("'right'"), "{warning}");
    }

    const RAY_SHAPES: [(usize, usize); 8] = [(1, 1), (1, 2), (2, 1), (2, 2), (1, 3), (3, 1), (1, 4), (4, 1)];
    const INTEGER_LAYOUT: [(f64, f64); 3] = [(0.0, 0.0), (24.0, 16.0), (48.0, 8.0)];
    const FRACTIONAL_LAYOUT: [(f64, f64); 3] = [(0.0, 0.0), (24.37, 16.81), (48.62, 8.45)];

    fn rejecting() -> MosaicConfig {
        MosaicConfig { reject: Some(RejectSpec::default()), ..grid_canvas() }
    }

    fn sky_points(sky: &Sky) -> Vec<(f64, f64)> {
        sky.stars.iter().map(|&(x, y, _)| (x, y)).collect()
    }

    fn trio_of(layout: [(f64, f64); 3]) -> Vec<Panel> {
        ["a", "b", "c"].iter().zip(layout).map(|(&name, (x0, y0))| panel(name, x0, y0, 0.0)).collect()
    }

    struct Rays<'a> {
        inside: &'a dyn Fn(f64, f64) -> bool,
        stars: &'a [(f64, f64)],
        sigmas: (f64, f64),
    }

    fn add_cosmic_rays(
        image: &mut Array2<f32>,
        panel: &Panel,
        seed: u64,
        count: usize,
        rays: &Rays,
        taken: &mut Vec<(f64, f64)>,
    ) -> Vec<(usize, usize)> {
        let mut rng = StdRng::seed_from_u64(seed);
        let (rows, cols) = image.dim();
        let mut hits = Vec::new();
        let mut placed = 0;
        while placed < count {
            let (h, w) = RAY_SHAPES[rng.gen_range(0..RAY_SHAPES.len())];
            let (y0, x0) = (rng.gen_range(0..rows - h), rng.gen_range(0..cols - w));
            let pixels: Vec<(usize, usize)> = (y0..y0 + h).flat_map(|y| (x0..x0 + w).map(move |x| (y, x))).collect();
            let on_sky = |&(y, x): &(usize, usize)| (panel.x0 + x as f64, panel.y0 + y as f64);
            let clear = pixels.iter().map(on_sky).all(|(sx, sy)| {
                (rays.inside)(sx, sy)
                    && rays.stars.iter().all(|&(px, py)| (px - sx).hypot(py - sy) >= 8.0)
                    && taken.iter().all(|&(tx, ty)| (tx - sx).hypot(ty - sy) >= 3.0)
            });
            if clear {
                for p in &pixels {
                    image[*p] += (rng.gen_range(rays.sigmas.0..rays.sigmas.1) * NOISE) as f32;
                }
                taken.extend(pixels.iter().map(on_sky));
                hits.extend(pixels);
                placed += 1;
            }
        }
        hits
    }

    fn flagged_sets(out: &MosaicOutput) -> Vec<HashSet<(usize, usize)>> {
        out.rejections.iter().map(|r| r.pixels.iter().copied().collect()).collect()
    }

    fn caught(hits: &[Vec<(usize, usize)>], flagged: &[HashSet<(usize, usize)>]) -> usize {
        hits.iter().zip(flagged).map(|(h, f)| h.iter().filter(|p| f.contains(p)).count()).sum()
    }

    fn near_a_star(stars: &[(f64, f64, f64)], (dx, dy): (f64, f64), panel: &Panel, (y, x): (usize, usize), radius: f64) -> bool {
        stars.iter().any(|&(sx, sy, _)| (sx + dx - panel.x0 - x as f64).hypot(sy + dy - panel.y0 - y as f64) <= radius)
    }

    fn expected_rejection_ram(estimate: &MosaicEstimate, tiles: &[TileGeometry]) -> u64 {
        let planes = tiles.iter().map(|t| t.planes as u64).max().unwrap();
        let canvas = (estimate.cols * estimate.rows) as u64 * (3 + planes);
        let in_flight = tiles
            .iter()
            .zip(&estimate.tiles)
            .map(|(t, row)| (t.width * t.height) as u64 * (planes + 3) + (row.window.cols * row.window.rows) as u64 * (planes + 1))
            .max()
            .unwrap();
        4 * (canvas + in_flight)
    }

    type StageLog = Vec<(MosaicStage, usize, usize)>;

    fn recorded_mosaic(tiles: &[TileGeometry], planes: &[Vec<Array2<f32>>], cfg: &MosaicConfig) -> (MosaicOutput, Vec<usize>, StageLog) {
        let calls = Mutex::new(Vec::new());
        let stages = Mutex::new(Vec::new());
        let load = |k: usize| -> Result<TilePixels> {
            calls.lock().unwrap().push(k);
            Ok(TilePixels { planes: planes[k].clone(), excluded: None })
        };
        let record = |stage: MosaicStage, done: usize, total: usize| {
            if stage != MosaicStage::Residuals {
                stages.lock().unwrap().push((stage, done, total));
            }
        };
        let out = run_mosaic_with(tiles, cfg, &load, None, &never_cancelled, &record).unwrap();
        (out, calls.into_inner().unwrap(), stages.into_inner().unwrap())
    }

    fn stage_run(stage: MosaicStage, total: usize) -> impl Iterator<Item = (MosaicStage, usize, usize)> {
        (1..=total).map(move |k| (stage, k, total))
    }

    #[test]
    fn cosmic_rays_at_coverage_3_rejected_99_percent() {
        let blobs = vec![Blob { x: 170.0, y: 130.0, amplitude: 300.0, width: 60.0 }];
        let sky = Sky { background: 100.0, blobs, stars: scattered_stars(81, (360.0, 260.0), 60) };
        let stars = sky_points(&sky);
        for (label, layout) in [("integer", INTEGER_LAYOUT), ("fractional", FRACTIONAL_LAYOUT)] {
            let panels = trio_of(layout);
            let (tiles, clean) = build(&sky, &panels);
            let (left, top) = layout.iter().fold((0.0f64, 0.0f64), |(x, y), o| (x.max(o.0), y.max(o.1)));
            let inside = |sx: f64, sy: f64| sx >= left + 4.0 && sx < 296.0 && sy >= top + 4.0 && sy < 236.0;
            let rays = Rays { inside: &inside, stars: &stars, sigmas: (10.0, 30.0) };
            let mut planes = clean.clone();
            let mut taken = Vec::new();
            let hits: Vec<Vec<(usize, usize)>> =
                panels.iter().enumerate().map(|(k, p)| add_cosmic_rays(&mut planes[k][0], p, 8100 + k as u64, 40, &rays, &mut taken)).collect();
            let cfg = rejecting();
            let out = mosaic_of(&tiles, &planes, &cfg);
            let flagged = flagged_sets(&out);
            let injected: usize = hits.iter().map(Vec::len).sum();
            let found = caught(&hits, &flagged);
            assert!(found as f64 >= 0.99 * injected as f64, "{label}: {found} of {injected} cosmic-ray pixels rejected");
            let tested: u64 = out.rejections.iter().map(|r| r.tested_px).sum();
            let spurious = out.rejected_px as usize - found;
            assert!((spurious as f64) < 1e-3 * tested as f64, "{label}: {spurious} clean pixels rejected out of {tested} tested");
            assert_eq!(out.rejections.len(), 3);
            for (k, r) in out.rejections.iter().enumerate() {
                assert_eq!(r.rejected_px, r.pixels.len() as u64, "{label} tile {k}");
                assert!(r.tested_px > 40_000, "{label} tile {k}: {} px tested", r.tested_px);
                assert!(r.sigma > 0.8 * NOISE && r.sigma < 1.25 * NOISE, "{label} tile {k}: sigma {}", r.sigma);
            }
            assert_eq!(out.rejected_px, out.rejections.iter().map(|r| r.rejected_px).sum::<u64>());
            assert!(out.history.iter().any(|h| h.starts_with("Outliers rejected where 3 or more tiles overlap")), "{:?}", out.history);
            assert!(out.history.iter().any(|h| h.starts_with("Tile 2 'b':") && h.contains("px rejected")), "{:?}", out.history);
            if label == "integer" {
                let estimate = mosaic_estimate(&tiles, &cfg, None).unwrap();
                let reference = mosaic_of(&tiles, &clean, &grid_canvas());
                let unrejected = mosaic_of(&tiles, &planes, &grid_canvas());
                let tolerance = 2.5 * NOISE as f32;
                let (mut checked, mut dirty) = (0, 0);
                for (k, (h, f)) in hits.iter().zip(&flagged).enumerate() {
                    let w = estimate.tiles[k].window;
                    for &(y, x) in h.iter().filter(|p| f.contains(p)) {
                        let (cy, cx) = (w.y0 + y, w.x0 + x);
                        if (112..236).contains(&cx) && (80..176).contains(&cy) {
                            let (value, truth) = (out.planes[0][[cy, cx]], reference.planes[0][[cy, cx]]);
                            assert!((value - truth).abs() < tolerance, "tile {k} ({y}, {x}): {value} against {truth}");
                            dirty += usize::from((unrejected.planes[0][[cy, cx]] - truth).abs() >= tolerance);
                            checked += 1;
                        }
                    }
                }
                assert!(checked >= 20, "{checked} rejected pixels deep inside all three tiles");
                assert_eq!(dirty, checked, "without the rejection every one of them is off by a third of its cosmic ray");
            }
        }
    }

    #[test]
    fn star_cores_kept_under_0_3_px_misregistration() {
        let mut rng = StdRng::seed_from_u64(91);
        let stars: Vec<(f64, f64, f64)> =
            (0..45).map(|_| (rng.gen_range(56.0..292.0), rng.gen_range(24.0..232.0), rng.gen_range(5000.0..20_000.0))).collect();
        let shift = (0.24, 0.18);
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: stars.clone() };
        let moved = Sky { background: 100.0, blobs: Vec::new(), stars: stars.iter().map(|&(x, y, f)| (x + shift.0, y + shift.1, f)).collect() };
        for (label, layout) in [("integer", INTEGER_LAYOUT), ("fractional", FRACTIONAL_LAYOUT)] {
            let panels = trio_of(layout);
            let tiles: Vec<TileGeometry> = panels.iter().map(|p| grid_window(p.name, p.x0, p.y0, p.width, p.height)).collect();
            let planes: Vec<Vec<Array2<f32>>> =
                panels.iter().enumerate().map(|(k, p)| vec![render(if k == 2 { &moved } else { &sky }, p, 9100 + k as u64)]).collect();
            let cores = |out: &MosaicOutput| -> usize {
                out.rejections
                    .iter()
                    .enumerate()
                    .map(|(k, r)| {
                        let offset = if k == 2 { shift } else { (0.0, 0.0) };
                        r.pixels.iter().filter(|&&p| near_a_star(&stars, offset, &panels[k], p, 3.0)).count()
                    })
                    .sum()
            };
            let out = mosaic_of(&tiles, &planes, &rejecting());
            assert_eq!(cores(&out), 0, "{label}: star-core pixels rejected");
            let tested: u64 = out.rejections.iter().map(|r| r.tested_px).sum();
            assert!(tested > 150_000, "{label}: {tested} px tested");
            assert!((out.rejected_px as f64) < 1e-3 * tested as f64, "{label}: {} of {tested} px rejected", out.rejected_px);
            let bare = MosaicConfig { reject: Some(RejectSpec { snr: 3.5, scale: 0.0 }), ..grid_canvas() };
            let control = cores(&mosaic_of(&tiles, &planes, &bare));
            assert!(control >= 20, "{label}: without the gradient term the misregistered star cores are rejected ({control} px)");
        }
    }

    #[test]
    fn coverage_2_never_rejects() {
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: scattered_stars(93, (540.0, 360.0), 50) };
        let stars = sky_points(&sky);

        let pair = [panel("left", 0.0, 0.0, 0.0), panel("right", 150.0, 0.0, 0.0)];
        let (tiles, clean) = build(&sky, &pair);
        let overlap = |sx: f64, sy: f64| (154.0..296.0).contains(&sx) && (4.0..236.0).contains(&sy);
        let strong = Rays { inside: &overlap, stars: &stars, sigmas: (50.0, 80.0) };
        let mut planes = clean.clone();
        let mut taken = Vec::new();
        for (k, p) in pair.iter().enumerate() {
            add_cosmic_rays(&mut planes[k][0], p, 9300 + k as u64, 30, &strong, &mut taken);
        }
        let (out, calls, stages) = recorded_mosaic(&tiles, &planes, &rejecting());
        let (plain, plain_calls, plain_stages) = recorded_mosaic(&tiles, &planes, &grid_canvas());
        assert_eq!(out.rejected_px, 0);
        assert_eq!(out.rejections.len(), 2);
        assert!(
            out.rejections.iter().all(|r| r.tested_px == 0 && r.rejected_px == 0 && r.pixels.is_empty() && r.sigma.is_nan()),
            "{:?}",
            out.rejections
        );
        assert_eq!(out.coverage, plain.coverage);
        assert!(
            out.planes[0].iter().zip(&plain.planes[0]).all(|(a, b)| a == b || (a.is_nan() && b.is_nan())),
            "nothing changes where only two tiles overlap"
        );
        assert!(out.warnings.iter().any(|w| w.contains("3 or more tiles") && w.contains("nothing was rejected")), "{:?}", out.warnings);
        assert!(out.history.iter().any(|h| h.starts_with("Tile 1 'left':") && h.contains("no pixel")), "{:?}", out.history);
        assert!(plain.history.iter().any(|h| h == "Outlier rejection off"), "{:?}", plain.history);
        assert_eq!((calls.clone(), stages.clone()), (plain_calls, plain_stages), "two tiles never take the rejection passes");
        assert_eq!(calls, [0, 1, 0, 1]);
        let expected: StageLog =
            stage_run(MosaicStage::Overlaps, 2).chain([(MosaicStage::Levels, 1, 1)]).chain(stage_run(MosaicStage::Warp, 2)).collect();
        assert_eq!(stages, expected);
        let estimate = |cfg: &MosaicConfig| mosaic_estimate(&tiles, cfg, None).unwrap();
        assert_eq!(estimate(&rejecting()).ram_bytes, estimate(&grid_canvas()).ram_bytes);

        let trio = [panel("a", 0.0, 0.0, 0.0), panel("b", 150.0, 0.0, 0.0), panel("c", 225.0, 120.0, 0.0)];
        let (tiles, clean) = build(&sky, &trio);
        let triple = |sx: f64, sy: f64| (225.0..300.0).contains(&sx) && (120.0..240.0).contains(&sy);
        let pairs_only = |sx: f64, sy: f64| {
            ((154.0..296.0).contains(&sx) && (4.0..116.0).contains(&sy))
                || ((154.0..221.0).contains(&sx) && (124.0..236.0).contains(&sy))
                || ((304.0..446.0).contains(&sx) && (124.0..236.0).contains(&sy))
        };
        let corner = |sx: f64, sy: f64| (229.0..296.0).contains(&sx) && (124.0..236.0).contains(&sy);
        let doubles = Rays { inside: &pairs_only, stars: &stars, sigmas: (50.0, 80.0) };
        let triples = Rays { inside: &corner, stars: &stars, sigmas: (10.0, 30.0) };
        let mut planes = clean.clone();
        let mut taken = Vec::new();
        let mut double_hits = Vec::new();
        let mut triple_hits = Vec::new();
        for (k, p) in trio.iter().enumerate() {
            double_hits.push(add_cosmic_rays(&mut planes[k][0], p, 9400 + k as u64, 20, &doubles, &mut taken));
            triple_hits.push(add_cosmic_rays(&mut planes[k][0], p, 9500 + k as u64, 15, &triples, &mut taken));
        }
        let (out, calls, stages) = recorded_mosaic(&tiles, &planes, &rejecting());
        assert_eq!(calls, [0, 1, 2, 0, 1, 2, 0, 1, 2], "levels, the reference, then the test with the accumulation");
        let expected: StageLog = stage_run(MosaicStage::Overlaps, 3)
            .chain([(MosaicStage::Levels, 1, 1)])
            .chain(stage_run(MosaicStage::Reject, 3))
            .chain(stage_run(MosaicStage::Warp, 3))
            .collect();
        assert_eq!(stages, expected);
        let flagged = flagged_sets(&out);
        for (k, f) in flagged.iter().enumerate() {
            for &(y, x) in f {
                let (sx, sy) = (trio[k].x0 + x as f64, trio[k].y0 + y as f64);
                assert!(triple(sx, sy), "tile {k} ({y}, {x}) at ({sx}, {sy}) is rejected outside the three-tile corner");
            }
        }
        assert_eq!(caught(&double_hits, &flagged), 0, "cosmic rays where two tiles overlap stay");
        let injected: usize = triple_hits.iter().map(Vec::len).sum();
        let found = caught(&triple_hits, &flagged);
        assert!(found as f64 >= 0.99 * injected as f64, "{found} of {injected} cosmic-ray pixels rejected in the three-tile corner");
    }

    #[test]
    fn rgb_tiles_reject_on_the_luminance_in_every_plane() {
        let sky = Sky { background: 100.0, blobs: Vec::new(), stars: scattered_stars(97, (360.0, 260.0), 40) };
        let stars = sky_points(&sky);
        let panels = trio_of(INTEGER_LAYOUT);
        let (mut tiles, mono) = build(&sky, &panels);
        tiles.iter_mut().for_each(|t| t.planes = 3);
        let gains = [1.0f32, 0.5, 0.25];
        let luminance_gain: f64 = gains.iter().map(|&g| f64::from(g)).sum();
        let clean: Vec<Vec<Array2<f32>>> = mono.iter().map(|p| gains.iter().map(|&g| &p[0] * g).collect()).collect();
        let deep = |sx: f64, sy: f64| (112.0..236.0).contains(&sx) && (80.0..176.0).contains(&sy);
        let rays = Rays { inside: &deep, stars: &stars, sigmas: (15.0 * luminance_gain, 30.0 * luminance_gain) };
        let mut planes = clean.clone();
        let mut taken = Vec::new();
        let hits: Vec<Vec<(usize, usize)>> =
            panels.iter().enumerate().map(|(k, p)| add_cosmic_rays(&mut planes[k][1], p, 9700 + k as u64, 10, &rays, &mut taken)).collect();
        let out = mosaic_of(&tiles, &planes, &rejecting());
        let flagged = flagged_sets(&out);
        let injected: usize = hits.iter().map(Vec::len).sum();
        assert_eq!(caught(&hits, &flagged), injected, "a cosmic ray in the green plane is found on the luminance");
        let luminance_noise = NOISE * luminance_gain;
        for r in &out.rejections {
            assert!(r.sigma > 0.8 * luminance_noise && r.sigma < 1.25 * luminance_noise, "sigma {} against {luminance_noise}", r.sigma);
            assert!((r.rejected_px as f64) < 1e-3 * r.tested_px as f64 + injected as f64, "{r:?}");
        }
        let estimate = mosaic_estimate(&tiles, &rejecting(), None).unwrap();
        let reference = mosaic_of(&tiles, &clean, &grid_canvas());
        for (k, h) in hits.iter().enumerate() {
            let w = estimate.tiles[k].window;
            for &(y, x) in h {
                for (c, &gain) in gains.iter().enumerate() {
                    let (value, truth) = (out.planes[c][[w.y0 + y, w.x0 + x]], reference.planes[c][[w.y0 + y, w.x0 + x]]);
                    assert!((value - truth).abs() < 2.5 * NOISE as f32 * gain, "tile {k} ({y}, {x}) plane {}: {value} against {truth}", c + 1);
                }
            }
        }
    }

    #[test]
    fn rejection_is_on_by_default_for_level_2_single_exposures_only() {
        let dir = tempfile::tempdir().unwrap();
        let plane = Array2::from_elem((30, 40), 5.0f32);
        let write = |name: &str, extra: &[(&str, String)]| -> String {
            let path = dir.path().join(name).to_string_lossy().into_owned();
            let mut cards = tan_cards(40, 30, (20.5, 15.5), (150.0, 30.0), rotated_cd(1.0, 0.0));
            cards.extend(extra.iter().cloned());
            write_fits_mono(&path, &plane, Some(&header(&cards))).unwrap();
            path
        };
        let paths = [
            write("jw01475001001_02101_00001_nrca1_cal.fits", &[]),
            write("jw01475001001_02101_00001_nrca1_crf.fits", &[]),
            write("j8pu0aabq_flt.fits", &[]),
            write("j8pu0aacq_FLC.fits", &[]),
            write("m31_panel_1.fits", &[]),
            write("renamed.fits", &[("FILENAME", "'jw02739001001_02105_00001_nrcb1_cal.fits'".to_string())]),
            write("stack_cal.fits", &[("NCOMBINE", "16".to_string())]),
        ];
        let tiles = load_tiles(&paths).unwrap();
        assert_eq!(tiles.iter().map(|t| t.single_exposure).collect::<Vec<_>>(), [true, true, true, true, false, true, false]);
        let on = Some(RejectSpec::default());
        assert_eq!(default_rejection(&tiles[..4]), on, "JWST _cal and _crf, HST _flt and _flc");
        assert_eq!(default_rejection(&tiles[5..6]), on, "FILENAME names the product of a renamed file");
        assert_eq!(default_rejection(&tiles[4..5]), None, "a panel that is not a Level-2 product");
        assert_eq!(default_rejection(&tiles[6..7]), None, "NCOMBINE above 1 is a stack");
        assert_eq!(default_rejection(&tiles), None, "one stacked panel turns the default off");
        assert_eq!(default_rejection(&[]), None);
        let plain = mosaic_estimate(&tiles[..3], &MosaicConfig::default(), None).unwrap();
        assert_eq!(plain.default_reject, on);
        assert_eq!(mosaic_estimate(&tiles[3..5], &MosaicConfig::default(), None).unwrap().default_reject, None);
        let with = mosaic_estimate(&tiles[..3], &MosaicConfig { reject: on, ..MosaicConfig::default() }, None).unwrap();
        assert_eq!(with.ram_bytes, expected_rejection_ram(&with, &tiles[..3]));
        assert!(with.ram_bytes > plain.ram_bytes, "{} against {}", with.ram_bytes, plain.ram_bytes);
        assert_eq!(plain.ram_bytes, expected_ram(&plain, &tiles[..3]));
        for spec in [RejectSpec { snr: 0.0, scale: 1.2 }, RejectSpec { snr: 3.5, scale: -1.0 }, RejectSpec { snr: f32::NAN, scale: 1.2 }] {
            let cfg = MosaicConfig { reject: Some(spec), ..MosaicConfig::default() };
            let err = format!("{:#}", run_mosaic(&tiles[..3], &cfg, &never_cancelled, &no_progress).unwrap_err());
            assert!(err.contains("snr") && err.contains("scale"), "{spec:?}: {err}");
        }
    }
}
