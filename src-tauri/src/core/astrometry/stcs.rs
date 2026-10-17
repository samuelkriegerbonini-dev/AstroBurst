use serde::Serialize;

use super::frames::{cartesian_to_spherical, spherical_to_cartesian};
use super::wcs::angular_separation;
use crate::core::imaging::region_file::{DS9_GLOBAL, DS9_HEADER};

pub const MAX_VERTICES_PER_SHAPE: usize = 4096;
pub const CIRCLE_VERTICES: usize = 64;
const MAX_NESTING: usize = 8;

const ACCEPTED_FRAMES: &[&str] = &["ICRS", "FK5", "J2000"];
const REFUSED_FRAMES: &[&str] = &[
    "GALACTIC",
    "GALACTIC_I",
    "GALACTIC_II",
    "SUPER_GALACTIC",
    "ECLIPTIC",
    "FK4",
    "B1950",
    "GEO_C",
    "GEO_D",
    "UNKNOWNFRAME",
    "UNITSPHERE",
    "CARTESIAN1",
    "CARTESIAN2",
    "CARTESIAN3",
    "SPHERICAL3",
];
const SKIPPED_PREFIX_TOKENS: &[&str] = &[
    "SPHERICAL2",
    "GEOCENTER",
    "BARYCENTER",
    "HELIOCENTER",
    "TOPOCENTER",
    "LSR",
    "LSRK",
    "LSRD",
    "GALACTIC_CENTER",
    "LOCAL_GROUP_CENTER",
    "MOON",
    "EMBARYCENTER",
    "MERCURY",
    "VENUS",
    "MARS",
    "JUPITER",
    "SATURN",
    "URANUS",
    "NEPTUNE",
    "PLUTO",
    "RELOCATABLE",
    "UNKNOWNREFPOS",
];
const SUPPORTED_SHAPES: &[&str] = &["POLYGON", "CIRCLE", "BOX", "UNION"];
const UNSUPPORTED_SHAPES: &[&str] = &[
    "INTERSECTION",
    "NOT",
    "DIFFERENCE",
    "POSITION",
    "CONVEX",
    "ELLIPSE",
    "SECTOR",
];

