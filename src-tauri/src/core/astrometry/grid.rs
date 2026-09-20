use serde::Serialize;

use super::frames::{convert_from_icrs, convert_to_icrs, SkyFrame};
use super::wcs::WcsTransform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GridKind {
    Lon,
    Lat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridLine {
    pub kind: GridKind,
    pub value_deg: f64,
    pub points: Vec<(f64, f64)>,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridLabel {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub edge: Edge,
    pub kind: GridKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct WcsGrid {
    pub frame: String,
    pub lines: Vec<GridLine>,
    pub labels: Vec<GridLabel>,
    pub lon_step_deg: f64,
    pub lat_step_deg: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepTable {
    Hours,
    Sexagesimal,
    Decimal,
}

pub const MIN_DENSITY: u8 = 1;
pub const MAX_DENSITY: u8 = 5;
pub const DEFAULT_DENSITY: u8 = 3;

pub const HOUR_STEPS_SECONDS: [f64; 17] = [
    21600.0, 10800.0, 7200.0, 3600.0, 1800.0, 1200.0, 900.0, 600.0, 300.0, 120.0, 60.0, 30.0,
    20.0, 10.0, 5.0, 2.0, 1.0,
];

pub const SEXAGESIMAL_STEPS_ARCSEC: [f64; 22] = [
    162000.0, 108000.0, 72000.0, 54000.0, 36000.0, 18000.0, 7200.0, 3600.0, 1800.0, 1200.0,
    900.0, 600.0, 300.0, 120.0, 60.0, 30.0, 20.0, 15.0, 10.0, 5.0, 2.0, 1.0,
];

pub const DECIMAL_STEPS_DEG: [f64; 20] = [
    45.0, 30.0, 20.0, 15.0, 10.0, 5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01, 0.005, 0.002,
    0.001, 0.0005, 0.0002, 0.0001,
];

const COARSE_SAMPLES_PER_AXIS: usize = 24;
const MAX_TRACE_SPACING_PX: f64 = 32.0;
const MIN_TRACE_POINTS: usize = 16;
const MIN_RING_POINTS: usize = 96;
const MAX_TRACE_POINTS: usize = 4096;
const MAX_LINES_PER_AXIS: usize = 64;
const RANGE_PADDING_FRACTION: f64 = 0.05;
const EDGE_EPSILON: f64 = 1e-6;

pub fn target_lines(density: u8) -> usize {
    3 + 2 * density.clamp(MIN_DENSITY, MAX_DENSITY) as usize
}

pub fn lon_step_table(frame: SkyFrame) -> StepTable {
    if frame.lon_in_hours() {
        StepTable::Hours
    } else {
        StepTable::Decimal
    }
}

pub fn step_values_deg(table: StepTable) -> Vec<f64> {
    match table {
        StepTable::Hours => HOUR_STEPS_SECONDS.iter().map(|s| s / 240.0).collect(),
        StepTable::Sexagesimal => SEXAGESIMAL_STEPS_ARCSEC.iter().map(|s| s / 3600.0).collect(),
        StepTable::Decimal => DECIMAL_STEPS_DEG.to_vec(),
    }
}

pub fn choose_step(table: StepTable, extent_deg: f64, target: usize) -> f64 {
    let steps = step_values_deg(table);
    let limit = target as f64 + 1e-9;
    let mut chosen = steps[0];
    for &step in &steps {
        if extent_deg.is_finite() && extent_deg / step <= limit {
            chosen = step;
        } else if extent_deg.is_finite() {
            break;
        }
    }
    chosen
}

pub fn format_lon_label(frame: SkyFrame, deg: f64, step_deg: f64) -> String {
    if frame.lon_in_hours() {
        format_lon_hours(deg, step_deg)
    } else {
        format_lon_decimal(deg, step_deg)
    }
}

pub fn format_lon_hours(deg: f64, step_deg: f64) -> String {
    let step_seconds = (step_deg * 240.0).round();
    let total = ((deg.rem_euclid(360.0)) * 240.0).round() as i64;
    let total = total.rem_euclid(86400);
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if step_seconds < 60.0 {
        format!("{h:02}h{m:02}m{s:02}s")
    } else {
        format!("{h:02}h{m:02}m")
    }
}

pub fn format_lon_decimal(deg: f64, step_deg: f64) -> String {
    let decimals = decimal_places(step_deg);
    let mut wrapped = deg.rem_euclid(360.0);
    let scale = 10f64.powi(decimals as i32);
    if (wrapped * scale).round() / scale >= 360.0 {
        wrapped = 0.0;
    }
    format!("{wrapped:.decimals$}°")
}

pub fn format_lat_label(deg: f64, step_deg: f64) -> String {
    let step_arcsec = (step_deg * 3600.0).round();
    let total = (deg.abs() * 3600.0).round() as i64;
    let sign = if deg < 0.0 && total > 0 { "-" } else { "+" };
    let d = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if step_arcsec < 60.0 {
        format!("{sign}{d:02}°{m:02}'{s:02}\"")
    } else {
        format!("{sign}{d:02}°{m:02}'")
    }
}

pub fn decimal_places(step_deg: f64) -> usize {
    if !step_deg.is_finite() || step_deg <= 0.0 {
        return 0;
    }
    for places in 0..=6usize {
        let scaled = step_deg * 10f64.powi(places as i32);
        if (scaled - scaled.round()).abs() < 1e-6 {
            return places;
        }
    }
    6
}

#[derive(Debug, Clone, Copy)]
struct ImageRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl ImageRect {
    fn new(naxis1: usize, naxis2: usize) -> Self {
        ImageRect {
            x0: -0.5,
            y0: -0.5,
            x1: naxis1 as f64 - 0.5,
            y1: naxis2 as f64 - 0.5,
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    fn edge_of(&self, x: f64, y: f64) -> Option<Edge> {
        if (x - self.x0).abs() < EDGE_EPSILON {
            Some(Edge::Left)
        } else if (x - self.x1).abs() < EDGE_EPSILON {
            Some(Edge::Right)
        } else if (y - self.y0).abs() < EDGE_EPSILON {
            Some(Edge::Top)
        } else if (y - self.y1).abs() < EDGE_EPSILON {
            Some(Edge::Bottom)
        } else {
            None
        }
    }

    fn clip_segment(&self, a: (f64, f64), b: (f64, f64)) -> Option<ClippedSegment> {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let mut t0 = 0.0f64;
        let mut t1 = 1.0f64;
        let bounds = [
            (-dx, a.0 - self.x0),
            (dx, self.x1 - a.0),
            (-dy, a.1 - self.y0),
            (dy, self.y1 - a.1),
        ];
        for (p, q) in bounds {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    if t > t1 {
                        return None;
                    }
                    if t > t0 {
                        t0 = t;
                    }
                } else {
                    if t < t0 {
                        return None;
                    }
                    if t < t1 {
                        t1 = t;
                    }
                }
            }
        }
        let lerp = |t: f64| (a.0 + dx * t, a.1 + dy * t);
        Some(ClippedSegment {
            start: lerp(t0),
            end: lerp(t1),
            entered: t0 > 0.0,
            exited: t1 < 1.0,
        })
    }
}

struct ClippedSegment {
    start: (f64, f64),
    end: (f64, f64),
    entered: bool,
    exited: bool,
}

struct Crossing {
    x: f64,
    y: f64,
    edge: Edge,
}

struct TracedLine {
    polylines: Vec<Vec<(f64, f64)>>,
    crossings: Vec<Crossing>,
}

fn wrap180(deg: f64) -> f64 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

fn linspace(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![lo];
    }
    (0..n)
        .map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64)
        .collect()
}

fn trace_point_count(arc_deg: f64, pixel_scale_deg: f64, min_points: usize) -> usize {
    let px = if pixel_scale_deg > 0.0 && arc_deg.is_finite() {
        arc_deg.abs() / pixel_scale_deg
    } else {
        0.0
    };
    let n = (px / MAX_TRACE_SPACING_PX).ceil() as usize + 1;
    n.clamp(min_points, MAX_TRACE_POINTS)
}

fn clip_polyline(pixels: &[(f64, f64)], rect: &ImageRect, jump_limit: f64) -> TracedLine {
    let mut polylines: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut crossings = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let flush = |current: &mut Vec<(f64, f64)>, polylines: &mut Vec<Vec<(f64, f64)>>| {
        if current.len() >= 2 {
            polylines.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for pair in pixels.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let finite = a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite();
        if !finite || (b.0 - a.0).hypot(b.1 - a.1) > jump_limit {
            flush(&mut current, &mut polylines);
            continue;
        }
        match rect.clip_segment(a, b) {
            None => flush(&mut current, &mut polylines),
            Some(seg) => {
                if seg.entered || current.is_empty() {
                    flush(&mut current, &mut polylines);
                    current.push(seg.start);
                    if seg.entered {
                        if let Some(edge) = rect.edge_of(seg.start.0, seg.start.1) {
                            crossings.push(Crossing { x: seg.start.0, y: seg.start.1, edge });
                        }
                    }
                }
                current.push(seg.end);
                if seg.exited {
                    if let Some(edge) = rect.edge_of(seg.end.0, seg.end.1) {
                        crossings.push(Crossing { x: seg.end.0, y: seg.end.1, edge });
                    }
                    flush(&mut current, &mut polylines);
                }
            }
        }
    }
    flush(&mut current, &mut polylines);
    TracedLine { polylines, crossings }
}

fn label_edge_preference(kind: GridKind) -> [Edge; 4] {
    match kind {
        GridKind::Lon => [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right],
        GridKind::Lat => [Edge::Left, Edge::Right, Edge::Bottom, Edge::Top],
    }
}

fn pick_label(kind: GridKind, text: &str, crossings: &[Crossing]) -> Option<GridLabel> {
    label_edge_preference(kind).iter().find_map(|edge| {
        crossings.iter().find(|c| c.edge == *edge).map(|c| GridLabel {
            x: c.x,
            y: c.y,
            text: text.to_string(),
            edge: *edge,
            kind,
        })
    })
}

fn sample_field(wcs: &WcsTransform, rect: &ImageRect, frame: SkyFrame) -> (Vec<(f64, f64)>, usize) {
    let xs = linspace(rect.x0, rect.x1, COARSE_SAMPLES_PER_AXIS + 1);
    let ys = linspace(rect.y0, rect.y1, COARSE_SAMPLES_PER_AXIS + 1);
    let mut pixels = Vec::with_capacity(xs.len() * ys.len());
    for &y in &ys {
        for &x in &xs {
            pixels.push((x, y));
        }
    }
    let mut invalid = 0usize;
    let samples = wcs
        .pixel_to_world_batch(&pixels)
        .into_iter()
        .filter_map(|c| {
            let (lon, lat) = convert_from_icrs(frame, c.ra, c.dec);
            if lon.is_finite() && lat.is_finite() {
                Some((lon, lat))
            } else {
                invalid += 1;
                None
            }
        })
        .collect();
    (samples, invalid)
}

fn field_centre(wcs: &WcsTransform, rect: &ImageRect, frame: SkyFrame) -> Option<(f64, f64)> {
    let c = wcs.pixel_to_world((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    let (lon, lat) = convert_from_icrs(frame, c.ra, c.dec);
    (lon.is_finite() && lat.is_finite()).then_some((lon, lat))
}

fn pole_inside(wcs: &WcsTransform, rect: &ImageRect, frame: SkyFrame, lon_c: f64) -> Option<f64> {
    [90.0f64, -90.0].into_iter().find(|&pole_lat| {
        let (ra, dec) = convert_to_icrs(frame, lon_c, pole_lat);
        let (x, y) = wcs.world_to_pixel(ra, dec);
        if !(x.is_finite() && y.is_finite() && rect.contains(x, y)) {
            return false;
        }
        let back = wcs.pixel_to_world(x, y);
        let (_, lat) = convert_from_icrs(frame, back.ra, back.dec);
        lat.is_finite() && (lat - pole_lat).abs() < 1e-6
    })
}

fn line_values(lo: f64, hi: f64, step: f64, half_open: bool) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || step <= 0.0 || hi < lo {
        return Vec::new();
    }
    let k0 = (lo / step).ceil() as i64;
    let k1 = (hi / step).floor() as i64;
    (k0..=k1)
        .map(|k| k as f64 * step)
        .filter(|v| !half_open || *v < hi - 1e-9)
        .take(MAX_LINES_PER_AXIS)
        .collect()
}

pub fn wcs_grid(
    wcs: &WcsTransform,
    naxis1: usize,
    naxis2: usize,
    frame: SkyFrame,
    density: u8,
) -> Result<WcsGrid, String> {
    if naxis1 < 2 || naxis2 < 2 {
        return Err(format!(
            "image dimensions {naxis1}x{naxis2} are too small for a coordinate grid"
        ));
    }
    let density = density.clamp(MIN_DENSITY, MAX_DENSITY);
    let target = target_lines(density);
    let rect = ImageRect::new(naxis1, naxis2);
    let (samples, invalid) = sample_field(wcs, &rect, frame);
    if samples.is_empty() {
        return Err("no pixel of the image projects onto the sky".into());
    }
    let mut notes = Vec::new();
    if invalid > 0 {
        notes.push(format!(
            "{invalid} of {} sampled pixels do not project onto the sky; the grid covers the valid part only",
            samples.len() + invalid
        ));
    }
    let (lon_c, _) = field_centre(wcs, &rect, frame).unwrap_or(samples[0]);

    let mut lon_min = f64::INFINITY;
    let mut lon_max = f64::NEG_INFINITY;
    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    for &(lon, lat) in &samples {
        let unwrapped = lon_c + wrap180(lon - lon_c);
        lon_min = lon_min.min(unwrapped);
        lon_max = lon_max.max(unwrapped);
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
    }

    let pole = pole_inside(wcs, &rect, frame, lon_c);
    if let Some(pole_lat) = pole {
        notes.push(format!(
            "{} pole of the {} frame lies inside the image; longitude lines converge on it and latitude rings surround it",
            if pole_lat > 0.0 { "north" } else { "south" },
            frame.name()
        ));
        lon_min = lon_c - 180.0;
        lon_max = lon_c + 180.0;
        if pole_lat > 0.0 {
            lat_max = 90.0;
        } else {
            lat_min = -90.0;
        }
    }

    let lon_extent = lon_max - lon_min;
    let lat_extent = lat_max - lat_min;
    let lon_step = choose_step(lon_step_table(frame), lon_extent, target);
    let lat_step = choose_step(StepTable::Sexagesimal, lat_extent, target);
    let pixel_scale_deg = wcs.pixel_scale_arcsec() / 3600.0;
    let padding = 2.0 * pixel_scale_deg;

    let (lon_lo, lon_hi) = if pole.is_some() {
        (lon_min, lon_max)
    } else {
        let pad = lon_extent * RANGE_PADDING_FRACTION + padding;
        (lon_min - pad, lon_max + pad)
    };
    let (lat_lo, lat_hi) = {
        let pad = lat_extent * RANGE_PADDING_FRACTION + padding;
        let mut lo = (lat_min - pad).max(-90.0);
        let mut hi = (lat_max + pad).min(90.0);
        match pole {
            Some(p) if p > 0.0 => hi = 90.0 - lat_step,
            Some(_) => lo = -90.0 + lat_step,
            None => {}
        }
        (lo, hi)
    };

    let jump_limit = 0.5 * naxis1.max(naxis2) as f64;
    let mut lines = Vec::new();
    let mut labels = Vec::new();

    for lon in line_values(lon_lo, lon_hi, lon_step, pole.is_some()) {
        let n = trace_point_count(lat_hi - lat_lo, pixel_scale_deg, MIN_TRACE_POINTS);
        let world: Vec<(f64, f64)> = linspace(lat_lo, lat_hi, n)
            .into_iter()
            .map(|lat| convert_to_icrs(frame, lon, lat))
            .collect();
        let pixels = wcs.world_to_pixel_batch(&world);
        let traced = clip_polyline(&pixels, &rect, jump_limit);
        if traced.polylines.is_empty() {
            continue;
        }
        let label = format_lon_label(frame, lon, lon_step);
        if let Some(l) = pick_label(GridKind::Lon, &label, &traced.crossings) {
            labels.push(l);
        }
        for points in traced.polylines {
            lines.push(GridLine {
                kind: GridKind::Lon,
                value_deg: lon.rem_euclid(360.0),
                points,
                label: label.clone(),
            });
        }
    }

    for lat in line_values(lat_lo, lat_hi, lat_step, false) {
        if lat.abs() >= 90.0 - 1e-9 {
            continue;
        }
        let min_points = if pole.is_some() { MIN_RING_POINTS } else { MIN_TRACE_POINTS };
        let arc = (lon_hi - lon_lo) * lat.to_radians().cos();
        let n = trace_point_count(arc, pixel_scale_deg, min_points);
        let world: Vec<(f64, f64)> = linspace(lon_lo, lon_hi, n)
            .into_iter()
            .map(|lon| convert_to_icrs(frame, lon, lat))
            .collect();
        let pixels = wcs.world_to_pixel_batch(&world);
        let traced = clip_polyline(&pixels, &rect, jump_limit);
        if traced.polylines.is_empty() {
            continue;
        }
        let label = format_lat_label(lat, lat_step);
        if let Some(l) = pick_label(GridKind::Lat, &label, &traced.crossings) {
            labels.push(l);
        }
        for points in traced.polylines {
            lines.push(GridLine {
                kind: GridKind::Lat,
                value_deg: lat,
                points,
                label: label.clone(),
            });
        }
    }

    Ok(WcsGrid {
        frame: frame.name().to_string(),
        lines,
        labels,
        lon_step_deg: lon_step,
        lat_step_deg: lat_step,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{make_header, north_up_cd, wcs_cards};
    use crate::types::header::HduHeader;

    const SIZE: usize = 200;

    fn header(cd: [[f64; 2]; 2], crval1: f64, crval2: f64) -> HduHeader {
        let mut cards = wcs_cards(cd);
        for (key, value) in cards.iter_mut() {
            match key.as_str() {
                "NAXIS1" | "NAXIS2" => *value = SIZE.to_string(),
                "CRPIX1" | "CRPIX2" => *value = format!("{}", SIZE as f64 / 2.0 + 0.5),
                "CRVAL1" => *value = format!("{crval1}"),
                "CRVAL2" => *value = format!("{crval2}"),
                _ => {}
            }
        }
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        make_header(&pairs)
    }

    fn grid_for(crval1: f64, crval2: f64, frame: SkyFrame, density: u8) -> WcsGrid {
        let h = header(north_up_cd(), crval1, crval2);
        let wcs = WcsTransform::from_header(&h).unwrap();
        wcs_grid(&wcs, SIZE, SIZE, frame, density).unwrap()
    }

    fn assert_points_inside(grid: &WcsGrid) {
        let max = SIZE as f64 + 0.5;
        for line in &grid.lines {
            assert!(line.points.len() >= 2, "polyline with {} points", line.points.len());
            for &(x, y) in &line.points {
                assert!(x.is_finite() && y.is_finite(), "non-finite point ({x},{y})");
                assert!(
                    (-1.5..=max).contains(&x) && (-1.5..=max).contains(&y),
                    "point ({x},{y}) is outside the image by more than 1 px"
                );
            }
        }
    }

    fn span<F: Fn(&(f64, f64)) -> f64>(points: &[(f64, f64)], f: F) -> f64 {
        let lo = points.iter().map(&f).fold(f64::INFINITY, f64::min);
        let hi = points.iter().map(&f).fold(f64::NEG_INFINITY, f64::max);
        hi - lo
    }

    #[test]
    fn north_up_field_gives_axis_aligned_lines_and_left_bottom_labels() {
        let grid = grid_for(150.0, 2.0, SkyFrame::Icrs, DEFAULT_DENSITY);
        assert_eq!(grid.frame, "icrs");
        assert!((grid.lon_step_deg * 240.0 - 2.0).abs() < 1e-9, "lon step {} deg", grid.lon_step_deg);
        assert!((grid.lat_step_deg * 3600.0 - 30.0).abs() < 1e-9, "lat step {} deg", grid.lat_step_deg);
        assert_points_inside(&grid);

        let lon_lines: Vec<&GridLine> = grid.lines.iter().filter(|l| l.kind == GridKind::Lon).collect();
        let lat_lines: Vec<&GridLine> = grid.lines.iter().filter(|l| l.kind == GridKind::Lat).collect();
        assert!(lon_lines.len() >= 5 && lon_lines.len() <= 9, "{} lon lines", lon_lines.len());
        assert!(lat_lines.len() >= 5 && lat_lines.len() <= 9, "{} lat lines", lat_lines.len());
        for line in &lon_lines {
            assert!(span(&line.points, |p| p.0) < 0.5, "RA line {} is not vertical", line.label);
            assert!(span(&line.points, |p| p.1) > 150.0, "RA line {} does not cross the image", line.label);
            assert!(line.label.contains('h') && line.label.contains('m') && line.label.ends_with('s'), "{}", line.label);
            assert!((0.0..360.0).contains(&line.value_deg));
        }
        for line in &lat_lines {
            assert!(span(&line.points, |p| p.1) < 0.5, "Dec line {} is not horizontal", line.label);
            assert!(span(&line.points, |p| p.0) > 150.0, "Dec line {} does not cross the image", line.label);
            assert!(line.label.starts_with('+') && line.label.contains('°') && line.label.ends_with('"'), "{}", line.label);
        }

        let left: Vec<&GridLabel> = grid.labels.iter().filter(|l| l.edge == Edge::Left).collect();
        let bottom: Vec<&GridLabel> = grid.labels.iter().filter(|l| l.edge == Edge::Bottom).collect();
        assert_eq!(left.len(), lat_lines.len(), "one left label per Dec line");
        assert_eq!(bottom.len(), lon_lines.len(), "one bottom label per RA line");
        assert!(grid.labels.iter().all(|l| l.edge == Edge::Left || l.edge == Edge::Bottom));
        for l in &left {
            assert_eq!(l.kind, GridKind::Lat);
            assert!((l.x + 0.5).abs() < 1e-6, "left label x {}", l.x);
            assert!(lat_lines.iter().any(|line| line.label == l.text));
        }
        for l in &bottom {
            assert_eq!(l.kind, GridKind::Lon);
            assert!((l.y - (SIZE as f64 - 0.5)).abs() < 1e-6, "bottom label y {}", l.y);
            assert!(lon_lines.iter().any(|line| line.label == l.text));
        }
        assert!(grid.labels.iter().any(|l| l.text == "10h00m00s"), "{:?}", grid.labels.iter().map(|l| &l.text).collect::<Vec<_>>());
        assert!(grid.labels.iter().any(|l| l.text == "+02°00'00\""));
        assert!(grid.notes.is_empty(), "{:?}", grid.notes);
    }

    #[test]
    fn field_crossing_ra_zero_keeps_lines_continuous() {
        let grid = grid_for(0.01, 2.0, SkyFrame::Icrs, DEFAULT_DENSITY);
        assert_points_inside(&grid);
        for line in &grid.lines {
            for pair in line.points.windows(2) {
                let d = ((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt();
                assert!(d < 100.0, "jump of {d} px in line {}", line.label);
            }
            assert!((0.0..360.0).contains(&line.value_deg), "value {}", line.value_deg);
        }
        let lon_labels: Vec<&str> = grid
            .labels
            .iter()
            .filter(|l| l.kind == GridKind::Lon)
            .map(|l| l.text.as_str())
            .collect();
        assert!(lon_labels.iter().any(|t| t.starts_with("23h59m")), "{lon_labels:?}");
        assert!(lon_labels.iter().any(|t| t.starts_with("00h00m")), "{lon_labels:?}");
        let lon_lines = grid.lines.iter().filter(|l| l.kind == GridKind::Lon).count();
        assert!(lon_lines >= 5, "{lon_lines} RA lines");
    }

    #[test]
    fn field_containing_the_north_pole_notes_it_and_stays_inside() {
        let grid = grid_for(150.0, 89.99, SkyFrame::Icrs, DEFAULT_DENSITY);
        assert!(grid.notes.iter().any(|n| n.contains("pole")), "{:?}", grid.notes);
        assert_points_inside(&grid);
        assert!(grid.lines.iter().any(|l| l.kind == GridKind::Lat), "no latitude rings");
        assert!(grid.lines.iter().any(|l| l.kind == GridKind::Lon), "no meridians");
        assert!((grid.lon_step_deg - 45.0).abs() < 1e-9, "lon step {}", grid.lon_step_deg);
        for line in grid.lines.iter().filter(|l| l.kind == GridKind::Lat) {
            assert!(line.value_deg < 90.0 && line.value_deg > 89.9, "ring at {}", line.value_deg);
        }
        for line in grid.lines.iter().filter(|l| l.kind == GridKind::Lon) {
            for pair in line.points.windows(2) {
                let d = ((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt();
                assert!(d < 100.0, "jump of {d} px in meridian {}", line.label);
            }
        }
    }

    #[test]
    fn galactic_frame_labels_longitude_in_decimal_degrees() {
        let grid = grid_for(150.0, 2.0, SkyFrame::Galactic, DEFAULT_DENSITY);
        assert_eq!(grid.frame, "galactic");
        assert!(
            DECIMAL_STEPS_DEG.iter().any(|s| (s - grid.lon_step_deg).abs() < 1e-12),
            "lon step {} is not from the decimal table",
            grid.lon_step_deg
        );
        assert!(grid.lon_step_deg <= 0.02 && grid.lon_step_deg >= 0.005, "lon step {}", grid.lon_step_deg);
        let decimals = decimal_places(grid.lon_step_deg);
        assert_points_inside(&grid);
        let lon_labels: Vec<&GridLabel> = grid.labels.iter().filter(|l| l.kind == GridKind::Lon).collect();
        let lat_labels: Vec<&GridLabel> = grid.labels.iter().filter(|l| l.kind == GridKind::Lat).collect();
        assert!(!lon_labels.is_empty() && !lat_labels.is_empty());
        for l in &lon_labels {
            assert!(l.text.ends_with('°') && !l.text.contains('h'), "{}", l.text);
            let value: f64 = l.text.trim_end_matches('°').parse().unwrap();
            assert!((0.0..360.0).contains(&value));
            assert_eq!(l.text.split('.').nth(1).map(|d| d.trim_end_matches('°').len()), Some(decimals), "{}", l.text);
        }
        for l in &lat_labels {
            assert!(l.text.starts_with('+') || l.text.starts_with('-'), "{}", l.text);
            assert!(l.text.contains('°') && l.text.contains('\''), "{}", l.text);
        }
        let lines_are_tilted = grid
            .lines
            .iter()
            .filter(|l| l.kind == GridKind::Lon)
            .any(|l| span(&l.points, |p| p.0) > 5.0);
        assert!(lines_are_tilted, "galactic meridians should not align with the pixel axes");
    }

    #[test]
    fn density_controls_the_number_of_lines() {
        let sparse = grid_for(150.0, 2.0, SkyFrame::Icrs, MIN_DENSITY);
        let dense = grid_for(150.0, 2.0, SkyFrame::Icrs, MAX_DENSITY);
        assert!(sparse.lat_step_deg > dense.lat_step_deg);
        assert!(sparse.lines.len() < dense.lines.len(), "{} vs {}", sparse.lines.len(), dense.lines.len());
        let clamped = grid_for(150.0, 2.0, SkyFrame::Icrs, 0);
        assert_eq!(clamped.lat_step_deg, sparse.lat_step_deg);
    }

    #[test]
    fn degenerate_dimensions_are_rejected() {
        let h = header(north_up_cd(), 150.0, 2.0);
        let wcs = WcsTransform::from_header(&h).unwrap();
        assert!(wcs_grid(&wcs, 0, SIZE, SkyFrame::Icrs, DEFAULT_DENSITY).is_err());
        assert!(wcs_grid(&wcs, SIZE, 1, SkyFrame::Icrs, DEFAULT_DENSITY).is_err());
    }

    #[test]
    fn step_selection_goldens() {
        let field = 200.12 / 3600.0;
        assert!((choose_step(StepTable::Hours, field, 9) * 240.0 - 2.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Sexagesimal, field, 9) * 3600.0 - 30.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Sexagesimal, field, 5) * 3600.0 - 60.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Sexagesimal, field, 13) * 3600.0 - 20.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Decimal, field, 9) - 0.01).abs() < 1e-12);
        assert!((choose_step(StepTable::Hours, 360.0, 9) - 45.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Sexagesimal, 1000.0, 9) - 45.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Sexagesimal, 0.0, 9) * 3600.0 - 1.0).abs() < 1e-9);
        assert!((choose_step(StepTable::Decimal, f64::NAN, 9) - 45.0).abs() < 1e-9);
        assert_eq!(target_lines(3), 9);
        assert_eq!(target_lines(0), 5);
        assert_eq!(target_lines(9), 13);
        assert_eq!(lon_step_table(SkyFrame::Icrs), StepTable::Hours);
        assert_eq!(lon_step_table(SkyFrame::Fk5J2000), StepTable::Hours);
        assert_eq!(lon_step_table(SkyFrame::Galactic), StepTable::Decimal);
        assert_eq!(lon_step_table(SkyFrame::EclipticJ2000), StepTable::Decimal);
    }

    #[test]
    fn label_formatting_goldens() {
        let two_seconds = 2.0 / 240.0;
        assert_eq!(format_lon_label(SkyFrame::Icrs, 150.0, two_seconds), "10h00m00s");
        assert_eq!(format_lon_label(SkyFrame::Icrs, 150.0 + 3.0 * two_seconds, two_seconds), "10h00m06s");
        assert_eq!(format_lon_label(SkyFrame::Fk5J2000, -1.0 / 240.0, 1.0 / 240.0), "23h59m59s");
        assert_eq!(format_lon_label(SkyFrame::Icrs, 150.0, 15.0), "10h00m");
        assert_eq!(format_lon_label(SkyFrame::Icrs, 150.0 + 0.5, 0.25), "10h02m");
        assert_eq!(format_lon_label(SkyFrame::Icrs, 360.0 - 1e-12, 1.0 / 240.0), "00h00m00s");
        assert_eq!(format_lon_label(SkyFrame::Galactic, 123.45, 0.01), "123.45°");
        assert_eq!(format_lon_label(SkyFrame::EclipticJ2000, 0.5, 0.5), "0.5°");
        assert_eq!(format_lon_label(SkyFrame::Galactic, 30.0, 5.0), "30°");
        assert_eq!(format_lon_label(SkyFrame::Galactic, -0.002, 0.002), "359.998°");
        assert_eq!(format_lon_label(SkyFrame::Galactic, 359.9999, 0.001), "0.000°");
        assert_eq!(format_lat_label(2.0 + 90.0 / 3600.0, 30.0 / 3600.0), "+02°01'30\"");
        assert_eq!(format_lat_label(-28.5, 1.0 / 60.0), "-28°30'");
        assert_eq!(format_lat_label(0.0, 1.0), "+00°00'");
        assert_eq!(format_lat_label(-1e-13, 1.0), "+00°00'");
        assert_eq!(format_lat_label(45.0, 45.0), "+45°00'");
        assert_eq!(format_lat_label(-0.5 - 1.0 / 3600.0, 1.0 / 3600.0), "-00°30'01\"");
        assert_eq!(decimal_places(0.5), 1);
        assert_eq!(decimal_places(0.02), 2);
        assert_eq!(decimal_places(5.0), 0);
        assert_eq!(decimal_places(0.0001), 4);
    }
}
