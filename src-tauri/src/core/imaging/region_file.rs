use serde::{Deserialize, Serialize};

use super::region::{
    image_to_physical_shape, physical_to_image_shape, shape_to_pixel, shape_to_sky, PhysicalMap, RegionError,
    RegionShape, RegionSystem,
};
use crate::core::astrometry::wcs::WcsTransform;

pub const DS9_HEADER: &str = "# Region file format: DS9 version 4.1";
pub const DS9_GLOBAL: &str = "global color=green dashlist=8 3 width=1 font=\"helvetica 10 normal roman\" select=1 highlite=1 dash=0 fixed=0 edit=1 move=1 delete=1 include=1 source=1";

const DEFAULT_COLOR: &str = "green";
const DEFAULT_WIDTH: u32 = 1;

const UNSUPPORTED_SYSTEMS: &[&str] = &[
    "fk4", "galactic", "ecliptic", "j2000", "b1950", "amplifier", "detector", "tile",
];
const UNSUPPORTED_SHAPES: &[&str] = &[
    "text", "vector", "ruler", "compass", "projection", "panda", "epanda", "bpanda", "composite",
];
const POINT_STYLES: &[&str] = &["circle", "box", "diamond", "cross", "x", "arrow", "boxcircle"];

fn default_include() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionProperties {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub dash: Option<bool>,
    #[serde(default = "default_include")]
    pub include: bool,
}

impl Default for RegionProperties {
    fn default() -> Self {
        Self { color: None, width: None, text: None, dash: None, include: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub shape: RegionShape,
    #[serde(default)]
    pub props: RegionProperties,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedRegions {
    pub regions: Vec<Region>,
    pub warnings: Vec<String>,
    pub systems: Vec<RegionSystem>,
}

fn opens_quote(prev: Option<char>) -> bool {
    matches!(prev, None | Some('=') | Some('(') | Some(',') | Some(' ') | Some('\t'))
}

fn split_outside(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
                cur.push(ch);
            }
            None => {
                if ch == '{' {
                    depth += 1;
                    cur.push(ch);
                } else if ch == '}' {
                    depth = depth.saturating_sub(1);
                    cur.push(ch);
                } else if (ch == '"' || ch == '\'') && depth == 0 && opens_quote(prev) {
                    quote = Some(ch);
                    cur.push(ch);
                } else if ch == sep && depth == 0 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.push(ch);
                }
            }
        }
        prev = Some(ch);
    }
    out.push(cur);
    out
}

fn split_comment(stmt: &str) -> (String, Option<String>) {
    let parts = split_outside(stmt, '#');
    let mut it = parts.into_iter();
    let head = it.next().unwrap_or_default();
    let rest: Vec<String> = it.collect();
    if rest.is_empty() {
        (head, None)
    } else {
        (head, Some(rest.join("#")))
    }
}

fn parse_prop_pairs(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect();
        if i >= chars.len() || chars[i] != '=' {
            continue;
        }
        i += 1;
        let value = if i < chars.len() && chars[i] == '{' {
            let vs = i + 1;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            let v: String = chars[vs..i.min(chars.len())].iter().collect();
            i += 1;
            v
        } else if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
            let q = chars[i];
            let vs = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != q {
                i += 1;
            }
            let v: String = chars[vs..i.min(chars.len())].iter().collect();
            i += 1;
            v
        } else {
            let vs = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            chars[vs..i].iter().collect()
        };
        if !key.is_empty() {
            out.push((key.to_ascii_lowercase(), value));
        }
    }
    out
}