#[derive(Debug, Clone, PartialEq)]
pub enum StcsShape {
    Polygon(Vec<(f64, f64)>),
    Circle {
        ra: f64,
        dec: f64,
        radius_deg: f64,
    },
    Box {
        ra: f64,
        dec: f64,
        width_deg: f64,
        height_deg: f64,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StcsError {
    #[error("the STC-S region is empty")]
    Empty,
    #[error("unexpected STC-S token '{0}'")]
    UnexpectedToken(String),
    #[error("STC-S coordinate system '{0}' is not supported; only ICRS, FK5 and J2000 sky coordinates are")]
    UnsupportedFrame(String),
    #[error("STC-S shape '{0}' is not supported; only POLYGON, CIRCLE, BOX and UNION are")]
    UnsupportedShape(String),
    #[error("STC-S {shape} expects {expected} values, got {found}")]
    WrongArity {
        shape: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("STC-S POLYGON needs an even number of at least 6 values, got {0}")]
    BadPolygon(usize),
    #[error("STC-S POLYGON has {count} vertices; at most {max} are accepted")]
    TooManyVertices { count: usize, max: usize },
    #[error("STC-S {what} {value} is out of range")]
    OutOfRange { what: &'static str, value: f64 },
    #[error("unbalanced parentheses in the STC-S region")]
    Unbalanced,
    #[error("the STC-S region nests deeper than {0} levels")]
    TooDeep(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundingCone {
    pub ra: f64,
    pub dec: f64,
    pub radius_deg: f64,
}

pub fn parse_stcs(text: &str) -> Result<Vec<StcsShape>, StcsError> {
    let mut parser = Parser {
        tokens: tokenize(text),
        pos: 0,
    };
    let mut shapes = Vec::new();
    parser.shapes(0, &mut shapes)?;
    if parser.pos < parser.tokens.len() {
        return Err(StcsError::Unbalanced);
    }
    if shapes.is_empty() {
        return Err(StcsError::Empty);
    }
    Ok(shapes)
}

fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while !rest.is_empty() {
            let cut = match rest.find(['(', ')']) {
                Some(0) => 1,
                Some(i) => i,
                None => rest.len(),
            };
            tokens.push(&rest[..cut]);
            rest = &rest[cut..];
        }
    }
    tokens
}

fn number(token: &str) -> Option<f64> {
    let first = *token.as_bytes().first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    token.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn is_keyword(token: &str, list: &[&str]) -> bool {
    list.iter().any(|k| token.eq_ignore_ascii_case(k))
}

fn ends_values(token: &str) -> bool {
    token == "("
        || token == ")"
        || is_keyword(token, SUPPORTED_SHAPES)
        || is_keyword(token, UNSUPPORTED_SHAPES)
}

fn unexpected(token: Option<&str>) -> StcsError {
    StcsError::UnexpectedToken(token.unwrap_or("end of text").to_string())
}

struct Parser<'a> {
    tokens: Vec<&'a str>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.pos).copied()
    }

    fn shapes(&mut self, depth: usize, out: &mut Vec<StcsShape>) -> Result<(), StcsError> {
        if depth > MAX_NESTING {
            return Err(StcsError::TooDeep(MAX_NESTING));
        }
        while let Some(token) = self.peek() {
            if token == ")" {
                return Ok(());
            }
            self.pos += 1;
            if token == "(" {
                self.group(depth + 1, out)?;
            } else if token.eq_ignore_ascii_case("POLYGON") {
                let values = self.values()?;
                out.push(polygon(&values)?);
            } else if token.eq_ignore_ascii_case("CIRCLE") {
                let values = self.values()?;
                out.push(circle(&values)?);
            } else if token.eq_ignore_ascii_case("BOX") {
                let values = self.values()?;
                out.push(rectangle(&values)?);
            } else if token.eq_ignore_ascii_case("UNION") {
                self.prefix()?;
                if self.peek() != Some("(") {
                    return Err(unexpected(self.peek()));
                }
                self.pos += 1;
                self.group(depth + 1, out)?;
            } else if is_keyword(token, UNSUPPORTED_SHAPES) {
                return Err(StcsError::UnsupportedShape(token.to_string()));
            } else {
                return Err(unexpected(Some(token)));
            }
        }
        Ok(())
    }

    fn group(&mut self, depth: usize, out: &mut Vec<StcsShape>) -> Result<(), StcsError> {
        let before = out.len();
        self.shapes(depth, out)?;
        if self.peek() != Some(")") {
            return Err(StcsError::Unbalanced);
        }
        self.pos += 1;
        if out.len() == before {
            return Err(StcsError::Empty);
        }
        Ok(())
    }

    fn prefix(&mut self) -> Result<(), StcsError> {
        while let Some(token) = self.peek() {
            if number(token).is_some() || ends_values(token) {
                return Ok(());
            }
            if is_keyword(token, ACCEPTED_FRAMES) || is_keyword(token, SKIPPED_PREFIX_TOKENS) {
                self.pos += 1;
            } else if is_keyword(token, REFUSED_FRAMES) {
                return Err(StcsError::UnsupportedFrame(token.to_string()));
            } else if token.eq_ignore_ascii_case("FILLFACTOR") {
                self.pos += 1;
                if self.peek().and_then(number).is_none() {
                    return Err(unexpected(self.peek()));
                }
                self.pos += 1;
            } else {
                return Ok(());
            }
        }
        Ok(())
    }

    fn values(&mut self) -> Result<Vec<f64>, StcsError> {
        self.prefix()?;
        let mut values = Vec::new();
        while let Some(value) = self.peek().and_then(number) {
            values.push(value);
            self.pos += 1;
        }
        match self.peek() {
            Some(token) if !ends_values(token) => Err(unexpected(Some(token))),
            _ => Ok(values),
        }
    }
}

fn normalize_ra(ra: f64) -> f64 {
    let wrapped = ra.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        wrapped
    }
}

fn sky(ra: f64, dec: f64) -> Result<(f64, f64), StcsError> {
    if !(-90.0..=90.0).contains(&dec) {
        return Err(StcsError::OutOfRange {
            what: "declination",
            value: dec,
        });
    }
    Ok((normalize_ra(ra), dec))
}

fn size(what: &'static str, value: f64, max: f64, max_inclusive: bool) -> Result<f64, StcsError> {
    let below_max = if max_inclusive {
        value <= max
    } else {
        value < max
    };
    if value > 0.0 && below_max {
        Ok(value)
    } else {
        Err(StcsError::OutOfRange { what, value })
    }
}

fn arity(shape: &'static str, values: &[f64], expected: usize) -> Result<(), StcsError> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(StcsError::WrongArity {
            shape,
            expected,
            found: values.len(),
        })
    }
}

fn polygon(values: &[f64]) -> Result<StcsShape, StcsError> {
    if values.len() < 6 || !values.len().is_multiple_of(2) {
        return Err(StcsError::BadPolygon(values.len()));
    }
    let count = values.len() / 2;
    if count > MAX_VERTICES_PER_SHAPE {
        return Err(StcsError::TooManyVertices {
            count,
            max: MAX_VERTICES_PER_SHAPE,
        });
    }
    values
        .chunks_exact(2)
        .map(|p| sky(p[0], p[1]))
        .collect::<Result<_, _>>()
        .map(StcsShape::Polygon)
}

fn circle(values: &[f64]) -> Result<StcsShape, StcsError> {
    arity("CIRCLE", values, 3)?;
    let (ra, dec) = sky(values[0], values[1])?;
    let radius_deg = size("radius", values[2], 180.0, true)?;
    Ok(StcsShape::Circle {
        ra,
        dec,
        radius_deg,
    })
}

fn rectangle(values: &[f64]) -> Result<StcsShape, StcsError> {
    arity("BOX", values, 4)?;
    let (ra, dec) = sky(values[0], values[1])?;
    let width_deg = size("width", values[2], 180.0, false)?;
    let height_deg = size("height", values[3], 180.0, false)?;
    Ok(StcsShape::Box {
        ra,
        dec,
        width_deg,
        height_deg,
    })
}

impl StcsShape {
    pub fn to_polygon(&self, circle_vertices: usize) -> Vec<(f64, f64)> {
        match *self {
            StcsShape::Polygon(ref points) => points.clone(),
            StcsShape::Circle {
                ra,
                dec,
                radius_deg,
            } => circle_to_polygon(ra, dec, radius_deg, circle_vertices),
            StcsShape::Box {
                ra,
                dec,
                width_deg,
                height_deg,
            } => {
                let basis = TangentBasis::at(ra, dec);
                let east = (width_deg / 2.0).to_radians().tan();
                let north = (height_deg / 2.0).to_radians().tan();
                [
                    (-east, -north),
                    (east, -north),
                    (east, north),
                    (-east, north),
                ]
                .iter()
                .map(|&(e, n)| basis.point(1.0, e, n))
                .collect()
            }
        }
    }
}

struct TangentBasis {
    centre: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
}

impl TangentBasis {
    fn at(ra: f64, dec: f64) -> Self {
        let (sa, ca) = ra.to_radians().sin_cos();
        let (sd, cd) = dec.to_radians().sin_cos();
        Self {
            centre: [cd * ca, cd * sa, sd],
            east: [-sa, ca, 0.0],
            north: [-sd * ca, -sd * sa, cd],
        }
    }

    fn point(&self, along_centre: f64, east: f64, north: f64) -> (f64, f64) {
        let v = std::array::from_fn(|i| {
            along_centre * self.centre[i] + east * self.east[i] + north * self.north[i]
        });
        cartesian_to_spherical(v)
    }
}

pub fn circle_to_polygon(ra: f64, dec: f64, radius_deg: f64, n: usize) -> Vec<(f64, f64)> {
    let n = n.max(3);
    let basis = TangentBasis::at(ra, dec);
    let (sr, cr) = radius_deg.to_radians().sin_cos();
    (0..n)
        .map(|k| {
            let (st, ct) = (std::f64::consts::TAU * k as f64 / n as f64).sin_cos();
            basis.point(cr, sr * st, sr * ct)
        })
        .collect()
}

pub fn bounding_cone(shapes: &[StcsShape]) -> Option<BoundingCone> {
    let mut points: Vec<(f64, f64, f64)> = Vec::new();
    for shape in shapes {
        match *shape {
            StcsShape::Circle {
                ra,
                dec,
                radius_deg,
            } => points.push((ra, dec, radius_deg)),
            ref other => points.extend(
                other
                    .to_polygon(CIRCLE_VERTICES)
                    .into_iter()
                    .map(|(ra, dec)| (ra, dec, 0.0)),
            ),
        }
    }
    let &(first_ra, first_dec, _) = points.first()?;
    let sum = points.iter().fold([0.0; 3], |acc, &(ra, dec, _)| {
        let v = spherical_to_cartesian(ra, dec);
        [acc[0] + v[0], acc[1] + v[1], acc[2] + v[2]]
    });
    let norm = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    let (ra, dec) = if norm > 1e-9 * points.len() as f64 {
        cartesian_to_spherical(sum)
    } else {
        (first_ra, first_dec)
    };
    let radius_deg = points
        .iter()
        .map(|&(pra, pdec, extra)| angular_separation(ra, dec, pra, pdec) + extra)
        .fold(0.0, f64::max)
        .min(180.0);
    Some(BoundingCone {
        ra,
        dec,
        radius_deg,
    })
}

pub fn unwrap_ra_about(ra: f64, center_ra: f64) -> f64 {
    center_ra + (ra - center_ra + 180.0).rem_euclid(360.0) - 180.0
}