fn parse_flag(v: &str) -> Option<bool> {
    match v.trim() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn apply_props(props: &mut RegionProperties, text: &str) {
    for (k, v) in parse_prop_pairs(text) {
        match k.as_str() {
            "color" => props.color = Some(v),
            "width" => {
                if let Ok(w) = v.trim().parse::<u32>() {
                    props.width = Some(w);
                }
            }
            "text" => props.text = Some(v),
            "dash" => {
                if let Some(b) = parse_flag(&v) {
                    props.dash = Some(b);
                }
            }
            "include" => {
                if let Some(b) = parse_flag(&v) {
                    props.include = b;
                }
            }
            _ => {}
        }
    }
}

fn invalid(line_no: usize, msg: impl std::fmt::Display) -> RegionError {
    RegionError::Invalid(format!("line {line_no}: {msg}"))
}

fn parse_sexagesimal(token: &str) -> Result<(f64, bool), RegionError> {
    let t = token.trim();
    let (sign, body) = match t.chars().next() {
        Some('-') => (-1.0, &t[1..]),
        Some('+') => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    let normalised: String = body
        .chars()
        .map(|c| match c {
            'h' | 'd' | 'm' | 's' | ':' => ':',
            other => other,
        })
        .collect();
    let parts: Vec<&str> = normalised.trim_end_matches(':').split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(RegionError::Invalid(format!("cannot parse sexagesimal '{token}'")));
    }
    let mut value = 0.0;
    let mut scale = 1.0;
    for p in &parts {
        let v: f64 = p
            .trim()
            .parse()
            .map_err(|_| RegionError::Invalid(format!("cannot parse sexagesimal '{token}'")))?;
        value += v * scale;
        scale /= 60.0;
    }
    let is_hours = body.contains('h') || (body.contains(':') && !body.contains('d'));
    Ok((sign * value, is_hours))
}

fn parse_plain(token: &str) -> Result<f64, RegionError> {
    token
        .trim()
        .parse::<f64>()
        .map_err(|_| RegionError::Invalid(format!("cannot parse number '{token}'")))
}

pub fn parse_lon(token: &str, system: RegionSystem) -> Result<f64, RegionError> {
    let t = token.trim();
    if !system.is_sky() {
        return parse_plain(t);
    }
    if t.contains(':') || t.contains('h') || t.contains('d') {
        let (v, is_hours) = parse_sexagesimal(t)?;
        return Ok(if is_hours { v * 15.0 } else { v });
    }
    parse_plain(t)
}

pub fn parse_lat(token: &str) -> Result<f64, RegionError> {
    let t = token.trim();
    if t.contains(':') || t.contains('d') {
        let (v, _) = parse_sexagesimal(t)?;
        return Ok(v);
    }
    parse_plain(t)
}

pub fn parse_size(token: &str, system: RegionSystem, physical: &PhysicalMap) -> Result<f64, RegionError> {
    let t = token.trim();
    let (body, unit) = match t.chars().last() {
        Some(c @ ('"' | '\'' | 'd' | 'r' | 'i' | 'p')) => (&t[..t.len() - c.len_utf8()], Some(c)),
        _ => (t, None),
    };
    let v = parse_plain(body)?;
    if system.is_sky() {
        return match unit {
            None | Some('d') => Ok(v * 3600.0),
            Some('"') => Ok(v),
            Some('\'') => Ok(v * 60.0),
            Some('r') => Ok(v.to_degrees() * 3600.0),
            Some(u) => Err(RegionError::Invalid(format!(
                "size '{token}' uses pixel unit '{u}' on a sky-system line"
            ))),
        };
    }
    match (system, unit) {
        (RegionSystem::Physical, Some('i')) => Ok(v / physical.scale()),
        (RegionSystem::Image, Some('p')) => Ok(v * physical.scale()),
        (_, None | Some('i') | Some('p')) => Ok(v),
        (_, Some(u)) => Err(RegionError::Invalid(format!(
            "size '{token}' uses angular unit '{u}' on an image-system line"
        ))),
    }
}

pub fn parse_angle_deg(token: &str) -> Result<f64, RegionError> {
    let t = token.trim();
    parse_plain(t.strip_suffix('d').unwrap_or(t))
}

pub fn format_sexagesimal_lon(deg: f64) -> String {
    let hours = deg.rem_euclid(360.0) / 15.0;
    let mut total_ms = (hours * 3_600_000.0).round() as i64;
    total_ms = total_ms.rem_euclid(24 * 3_600_000);
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s_ms = total_ms % 60_000;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s_ms / 1000, s_ms % 1000)
}

pub fn format_sexagesimal_lat(deg: f64) -> String {
    let sign = if deg < 0.0 { '-' } else { '+' };
    let total_cs = (deg.abs() * 360_000.0).round() as i64;
    let d = total_cs / 360_000;
    let m = (total_cs % 360_000) / 6_000;
    let s_cs = total_cs % 6_000;
    format!("{}{:02}:{:02}:{:02}.{:02}", sign, d, m, s_cs / 100, s_cs % 100)
}

pub fn format_num(v: f64) -> String {
    let s = format!("{:.6}", v);
    let trimmed = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed
    }
}

struct ShapeLine {
    name: String,
    args: Vec<String>,
    exclude: bool,
}

fn tokenize_shape(stmt: &str, line_no: usize) -> Result<ShapeLine, RegionError> {
    let mut s = stmt.trim();
    let mut exclude = false;
    if let Some(rest) = s.strip_prefix('-') {
        exclude = true;
        s = rest.trim_start();
    } else if let Some(rest) = s.strip_prefix('+') {
        s = rest.trim_start();
    }
    let name_len = s.chars().take_while(|c| c.is_ascii_alphabetic() || *c == '_').count();
    if name_len == 0 {
        return Err(invalid(line_no, format!("cannot read a shape name from '{stmt}'")));
    }
    let mut name = s[..name_len].to_ascii_lowercase();
    let mut rest = s[name_len..].trim_start();
    if POINT_STYLES.contains(&name.as_str()) {
        let lower = rest.to_ascii_lowercase();
        if let Some(after) = lower.strip_prefix("point") {
            if after.is_empty() || after.starts_with('(') || after.starts_with(char::is_whitespace) {
                name = "point".to_string();
                rest = rest[5..].trim_start();
            }
        }
    }
    let inner = if let Some(open) = rest.strip_prefix('(') {
        let close = open
            .rfind(')')
            .ok_or_else(|| invalid(line_no, format!("missing ')' in '{stmt}'")))?;
        &open[..close]
    } else {
        rest
    };
    let args: Vec<String> = inner
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    Ok(ShapeLine { name, args, exclude })
}

fn need_args(line_no: usize, name: &str, args: &[String], n: usize) -> Result<(), RegionError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(invalid(line_no, format!("{name} expects {n} arguments, got {}", args.len())))
    }
}

struct ArgReader<'a> {
    system: RegionSystem,
    line_no: usize,
    args: &'a [String],
    physical: &'a PhysicalMap,
}

impl ArgReader<'_> {
    fn err(&self, e: RegionError) -> RegionError {
        match e {
            RegionError::Invalid(m) => invalid(self.line_no, m),
            other => other,
        }
    }

    fn lon(&self, i: usize) -> Result<f64, RegionError> {
        parse_lon(&self.args[i], self.system).map_err(|e| self.err(e))
    }

    fn lat(&self, i: usize) -> Result<f64, RegionError> {
        if self.system.is_sky() {
            parse_lat(&self.args[i]).map_err(|e| self.err(e))
        } else {
            parse_plain(&self.args[i]).map_err(|e| self.err(e))
        }
    }

    fn size(&self, i: usize) -> Result<f64, RegionError> {
        parse_size(&self.args[i], self.system, self.physical).map_err(|e| self.err(e))
    }

    fn angle(&self, i: usize) -> Result<f64, RegionError> {
        parse_angle_deg(&self.args[i]).map_err(|e| self.err(e))
    }
}

fn build_shapes(
    line: &ShapeLine,
    system: RegionSystem,
    physical: &PhysicalMap,
    line_no: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<RegionShape>, RegionError> {
    let r = ArgReader { system, line_no, args: &line.args, physical };
    let name = line.name.as_str();
    let n = line.args.len();
    let unsupported = |what: &str, warnings: &mut Vec<String>| {
        warnings.push(format!("line {line_no}: unsupported shape '{what}' skipped"));
        Ok(Vec::new())
    };
    match name {
        "circle" => {
            need_args(line_no, name, &line.args, 3)?;
            Ok(vec![RegionShape::Circle { x: r.lon(0)?, y: r.lat(1)?, r: r.size(2)? }])
        }
        "ellipse" => {
            if n > 5 {
                return unsupported("ellipse annulus", warnings);
            }
            if n != 4 && n != 5 {
                return Err(invalid(line_no, format!("ellipse expects 4 or 5 arguments, got {n}")));
            }
            let angle = if n == 5 { r.angle(4)? } else { 0.0 };
            Ok(vec![RegionShape::Ellipse { x: r.lon(0)?, y: r.lat(1)?, rx: r.size(2)?, ry: r.size(3)?, angle }])
        }
        "box" => {
            if n > 5 {
                return unsupported("box annulus", warnings);
            }
            if n != 4 && n != 5 {
                return Err(invalid(line_no, format!("box expects 4 or 5 arguments, got {n}")));
            }
            let angle = if n == 5 { r.angle(4)? } else { 0.0 };
            Ok(vec![RegionShape::Box { x: r.lon(0)?, y: r.lat(1)?, width: r.size(2)?, height: r.size(3)?, angle }])
        }
        "annulus" => {
            if n < 4 {
                return Err(invalid(line_no, format!("annulus expects at least 4 arguments, got {n}")));
            }
            let x = r.lon(0)?;
            let y = r.lat(1)?;
            let mut radii = Vec::with_capacity(n - 2);
            for i in 2..n {
                radii.push(r.size(i)?);
            }
            Ok(radii
                .windows(2)
                .map(|w| RegionShape::Annulus { x, y, r_inner: w[0], r_outer: w[1] })
                .collect())
        }
        "polygon" => {
            if n < 6 || !n.is_multiple_of(2) {
                return Err(invalid(line_no, format!("polygon expects an even number (>= 6) of arguments, got {n}")));
            }
            let mut points = Vec::with_capacity(n / 2);
            for i in (0..n).step_by(2) {
                points.push([r.lon(i)?, r.lat(i + 1)?]);
            }
            Ok(vec![RegionShape::Polygon { points }])
        }
        "line" => {
            need_args(line_no, name, &line.args, 4)?;
            Ok(vec![RegionShape::Line { x1: r.lon(0)?, y1: r.lat(1)?, x2: r.lon(2)?, y2: r.lat(3)? }])
        }
        "point" => {
            need_args(line_no, name, &line.args, 2)?;
            Ok(vec![RegionShape::Point { x: r.lon(0)?, y: r.lat(1)? }])
        }
        other if UNSUPPORTED_SHAPES.contains(&other) => unsupported(other, warnings),
        other => unsupported(other, warnings),
    }
}

pub fn parse_reg_with_physical(
    text: &str,
    wcs: Option<&WcsTransform>,
    physical: &PhysicalMap,
) -> Result<ParsedRegions, RegionError> {
    let mut regions = Vec::new();
    let mut warnings = Vec::new();
    let mut systems: Vec<RegionSystem> = Vec::new();
    let mut system = RegionSystem::Image;
    let mut defaults = RegionProperties::default();

    for (idx, raw_line) in text.split('\n').enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end_matches('\r');
        for stmt in split_outside(line, ';') {
            let stmt = stmt.trim();
            if stmt.is_empty() || stmt.starts_with('#') {
                continue;
            }
            let lower = stmt.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("global") {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    apply_props(&mut defaults, &stmt[6..]);
                    continue;
                }
            }
            if lower.chars().all(|c| c.is_ascii_alphanumeric()) {
                if UNSUPPORTED_SYSTEMS.contains(&lower.as_str()) {
                    return Err(RegionError::UnsupportedSystem(lower));
                }
                if let Ok(sys) = RegionSystem::parse(&lower) {
                    system = sys;
                    continue;
                }
            }
            let (shape_part, props_part) = split_comment(stmt);
            let parsed = tokenize_shape(&shape_part, line_no)?;
            let shapes = build_shapes(&parsed, system, physical, line_no, &mut warnings)?;
            if shapes.is_empty() {
                continue;
            }
            let mut props = defaults.clone();
            if let Some(p) = &props_part {
                apply_props(&mut props, p);
            }
            if parsed.exclude {
                props.include = false;
            }
            if !systems.contains(&system) {
                systems.push(system);
            }
            for shape in shapes {
                let pixel = match system {
                    RegionSystem::Physical => physical_to_image_shape(&shape, physical)?.from_ds9_image(),
                    _ if system.is_sky() => shape_to_pixel(&shape, system, wcs)?,
                    _ => shape.from_ds9_image(),
                };
                regions.push(Region { shape: pixel, props: props.clone() });
            }
        }
    }

    Ok(ParsedRegions { regions, warnings, systems })
}