fn ds9_text_suffix(label: &str) -> String {
    let clean: String = label
        .chars()
        .filter(|c| !c.is_control() && *c != '{' && *c != '}')
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        String::new()
    } else {
        format!(" # text={{{clean}}}")
    }
}

pub fn to_ds9_sky_region_text(groups: &[(&str, &[StcsShape])]) -> String {
    let mut lines = vec![
        DS9_HEADER.to_string(),
        DS9_GLOBAL.to_string(),
        "icrs".to_string(),
    ];
    for &(label, shapes) in groups {
        let suffix = ds9_text_suffix(label);
        for shape in shapes {
            let coords: Vec<String> = shape
                .to_polygon(CIRCLE_VERTICES)
                .iter()
                .map(|(ra, dec)| format!("{ra},{dec}"))
                .collect();
            lines.push(format!("polygon({}){suffix}", coords.join(",")));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
    use crate::core::imaging::region::test_support::make_header;
    use crate::core::imaging::region::{shape_to_pixel, PhysicalMap, RegionShape, RegionSystem};
    use crate::core::imaging::region_file::{parse_reg_with_physical, DS9_GLOBAL, DS9_HEADER};

    struct FixtureLine {
        name: String,
        vertices: usize,
        text: String,
    }

    fn fixture_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast")
    }

    fn s_region_fixture() -> Vec<FixtureLine> {
        let text = std::fs::read_to_string(fixture_dir().join("s_region.txt")).unwrap();
        text.split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut parts = line.splitn(3, '\t');
                let name = parts.next().unwrap().to_string();
                let vertices = parts.next().unwrap().parse().unwrap();
                let text = parts.next().unwrap().to_string();
                FixtureLine {
                    name,
                    vertices,
                    text,
                }
            })
            .collect()
    }

    fn polygon_of(shape: &StcsShape) -> &[(f64, f64)] {
        match shape {
            StcsShape::Polygon(points) => points,
            other => panic!("expected a polygon, got {other:?}"),
        }
    }

    fn only_polygon(text: &str) -> Vec<(f64, f64)> {
        let shapes = parse_stcs(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(shapes.len(), 1, "{text}");
        polygon_of(&shapes[0]).to_vec()
    }

    fn square_polygon_text(keyword: &str, frame: &str) -> String {
        format!("{keyword} {frame} 10 20 11 20 11 21 10 21")
    }

    #[test]
    fn live_jwst_and_roman_strings_parse_to_one_polygon_with_their_vertex_count() {
        let lines = s_region_fixture();
        assert_eq!(lines.len(), 4);
        for line in &lines {
            let points = only_polygon(&line.text);
            assert_eq!(points.len(), line.vertices, "{}", line.name);
            assert!(points
                .iter()
                .all(|&(ra, dec)| (0.0..360.0).contains(&ra) && (-90.0..=90.0).contains(&dec)));
        }
        assert!(lines[0].text.starts_with("POLYGON ICRS  "));
        assert!(lines[3].text.ends_with(' '));
    }

    #[test]
    fn the_live_nircam_mosaic_vertices_are_read_exactly() {
        let line = &s_region_fixture()[0];
        assert_eq!(line.name, "jwst_m16_l3/jw02739-o001_t001_nircam");
        let points = only_polygon(&line.text);
        assert_eq!(points[0], (274.690362906, -13.894146555));
        assert_eq!(points[1], (274.690189140, -13.894139986));
        assert_eq!(points[77], (274.690406767, -13.892811377));
    }

    #[test]
    fn every_s_region_in_the_staged_mast_fixtures_parses_to_one_polygon() {
        let mut checked = 0;
        for name in [
            "jwst_m16_l3.json",
            "jwst_m16_l3_optical.json",
            "jwst_p2739.json",
            "c_wildcard.json",
            "c_nowild.json",
        ] {
            let text = std::fs::read_to_string(fixture_dir().join(name)).unwrap();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            for row in json["results"].as_array().unwrap() {
                let Some(region) = row["s_region"].as_str() else {
                    continue;
                };
                let numbers = region
                    .split_whitespace()
                    .filter(|t| t.parse::<f64>().is_ok())
                    .count();
                let points = only_polygon(region);
                assert_eq!(points.len() * 2, numbers, "{name}: {region}");
                checked += 1;
            }
        }
        assert!(
            checked >= 70,
            "only {checked} live s_region strings checked"
        );
    }

    #[test]
    fn keywords_and_frames_are_case_insensitive_and_whitespace_is_free() {
        let expected = vec![(10.0, 20.0), (11.0, 20.0), (11.0, 21.0), (10.0, 21.0)];
        for text in [
            square_polygon_text("POLYGON", "ICRS"),
            square_polygon_text("polygon", "icrs"),
            square_polygon_text("Polygon", "Icrs"),
            "  POLYGON\tICRS \t 10   20\n11 20\r\n 11 21    10 21  ".to_string(),
        ] {
            assert_eq!(only_polygon(&text), expected, "{text:?}");
        }
    }

    #[test]
    fn fk5_j2000_reference_positions_flavours_fill_factor_and_a_missing_frame_are_accepted() {
        let expected = vec![(10.0, 20.0), (11.0, 20.0), (11.0, 21.0), (10.0, 21.0)];
        for frame in [
            "FK5",
            "J2000",
            "ICRS GEOCENTER SPHERICAL2",
            "ICRS BARYCENTER",
            "fillfactor 0.5 ICRS",
            "UNKNOWNRefPos",
            "",
        ] {
            assert_eq!(
                only_polygon(&square_polygon_text("POLYGON", frame)),
                expected,
                "frame {frame:?}"
            );
        }
    }

    #[test]
    fn circle_and_box_are_parsed_with_their_sizes() {
        assert_eq!(
            parse_stcs("CIRCLE ICRS 274.7 -13.8 0.05").unwrap(),
            vec![StcsShape::Circle {
                ra: 274.7,
                dec: -13.8,
                radius_deg: 0.05
            }]
        );
        assert_eq!(
            parse_stcs("circle 274.7 -13.8 0.05").unwrap(),
            vec![StcsShape::Circle {
                ra: 274.7,
                dec: -13.8,
                radius_deg: 0.05
            }]
        );
        assert_eq!(
            parse_stcs("Box FK5 150 2 0.1 0.05").unwrap(),
            vec![StcsShape::Box {
                ra: 150.0,
                dec: 2.0,
                width_deg: 0.1,
                height_deg: 0.05
            }]
        );
    }

    #[test]
    fn union_forms_and_concatenated_shapes_are_flattened_in_order() {
        let first = vec![(10.0, 10.0), (11.0, 10.0), (11.0, 11.0)];
        let second = vec![(12.0, 10.0), (13.0, 10.0), (13.0, 11.0)];
        for text in [
            "UNION ICRS ( POLYGON 10 10 11 10 11 11 POLYGON 12 10 13 10 13 11 )",
            "Union ICRS (Polygon 10 10 11 10 11 11 Polygon 12 10 13 10 13 11)",
            "UNION (POLYGON ICRS 10 10 11 10 11 11) (POLYGON ICRS 12 10 13 10 13 11)",
            "POLYGON ICRS 10 10 11 10 11 11 POLYGON ICRS 12 10 13 10 13 11",
            "union icrs ( polygon 10 10 11 10 11 11 union ( polygon 12 10 13 10 13 11 ) )",
        ] {
            let shapes = parse_stcs(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(
                shapes,
                vec![
                    StcsShape::Polygon(first.clone()),
                    StcsShape::Polygon(second.clone())
                ],
                "{text}"
            );
        }
        let mixed =
            parse_stcs("UNION ICRS ( POLYGON 10 10 11 10 11 11 CIRCLE 12 10 0.5 BOX 14 10 1 1 )")
                .unwrap();
        assert_eq!(mixed.len(), 3);
        assert_eq!(
            mixed[1],
            StcsShape::Circle {
                ra: 12.0,
                dec: 10.0,
                radius_deg: 0.5
            }
        );
    }

    #[test]
    fn a_polygon_crossing_ra_zero_keeps_its_vertices_and_centres_its_cone_on_zero() {
        let points = only_polygon("POLYGON ICRS 359.9 -0.1 0.1 -0.1 0.1 0.1 -0.1 0.1");
        assert_eq!(points[0], (359.9, -0.1));
        assert_eq!(points[1], (0.1, -0.1));
        assert!((points[3].0 - 359.9).abs() < 1e-12);
        let cone = bounding_cone(&[StcsShape::Polygon(points.clone())]).unwrap();
        assert!(
            unwrap_ra_about(cone.ra, 0.0).abs() < 1e-9,
            "centre ra {}",
            cone.ra
        );
        assert!(cone.dec.abs() < 1e-9);
        let corner = angular_separation(0.0, 0.0, 0.1, 0.1);
        assert!(
            (cone.radius_deg - corner).abs() < 1e-9,
            "radius {}",
            cone.radius_deg
        );
        let unwrapped: Vec<f64> = points
            .iter()
            .map(|p| unwrap_ra_about(p.0, cone.ra))
            .collect();
        assert!(
            unwrapped.iter().all(|ra| ra.abs() < 0.1 + 1e-9),
            "{unwrapped:?}"
        );
    }

    #[test]
    fn a_malformed_token_is_named_in_the_error() {
        for (text, token) in [
            ("POLYGON ICRS 10 20 11 2O 11 21", "2O"),
            ("POLYGON ICRS 10 20 nan 20 11 21", "nan"),
            ("POLYGON ICRS 10 20 inf 20 11 21", "inf"),
            ("POLYGON ICRS 10 20 1e999 20 11 21", "1e999"),
            ("POLYGON ICRSX 10 20 11 20 11 21", "ICRSX"),
            ("POLYGON ICRS 10 20 11 20 11 21 ICRS", "ICRS"),
            ("10 20 11 20 11 21", "10"),
            ("UNION ICRS POLYGON 10 20 11 20 11 21", "POLYGON"),
        ] {
            let error = parse_stcs(text).unwrap_err();
            assert_eq!(
                error,
                StcsError::UnexpectedToken(token.to_string()),
                "{text}"
            );
            assert!(error.to_string().contains(token));
        }
    }

    #[test]
    fn galactic_ecliptic_and_cartesian_systems_are_refused_by_name() {
        for frame in ["GALACTIC", "ecliptic", "FK4", "UNKNOWNFrame", "CARTESIAN2"] {
            let error = parse_stcs(&square_polygon_text("POLYGON", frame)).unwrap_err();
            assert_eq!(error, StcsError::UnsupportedFrame(frame.to_string()));
            assert!(error.to_string().contains(frame));
        }
        assert_eq!(
            parse_stcs("CIRCLE GALACTIC 10 20 1").unwrap_err(),
            StcsError::UnsupportedFrame("GALACTIC".to_string())
        );
    }

    #[test]
    fn structural_errors_are_reported() {
        let cases: Vec<(String, StcsError)> = vec![
            ("".into(), StcsError::Empty),
            ("   \t ".into(), StcsError::Empty),
            ("UNION ICRS ( )".into(), StcsError::Empty),
            ("()".into(), StcsError::Empty),
            (
                "POLYGON ICRS 10 20 11 20 11".into(),
                StcsError::BadPolygon(5),
            ),
            ("POLYGON ICRS 10 20 11 20".into(), StcsError::BadPolygon(4)),
            ("POLYGON ICRS".into(), StcsError::BadPolygon(0)),
            (
                "CIRCLE ICRS 10 20".into(),
                StcsError::WrongArity {
                    shape: "CIRCLE",
                    expected: 3,
                    found: 2,
                },
            ),
            (
                "CIRCLE ICRS 10 20 1 2".into(),
                StcsError::WrongArity {
                    shape: "CIRCLE",
                    expected: 3,
                    found: 4,
                },
            ),
            (
                "BOX ICRS 10 20 1".into(),
                StcsError::WrongArity {
                    shape: "BOX",
                    expected: 4,
                    found: 3,
                },
            ),
            (
                "UNION ICRS ( POLYGON 10 20 11 20 11 21".into(),
                StcsError::Unbalanced,
            ),
            ("POLYGON 10 20 11 20 11 21 )".into(), StcsError::Unbalanced),
            (
                "INTERSECTION ICRS ( CIRCLE 10 20 1 CIRCLE 10 20.5 1 )".into(),
                StcsError::UnsupportedShape("INTERSECTION".into()),
            ),
            (
                "Not ( CIRCLE 10 20 1 )".into(),
                StcsError::UnsupportedShape("Not".into()),
            ),
            (
                "POLYGON ICRS 10 95 11 20 11 21".into(),
                StcsError::OutOfRange {
                    what: "declination",
                    value: 95.0,
                },
            ),
            (
                "CIRCLE ICRS 10 20 0".into(),
                StcsError::OutOfRange {
                    what: "radius",
                    value: 0.0,
                },
            ),
            (
                "CIRCLE ICRS 10 20 -1".into(),
                StcsError::OutOfRange {
                    what: "radius",
                    value: -1.0,
                },
            ),
            (
                "BOX ICRS 10 20 180 1".into(),
                StcsError::OutOfRange {
                    what: "width",
                    value: 180.0,
                },
            ),
            (
                "BOX ICRS 10 20 1 0".into(),
                StcsError::OutOfRange {
                    what: "height",
                    value: 0.0,
                },
            ),
            (
                format!(
                    "{}POLYGON 10 20 11 20 11 21{}",
                    "( ".repeat(9),
                    " )".repeat(9)
                ),
                StcsError::TooDeep(8),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_stcs(&text), Err(expected), "{text:?}");
        }
        let nested = format!(
            "{}POLYGON 10 20 11 20 11 21{}",
            "( ".repeat(8),
            " )".repeat(8)
        );
        assert_eq!(parse_stcs(&nested).unwrap().len(), 1);
    }

    #[test]
    fn ra_is_normalised_into_zero_to_three_sixty() {
        let points = only_polygon("POLYGON ICRS -0.5 10 360 10 720.25 11");
        assert!((points[0].0 - 359.5).abs() < 1e-12);
        assert_eq!(points[1].0, 0.0);
        assert!((points[2].0 - 0.25).abs() < 1e-12);
    }

    #[test]
    fn the_vertex_cap_is_enforced_per_shape() {
        let polygon = |n: usize| {
            let coords: Vec<String> = (0..n)
                .map(|i| {
                    format!(
                        "{} {}",
                        10.0 + i as f64 * 1e-4,
                        20.0 + (i % 2) as f64 * 1e-3
                    )
                })
                .collect();
            format!("POLYGON ICRS {}", coords.join(" "))
        };
        assert_eq!(
            only_polygon(&polygon(MAX_VERTICES_PER_SHAPE)).len(),
            MAX_VERTICES_PER_SHAPE
        );
        assert_eq!(
            parse_stcs(&polygon(MAX_VERTICES_PER_SHAPE + 1)),
            Err(StcsError::TooManyVertices {
                count: MAX_VERTICES_PER_SHAPE + 1,
                max: MAX_VERTICES_PER_SHAPE
            })
        );
    }

    #[test]
    fn circle_to_polygon_keeps_every_vertex_on_the_circle() {
        let points = circle_to_polygon(274.7, -13.8, 0.05, CIRCLE_VERTICES);
        assert_eq!(points.len(), CIRCLE_VERTICES);
        for &(ra, dec) in &points {
            assert!((angular_separation(274.7, -13.8, ra, dec) - 0.05).abs() < 1e-9);
        }
        assert!(
            points.iter().any(|p| p.1 > -13.8 + 0.049)
                && points.iter().any(|p| p.1 < -13.8 - 0.049)
        );
        let polar = circle_to_polygon(10.0, 90.0, 1.0, 8);
        assert_eq!(polar.len(), 8);
        for &(_, dec) in &polar {
            assert!((dec - 89.0).abs() < 1e-9);
        }
        let mut ras: Vec<f64> = polar.iter().map(|p| p.0).collect();
        ras.sort_by(f64::total_cmp);
        assert!(ras.windows(2).all(|w| w[1] - w[0] > 40.0), "{ras:?}");
        assert_eq!(circle_to_polygon(10.0, 20.0, 1.0, 2).len(), 3);
        let shape = StcsShape::Circle {
            ra: 274.7,
            dec: -13.8,
            radius_deg: 0.05,
        };
        assert_eq!(shape.to_polygon(CIRCLE_VERTICES), points);
    }

    #[test]
    fn box_corners_lie_where_the_great_circle_sides_meet() {
        let (ra, dec, width, height) = (150.0, 2.0, 0.1, 0.05);
        let corners = StcsShape::Box {
            ra,
            dec,
            width_deg: width,
            height_deg: height,
        }
        .to_polygon(CIRCLE_VERTICES);
        assert_eq!(corners.len(), 4);
        let half_w = (width / 2.0).to_radians().tan();
        let half_h = (height / 2.0).to_radians().tan();
        let diagonal = half_w.hypot(half_h).atan().to_degrees();
        for &(cra, cdec) in &corners {
            assert!((angular_separation(ra, dec, cra, cdec) - diagonal).abs() < 1e-9);
        }
        assert_eq!(corners.iter().filter(|c| c.1 > dec).count(), 2);
        assert_eq!(
            corners
                .iter()
                .filter(|c| unwrap_ra_about(c.0, ra) > ra)
                .count(),
            2
        );
        let polygon = vec![(1.0, 2.0), (3.0, 4.0), (5.0, 6.0)];
        assert_eq!(
            StcsShape::Polygon(polygon.clone()).to_polygon(CIRCLE_VERTICES),
            polygon
        );
    }

    #[test]
    fn the_bounding_cone_covers_every_vertex() {
        let shapes = parse_stcs(&s_region_fixture()[0].text).unwrap();
        let cone = bounding_cone(&shapes).unwrap();
        let points = polygon_of(&shapes[0]);
        let farthest = points
            .iter()
            .map(|p| angular_separation(cone.ra, cone.dec, p.0, p.1))
            .fold(0.0, f64::max);
        assert!(farthest <= cone.radius_deg + 1e-12);
        assert!((farthest - cone.radius_deg).abs() < 1e-12);
        assert!(
            cone.radius_deg > 0.01 && cone.radius_deg < 0.2,
            "radius {}",
            cone.radius_deg
        );
        assert!((274.6..274.8).contains(&cone.ra) && (-14.0..-13.7).contains(&cone.dec));

        let circle = bounding_cone(&[StcsShape::Circle {
            ra: 274.7,
            dec: -13.8,
            radius_deg: 0.05,
        }])
        .unwrap();
        assert!(angular_separation(circle.ra, circle.dec, 274.7, -13.8) < 1e-9);
        assert!((circle.radius_deg - 0.05).abs() < 1e-9);

        let mixed = [
            StcsShape::Circle {
                ra: 10.0,
                dec: 0.0,
                radius_deg: 0.5,
            },
            StcsShape::Circle {
                ra: 12.0,
                dec: 0.0,
                radius_deg: 0.5,
            },
        ];
        let both = bounding_cone(&mixed).unwrap();
        assert!(angular_separation(both.ra, both.dec, 11.0, 0.0) < 1e-9);
        assert!((both.radius_deg - 1.5).abs() < 1e-9);
        assert_eq!(bounding_cone(&[]), None);
        assert_eq!(
            serde_json::to_value(BoundingCone {
                ra: 1.0,
                dec: 2.0,
                radius_deg: 3.0
            })
            .unwrap(),
            serde_json::json!({ "ra": 1.0, "dec": 2.0, "radiusDeg": 3.0 })
        );
    }

    #[test]
    fn unwrap_ra_about_moves_ra_next_to_the_centre() {
        assert!((unwrap_ra_about(0.1, 359.95) - 360.1).abs() < 1e-12);
        assert!((unwrap_ra_about(359.9, 0.05) + 0.1).abs() < 1e-12);
        assert_eq!(unwrap_ra_about(10.0, 20.0), 10.0);
        assert_eq!(unwrap_ra_about(200.0, 20.0), -160.0);
    }

    #[test]
    fn ds9_sky_text_reimports_through_the_region_reader_within_a_micro_pixel() {
        let miri = parse_stcs(&s_region_fixture()[1].text).unwrap();
        let circle = parse_stcs("CIRCLE ICRS 274.74 -13.85 0.01").unwrap();
        let text = to_ds9_sky_region_text(&[
            ("jw02739-o002_t001_miri", miri.as_slice()),
            ("a{b}\nc", circle.as_slice()),
        ]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(&lines[..3], &[DS9_HEADER, DS9_GLOBAL, "icrs"]);
        assert_eq!(lines.len(), 5);
        assert!(lines[3].starts_with("polygon(274.72171577,-13.87856252,"));
        assert!(lines[3].ends_with(" # text={jw02739-o002_t001_miri}"));
        assert!(lines[4].ends_with(" # text={abc}"));
        assert!(text.ends_with('\n'));

        let scale = 0.11 / 3600.0;
        let header = make_header(&[
            ("NAXIS", "2"),
            ("NAXIS1", "2048"),
            ("NAXIS2", "2048"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRPIX1", "1024.5"),
            ("CRPIX2", "1024.5"),
            ("CRVAL1", "274.74"),
            ("CRVAL2", "-13.85"),
            ("CD1_1", &format!("{:.12e}", -scale)),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", &format!("{:.12e}", scale)),
        ]);
        let wcs = WcsTransform::from_header(&header).unwrap();
        let parsed = parse_reg_with_physical(&text, Some(&wcs), &PhysicalMap::identity()).unwrap();
        assert_eq!(parsed.regions.len(), 2);
        assert_eq!(
            parsed.regions[0].props.text.as_deref(),
            Some("jw02739-o002_t001_miri")
        );
        for (region, shape) in parsed.regions.iter().zip(miri.iter().chain(circle.iter())) {
            let sky: Vec<[f64; 2]> = shape
                .to_polygon(CIRCLE_VERTICES)
                .iter()
                .map(|p| [p.0, p.1])
                .collect();
            let expected = shape_to_pixel(
                &RegionShape::Polygon { points: sky },
                RegionSystem::Icrs,
                Some(&wcs),
            )
            .unwrap();
            let (RegionShape::Polygon { points: got }, RegionShape::Polygon { points: want }) =
                (&region.shape, &expected)
            else {
                panic!("expected polygons");
            };
            assert_eq!(got.len(), want.len());
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6,
                    "{g:?} vs {w:?}"
                );
            }
        }
    }
}