fn text_prop(text: &str) -> String {
    let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if !clean.contains(['{', '}']) {
        format!("text={{{clean}}}")
    } else if !clean.contains('"') {
        format!("text=\"{clean}\"")
    } else if !clean.contains('\'') {
        format!("text='{clean}'")
    } else {
        format!("text={{{}}}", clean.replace(['{', '}'], ""))
    }
}

fn props_suffix(props: &RegionProperties) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = &props.color {
        if c != DEFAULT_COLOR {
            parts.push(format!("color={c}"));
        }
    }
    if let Some(w) = props.width {
        if w != DEFAULT_WIDTH {
            parts.push(format!("width={w}"));
        }
    }
    if props.dash == Some(true) {
        parts.push("dash=1".to_string());
    }
    if let Some(t) = &props.text {
        parts.push(text_prop(t));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" # {}", parts.join(" "))
    }
}

struct Formatter {
    sky: bool,
    sexagesimal: bool,
}

impl Formatter {
    fn pos(&self, x: f64, y: f64) -> String {
        if !self.sky {
            format!("{},{}", format_num(x), format_num(y))
        } else if self.sexagesimal {
            format!("{},{}", format_sexagesimal_lon(x), format_sexagesimal_lat(y))
        } else {
            format!("{:.6},{:.6}", x, y)
        }
    }

    fn size(&self, v: f64) -> String {
        if self.sky {
            format!("{}\"", format_num(v))
        } else {
            format_num(v)
        }
    }

    fn shape(&self, s: &RegionShape) -> String {
        match s {
            RegionShape::Circle { x, y, r } => format!("circle({},{})", self.pos(*x, *y), self.size(*r)),
            RegionShape::Ellipse { x, y, rx, ry, angle } => format!(
                "ellipse({},{},{},{})",
                self.pos(*x, *y),
                self.size(*rx),
                self.size(*ry),
                format_num(*angle)
            ),
            RegionShape::Box { x, y, width, height, angle } => format!(
                "box({},{},{},{})",
                self.pos(*x, *y),
                self.size(*width),
                self.size(*height),
                format_num(*angle)
            ),
            RegionShape::Annulus { x, y, r_inner, r_outer } => format!(
                "annulus({},{},{})",
                self.pos(*x, *y),
                self.size(*r_inner),
                self.size(*r_outer)
            ),
            RegionShape::Polygon { points } => {
                let args: Vec<String> = points.iter().map(|p| self.pos(p[0], p[1])).collect();
                format!("polygon({})", args.join(","))
            }
            RegionShape::Line { x1, y1, x2, y2 } => {
                format!("line({},{})", self.pos(*x1, *y1), self.pos(*x2, *y2))
            }
            RegionShape::Point { x, y } => format!("point({})", self.pos(*x, *y)),
        }
    }
}

pub fn write_reg_with_physical(
    regions: &[Region],
    system: RegionSystem,
    wcs: Option<&WcsTransform>,
    sexagesimal: bool,
    physical: &PhysicalMap,
) -> Result<String, RegionError> {
    if system.is_sky() && wcs.is_none() {
        return Err(RegionError::WcsRequired);
    }
    let fmt = Formatter { sky: system.is_sky(), sexagesimal };
    let mut lines = vec![DS9_HEADER.to_string(), DS9_GLOBAL.to_string(), system.name().to_string()];
    for region in regions {
        let shape = match system {
            RegionSystem::Physical => image_to_physical_shape(&region.shape.to_ds9_image(), physical)?,
            _ if system.is_sky() => shape_to_sky(&region.shape, system, wcs)?,
            _ => region.shape.to_ds9_image(),
        };
        let prefix = if region.props.include { "" } else { "-" };
        lines.push(format!("{prefix}{}{}", fmt.shape(&shape), props_suffix(&region.props)));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::frames::fk5_j2000_to_icrs;
    use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd, rotated_cd};

    fn parse_reg(text: &str, wcs: Option<&WcsTransform>) -> Result<ParsedRegions, RegionError> {
        parse_reg_with_physical(text, wcs, &PhysicalMap::identity())
    }

    fn write_reg(
        regions: &[Region],
        system: RegionSystem,
        wcs: Option<&WcsTransform>,
        sexagesimal: bool,
    ) -> Result<String, RegionError> {
        write_reg_with_physical(regions, system, wcs, sexagesimal, &PhysicalMap::identity())
    }

    const IMAGE_SAMPLE: &str = "# Region file format: DS9 version 4.1\n\
global color=green dashlist=8 3 width=1 font=\"helvetica 10 normal roman\" select=1 highlite=1 dash=0 fixed=0 edit=1 move=1 delete=1 include=1 source=1\n\
image\n\
circle(100,100,20) # color=red text={star A}\n\
ellipse(200,150,30,15,45)\n\
box(300,300,40,20,0) # width=2 dash=1\n\
annulus(120,120,5,10)\n\
polygon(10,10,50,10,50,50,10,50)\n\
line(0,0,100,100) # line=0 0\n\
point(64,64) # point=circle\n\
-circle(100,100,5)\n\
circle(20,20,3);box(30,30,4,4,0)\n";

    const FK5_SAMPLE: &str = "# Region file format: DS9 version 4.1\n\
global color=green dashlist=8 3 width=1 font=\"helvetica 10 normal roman\" select=1 highlite=1 dash=0 fixed=0 edit=1 move=1 delete=1 include=1 source=1\n\
fk5\n\
circle(10:00:30.000,+02:12:00.00,3.5\")\n\
box(150.1250,2.2000,0.5',0.25',30)\n\
circle(10h00m30s,+2d12m00s,5\")\n";

    fn north_up() -> WcsTransform {
        WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap()
    }

    fn defaults_from_global() -> RegionProperties {
        RegionProperties {
            color: Some("green".into()),
            width: Some(1),
            text: None,
            dash: Some(false),
            include: true,
        }
    }

    #[test]
    fn image_sample_parses_all_shapes_and_props() {
        let parsed = parse_reg(IMAGE_SAMPLE, None).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.systems, vec![RegionSystem::Image]);
        assert_eq!(parsed.regions.len(), 10);
        let r = &parsed.regions;
        assert_eq!(r[0].shape, RegionShape::Circle { x: 99.0, y: 99.0, r: 20.0 });
        assert_eq!(r[0].props.color.as_deref(), Some("red"));
        assert_eq!(r[0].props.text.as_deref(), Some("star A"));
        assert_eq!(r[0].props.width, Some(1));
        assert!(r[0].props.include);
        assert_eq!(r[1].shape, RegionShape::Ellipse { x: 199.0, y: 149.0, rx: 30.0, ry: 15.0, angle: 45.0 });
        assert_eq!(r[1].props, defaults_from_global());
        assert_eq!(r[2].shape, RegionShape::Box { x: 299.0, y: 299.0, width: 40.0, height: 20.0, angle: 0.0 });
        assert_eq!(r[2].props.width, Some(2));
        assert_eq!(r[2].props.dash, Some(true));
        assert_eq!(r[2].props.color.as_deref(), Some("green"));
        assert_eq!(r[3].shape, RegionShape::Annulus { x: 119.0, y: 119.0, r_inner: 5.0, r_outer: 10.0 });
        assert_eq!(
            r[4].shape,
            RegionShape::Polygon { points: vec![[9.0, 9.0], [49.0, 9.0], [49.0, 49.0], [9.0, 49.0]] }
        );
        assert_eq!(r[5].shape, RegionShape::Line { x1: -1.0, y1: -1.0, x2: 99.0, y2: 99.0 });
        assert_eq!(r[5].props, defaults_from_global());
        assert_eq!(r[6].shape, RegionShape::Point { x: 63.0, y: 63.0 });
        assert_eq!(r[7].shape, RegionShape::Circle { x: 99.0, y: 99.0, r: 5.0 });
        assert!(!r[7].props.include);
        assert_eq!(r[8].shape, RegionShape::Circle { x: 19.0, y: 19.0, r: 3.0 });
        assert_eq!(r[9].shape, RegionShape::Box { x: 29.0, y: 29.0, width: 4.0, height: 4.0, angle: 0.0 });
    }

    #[test]
    fn fk5_sample_maps_to_expected_pixels() {
        let wcs = north_up();
        let parsed = parse_reg(FK5_SAMPLE, Some(&wcs)).unwrap();
        assert_eq!(parsed.regions.len(), 3);
        assert_eq!(parsed.systems, vec![RegionSystem::Fk5]);
        let (ra, dec) = fk5_j2000_to_icrs(150.125, 2.2);
        let (ex, ey) = wcs.world_to_pixel(ra, dec);
        for (i, region) in parsed.regions.iter().enumerate() {
            let (x, y) = region.shape.centre();
            assert!((x - ex).abs() < 1e-6 && (y - ey).abs() < 1e-6, "region {i}: ({x},{y}) vs ({ex},{ey})");
        }
        match &parsed.regions[0].shape {
            RegionShape::Circle { r, .. } => assert!((r - 3.5).abs() < 1e-9),
            other => panic!("{other:?}"),
        }
        match &parsed.regions[1].shape {
            RegionShape::Box { width, height, angle, .. } => {
                assert!((width - 30.0).abs() < 1e-9 && (height - 15.0).abs() < 1e-9);
                assert!((angle - 30.0).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        match &parsed.regions[2].shape {
            RegionShape::Circle { r, .. } => assert!((r - 5.0).abs() < 1e-9),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn multi_radius_annulus_and_unsupported_shapes() {
        let text = "image\nannulus(50,50,2,4,6)\nvector(10,10,5,45) # vector=1\ntext(20,20) # text={hi}\ncircle(5,5,1)\n";
        let parsed = parse_reg(text, None).unwrap();
        assert_eq!(parsed.regions.len(), 3);
        assert_eq!(parsed.regions[0].shape, RegionShape::Annulus { x: 49.0, y: 49.0, r_inner: 2.0, r_outer: 4.0 });
        assert_eq!(parsed.regions[1].shape, RegionShape::Annulus { x: 49.0, y: 49.0, r_inner: 4.0, r_outer: 6.0 });
        assert_eq!(parsed.regions[2].shape, RegionShape::Circle { x: 4.0, y: 4.0, r: 1.0 });
        assert_eq!(
            parsed.warnings,
            vec![
                "line 3: unsupported shape 'vector' skipped".to_string(),
                "line 4: unsupported shape 'text' skipped".to_string(),
            ]
        );
        let parsed = parse_reg("image\nellipse(10,10,4,5,8,9,30)\nbox(1,1,2,2,4,4,0)\n", None).unwrap();
        assert!(parsed.regions.is_empty());
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings[0].contains("ellipse annulus"));
        assert!(parsed.warnings[1].contains("box annulus"));
    }

    #[test]
    fn systems_and_point_variants() {
        assert_eq!(parse_reg("galactic\ncircle(1,1,1)\n", None), Err(RegionError::UnsupportedSystem("galactic".into())));
        assert_eq!(parse_reg("fk5\ncircle(150,2,3\")\n", None), Err(RegionError::WcsRequired));
        let parsed = parse_reg("PHYSICAL\ncircle point(10,10)\nbox point 5 5\npoint(3,3) # point=cross\n", None).unwrap();
        assert_eq!(parsed.systems, vec![RegionSystem::Physical]);
        assert_eq!(parsed.regions.len(), 3);
        assert_eq!(parsed.regions[0].shape, RegionShape::Point { x: 9.0, y: 9.0 });
        assert_eq!(parsed.regions[1].shape, RegionShape::Point { x: 4.0, y: 4.0 });
        assert_eq!(parsed.regions[2].shape, RegionShape::Point { x: 2.0, y: 2.0 });
        let parsed = parse_reg("circle 10 10 3 # color=blue\n", None).unwrap();
        assert_eq!(parsed.regions[0].shape, RegionShape::Circle { x: 9.0, y: 9.0, r: 3.0 });
        assert_eq!(parsed.regions[0].props.color.as_deref(), Some("blue"));
        assert_eq!(parsed.regions[0].props.width, None);
        assert!(parse_reg("image\ncircle(1,2)\n", None).is_err());
        assert!(parse_reg("image\ncircle(a,b,c)\n", None).is_err());
        let parsed = parse_reg("image\ncircle(4,4,2) # include=0 text=\"a;b\"\n", None).unwrap();
        assert!(!parsed.regions[0].props.include);
        assert_eq!(parsed.regions[0].props.text.as_deref(), Some("a;b"));
    }

    #[test]
    fn token_parsers() {
        assert!((parse_lon("150.125", RegionSystem::Fk5).unwrap() - 150.125).abs() < 1e-12);
        assert!((parse_lon("10:00:30.0", RegionSystem::Icrs).unwrap() - 150.125).abs() < 1e-12);
        assert!((parse_lon("10h00m30.0s", RegionSystem::Fk5).unwrap() - 150.125).abs() < 1e-12);
        assert!((parse_lon("150d07m30s", RegionSystem::Fk5).unwrap() - 150.125).abs() < 1e-12);
        assert_eq!(parse_lon("100", RegionSystem::Image).unwrap(), 100.0);
        assert!(parse_lon("10:00:30", RegionSystem::Image).is_err());
        assert!((parse_lat("+02:12:00.0").unwrap() - 2.2).abs() < 1e-12);
        assert!((parse_lat("-2d12m00s").unwrap() + 2.2).abs() < 1e-12);
        assert!((parse_lat("-00:30:00").unwrap() + 0.5).abs() < 1e-12);
        assert_eq!(parse_lat("2.2").unwrap(), 2.2);
        let id = PhysicalMap::identity();
        assert_eq!(parse_size("3.5\"", RegionSystem::Fk5, &id).unwrap(), 3.5);
        assert_eq!(parse_size("0.5'", RegionSystem::Fk5, &id).unwrap(), 30.0);
        assert_eq!(parse_size("0.01d", RegionSystem::Fk5, &id).unwrap(), 36.0);
        assert!((parse_size("0.001", RegionSystem::Icrs, &id).unwrap() - 3.6).abs() < 1e-12);
        assert!(parse_size("3i", RegionSystem::Fk5, &id).is_err());
        assert!(parse_size("3p", RegionSystem::Fk5, &id).is_err());
        assert_eq!(parse_size("3", RegionSystem::Image, &id).unwrap(), 3.0);
        assert_eq!(parse_size("3i", RegionSystem::Image, &id).unwrap(), 3.0);
        assert_eq!(parse_size("3p", RegionSystem::Image, &id).unwrap(), 3.0);
        assert!(parse_size("3\"", RegionSystem::Image, &id).is_err());
        assert!(parse_size("3'", RegionSystem::Image, &id).is_err());
        assert_eq!(parse_angle_deg("30").unwrap(), 30.0);
        assert_eq!(parse_angle_deg("30d").unwrap(), 30.0);
        assert!(parse_angle_deg("x").is_err());
    }

    #[test]
    fn formatters() {
        assert_eq!(format_sexagesimal_lon(150.125), "10:00:30.000");
        assert_eq!(format_sexagesimal_lon(360.0), "00:00:00.000");
        assert_eq!(format_sexagesimal_lon(-15.0), "23:00:00.000");
        assert_eq!(format_sexagesimal_lat(-2.2), "-02:12:00.00");
        assert_eq!(format_sexagesimal_lat(2.2), "+02:12:00.00");
        assert_eq!(format_sexagesimal_lat(-0.5), "-00:30:00.00");
        assert_eq!(format_num(100.0), "100");
        assert_eq!(format_num(100.5), "100.5");
        assert_eq!(format_num(1.0 / 3.0), "0.333333");
        assert_eq!(format_num(-0.0000001), "0");
        assert_eq!(format_num(-2.25), "-2.25");
    }

    #[test]
    fn writer_layout_and_round_trip() {
        let parsed = parse_reg(IMAGE_SAMPLE, None).unwrap();
        let text = write_reg(&parsed.regions, RegionSystem::Image, None, true).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], DS9_HEADER);
        assert_eq!(lines[1], DS9_GLOBAL);
        assert_eq!(lines[2], "image");
        assert_eq!(lines[3], "circle(100,100,20) # color=red text={star A}");
        assert_eq!(lines[4], "ellipse(200,150,30,15,45)");
        assert_eq!(lines[5], "box(300,300,40,20,0) # width=2 dash=1");
        assert_eq!(lines[8], "line(0,0,100,100)");
        assert_eq!(lines[9], "point(64,64)");
        assert_eq!(lines[10], "-circle(100,100,5)");
        let again = parse_reg(&text, None).unwrap();
        assert_eq!(again.regions, parsed.regions);
        assert!(again.warnings.is_empty());
        assert_eq!(write_reg(&parsed.regions, RegionSystem::Fk5, None, true), Err(RegionError::WcsRequired));
    }

    #[test]
    fn sky_round_trip_on_rotated_header() {
        let wcs = WcsTransform::from_header(&header_with_cd(rotated_cd(30.0))).unwrap();
        let parsed = parse_reg(IMAGE_SAMPLE, None).unwrap();
        for sexagesimal in [true, false] {
            for system in [RegionSystem::Fk5, RegionSystem::Icrs] {
                let text = write_reg(&parsed.regions, system, Some(&wcs), sexagesimal).unwrap();
                assert_eq!(text.lines().nth(2), Some(system.name()));
                assert!(text.lines().nth(3).unwrap().ends_with("\") # color=red text={star A}"), "{text}");
                let back = parse_reg(&text, Some(&wcs)).unwrap();
                assert_eq!(back.regions.len(), parsed.regions.len());
                let tol = if sexagesimal { 2e-2 } else { 5e-3 };
                for (a, b) in parsed.regions.iter().zip(&back.regions) {
                    assert_eq!(a.props, b.props);
                    assert_eq!(a.shape.kind(), b.shape.kind());
                    let (ax, ay) = a.shape.centre();
                    let (bx, by) = b.shape.centre();
                    assert!((ax - bx).abs() < tol && (ay - by).abs() < tol, "{sexagesimal} {system:?}: {a:?} vs {b:?}");
                    let ja = serde_json::to_value(&a.shape).unwrap();
                    let jb = serde_json::to_value(&b.shape).unwrap();
                    for key in ["r", "rx", "ry", "width", "height", "r_inner", "r_outer"] {
                        if let (Some(va), Some(vb)) = (ja.get(key).and_then(|v| v.as_f64()), jb.get(key).and_then(|v| v.as_f64())) {
                            assert!((va - vb).abs() < 1e-5, "{key}: {va} vs {vb}");
                        }
                    }
                    if let (Some(va), Some(vb)) = (ja.get("angle").and_then(|v| v.as_f64()), jb.get("angle").and_then(|v| v.as_f64())) {
                        assert!((va - vb).abs() < 1e-5, "angle: {va} vs {vb}");
                    }
                }
            }
        }
        let text = write_reg(&parsed.regions, RegionSystem::Fk5, Some(&wcs), true).unwrap();
        let sexa = text.lines().nth(3).unwrap();
        assert!(sexa.starts_with("circle("), "{sexa}");
        let inner = &sexa["circle(".len()..sexa.find(')').unwrap()];
        let parts: Vec<&str> = inner.split(',').collect();
        assert_eq!(parts[0].matches(':').count(), 2);
        assert!(parts[1].starts_with('+') || parts[1].starts_with('-'));
    }

    #[test]
    fn physical_regions_follow_the_cutout_ltv() {
        let parent = crate::core::imaging::region::test_support::make_header(&[("NAXIS1", "400"), ("NAXIS2", "400")]);
        let rect = crate::core::imaging::cutout::CutoutRect { x0: 100, y0: 20, width: 80, height: 60 };
        let cutout_header = crate::core::imaging::cutout::shift_header(&parent, &rect);
        let map = PhysicalMap::from_header(&cutout_header);
        let text = "physical\ncircle(150,50,5)\nbox(121,31,4,2,30)\n";
        let parsed = parse_reg_with_physical(text, None, &map).unwrap();
        assert_eq!(parsed.systems, vec![RegionSystem::Physical]);
        assert_eq!(parsed.regions[0].shape, RegionShape::Circle { x: 49.0, y: 29.0, r: 5.0 });
        assert_eq!(
            parsed.regions[1].shape,
            RegionShape::Box { x: 20.0, y: 10.0, width: 4.0, height: 2.0, angle: 30.0 }
        );
        let written = write_reg_with_physical(&parsed.regions, RegionSystem::Physical, None, true, &map).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines[2], "physical");
        assert_eq!(lines[3], "circle(150,50,5)");
        assert_eq!(lines[4], "box(121,31,4,2,30)");
        let binned = crate::core::imaging::region::test_support::make_header(&[
            ("LTV1", "-10"),
            ("LTV2", "-10"),
            ("LTM1_1", "0.5"),
            ("LTM2_2", "0.5"),
        ]);
        let parsed = parse_reg_with_physical("physical\ncircle(62,62,8)\n", None, &PhysicalMap::from_header(&binned)).unwrap();
        assert_eq!(parsed.regions[0].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 4.0 });
        let as_image = write_reg(&parsed.regions, RegionSystem::Image, None, true).unwrap();
        assert_eq!(as_image.lines().nth(2), Some("image"));
        assert_eq!(as_image.lines().nth(3), Some("circle(21,21,4)"));
    }

    #[test]
    fn size_units_convert_between_image_and_physical_pixels() {
        let binned = PhysicalMap::from_header(&crate::core::imaging::region::test_support::make_header(&[
            ("LTV1", "-10"),
            ("LTV2", "-10"),
            ("LTM1_1", "0.5"),
            ("LTM2_2", "0.5"),
        ]));
        let text = "physical\ncircle(62,62,8i)\ncircle(62,62,8p)\ncircle(62,62,8)\nannulus(62,62,2i,6)\n";
        let parsed = parse_reg_with_physical(text, None, &binned).unwrap();
        assert_eq!(parsed.regions[0].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 8.0 });
        assert_eq!(parsed.regions[1].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 4.0 });
        assert_eq!(parsed.regions[2].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 4.0 });
        assert_eq!(parsed.regions[3].shape, RegionShape::Annulus { x: 20.0, y: 20.0, r_inner: 2.0, r_outer: 3.0 });

        let parsed = parse_reg_with_physical("image\ncircle(21,21,8p)\nbox(21,21,8p,6i,0)\ncircle(21,21,5)\n", None, &binned).unwrap();
        assert_eq!(parsed.regions[0].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 4.0 });
        assert_eq!(
            parsed.regions[1].shape,
            RegionShape::Box { x: 20.0, y: 20.0, width: 4.0, height: 6.0, angle: 0.0 }
        );
        assert_eq!(parsed.regions[2].shape, RegionShape::Circle { x: 20.0, y: 20.0, r: 5.0 });

        let unbinned = parse_reg("physical\ncircle(21,21,8i)\nimage\ncircle(21,21,8p)\n", None).unwrap();
        assert!(unbinned.regions.iter().all(|r| r.shape == RegionShape::Circle { x: 20.0, y: 20.0, r: 8.0 }));
    }

    #[test]
    fn labels_with_braces_survive_a_round_trip() {
        for label in ["NGC 1275 {core}", "a}b", "x{y", "say \"hi\" {now}", "both \"'\" {}", "tab\there"] {
            let region = Region {
                shape: RegionShape::Circle { x: 9.0, y: 9.0, r: 2.0 },
                props: RegionProperties { text: Some(label.to_string()), ..RegionProperties::default() },
            };
            let text = write_reg(std::slice::from_ref(&region), RegionSystem::Image, None, true).unwrap();
            let back = parse_reg(&text, None).unwrap();
            assert_eq!(back.regions.len(), 1, "{text}");
            let got = back.regions[0].props.text.clone().unwrap_or_default();
            let expected = match label {
                "both \"'\" {}" => "both \"'\" ".to_string(),
                "tab\there" => "tab here".to_string(),
                other => other.to_string(),
            };
            assert_eq!(got, expected, "{text}");
            assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        }
    }

    #[test]
    fn region_json_shape() {
        let r = Region {
            shape: RegionShape::Circle { x: 99.0, y: 99.0, r: 20.0 },
            props: RegionProperties { color: Some("red".into()), width: None, text: Some("star A".into()), dash: None, include: true },
        };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(
            j,
            serde_json::json!({"shape":{"shape":"circle","x":99.0,"y":99.0,"r":20.0},"props":{"color":"red","width":null,"text":"star A","dash":null,"include":true}})
        );
        let back: Region = serde_json::from_str(r#"{"shape":{"shape":"point","x":1,"y":2}}"#).unwrap();
        assert_eq!(back.props, RegionProperties::default());
        let back: Region = serde_json::from_str(r#"{"shape":{"shape":"point","x":1,"y":2},"props":{}}"#).unwrap();
        assert!(back.props.include);
    }
}
