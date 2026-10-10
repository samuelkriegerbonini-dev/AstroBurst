use serde_yaml::Value;

use super::pipeline::{BoundingBox, FrameKind, GwcsFrame, GwcsPipeline, GwcsStep};
use super::transform::{ArithOp, Axis, Kind, Node, MAX_LANES};
use super::GwcsError;
use crate::infra::asdf::tree::{flatten_inline, untag, ArraySource, NdArrayMeta, WcsInfo};

pub type ArrayResolver<'a> = &'a dyn Fn(&Value) -> Option<(Vec<usize>, Vec<f64>)>;

#[derive(Debug, Clone, PartialEq)]
pub struct WcsNodeKey(pub String);

#[derive(Debug, Clone, PartialEq)]
pub enum WcsNodeClass {
    Imaging2D,
    FitsEquivalent,
    NotImaging2D(String),
}

pub const WCS_NODE_SEARCH_ORDER: [&str; 6] =
    ["meta.wcs", "roman.meta.wcs", "wcs", "gwcs", "roman.wcs_l2", "roman.wcs_l1"];

const PIXEL_UNITS: [&str; 3] = ["", "pixel", "pix"];
const ICRS_LIKE_REFERENCES: [&str; 3] = ["", "icrs", "fk5"];

pub fn tag_family(node: &Value) -> Option<String> {
    matches!(node, Value::Tagged(_)).then(|| WcsInfo::model_name(node))
}

pub fn wcs_node_at<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(tree, |node, part| node.get(part))
}

fn has_steps(node: &Value) -> bool {
    node.get("steps").and_then(Value::as_sequence).is_some()
}

pub fn find_wcs_node(tree: &Value) -> Option<(WcsNodeKey, &Value)> {
    WCS_NODE_SEARCH_ORDER.iter().find_map(|key| {
        wcs_node_at(tree, key)
            .filter(|node| has_steps(node))
            .map(|node| (WcsNodeKey(key.to_string()), node))
    })
}

pub fn missing_wcs_node() -> GwcsError {
    GwcsError::Parse(format!(
        "tree has no node with 'steps' under {}",
        join_with_or(&WCS_NODE_SEARCH_ORDER)
    ))
}

fn join_with_or(items: &[&str]) -> String {
    match items.split_last() {
        Some((last, head)) if !head.is_empty() => format!("{} or {last}", head.join(", ")),
        Some((last, _)) => last.to_string(),
        None => String::new(),
    }
}

pub fn is_fitswcs_imaging(transform: &Value) -> bool {
    if tag_family(transform).as_deref() == Some("fitswcs_imaging") {
        return true;
    }
    transform.get("crpix").is_some()
        && transform.get("crval").is_some()
        && transform.get("projection").is_some()
        && transform.get("forward").is_none()
}

pub fn classify_wcs_node(wcs: &Value) -> WcsNodeClass {
    let Some(steps) = wcs.get("steps").and_then(Value::as_sequence).filter(|s| !s.is_empty()) else {
        return WcsNodeClass::NotImaging2D("node has no steps".into());
    };
    if let Some(t) = steps[0].get("transform").filter(|t| !t.is_null()) {
        if is_fitswcs_imaging(t) {
            return WcsNodeClass::FitsEquivalent;
        }
    }
    let (Some(first), Some(last)) = (steps[0].get("frame"), steps[steps.len() - 1].get("frame")) else {
        return WcsNodeClass::NotImaging2D("steps lack frames".into());
    };
    let first = parse_frame(first);
    let last = parse_frame(last);
    if first.naxes != 2 {
        return WcsNodeClass::NotImaging2D(format!("first frame '{}' has {} axes", first.frame.name, first.naxes));
    }
    match &last.frame.kind {
        FrameKind::Celestial { .. } => {}
        FrameKind::Frame2D => {
            return WcsNodeClass::NotImaging2D(format!("world frame '{}' is a frame2d, not a celestial frame", last.frame.name));
        }
        FrameKind::Other(tag) => {
            return WcsNodeClass::NotImaging2D(format!("world frame '{}' is a {tag}, not a celestial frame", last.frame.name));
        }
    }
    if last.naxes != 2 {
        return WcsNodeClass::NotImaging2D(format!("world frame '{}' has {} axes", last.frame.name, last.naxes));
    }
    WcsNodeClass::Imaging2D
}

pub fn wcsinfo_sip_residuals(tree: &Value) -> (Option<f64>, Option<f64>) {
    let wcsinfo = ["meta.wcsinfo", "roman.meta.wcsinfo"].iter().find_map(|key| wcs_node_at(tree, key));
    let read = |key: &str| {
        wcsinfo
            .and_then(|w| w.get(key))
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    (read("sipmxerr"), read("sipiverr"))
}

pub fn parse_node(node: &Value, arrays: ArrayResolver, path: &str) -> Result<Node, GwcsError> {
    parse_node_impl(node, arrays, path, true)
}

fn parse_node_impl(node: &Value, arrays: ArrayResolver, path: &str, read_inverse: bool) -> Result<Node, GwcsError> {
    let family = tag_family(node).unwrap_or_else(|| "untagged".to_string());
    let kind = match family.as_str() {
        "compose" => {
            let (a, b) = compound_children(node, path)?;
            let a = parse_node_impl(a, arrays, &format!("{path}/forward[0]"), true)?;
            let b = parse_node_impl(b, arrays, &format!("{path}/forward[1]"), true)?;
            if a.n_outputs != b.n_inputs {
                return Err(GwcsError::Shape {
                    path: path.to_string(),
                    reason: format!("'{}' produces {} outputs but '{}' takes {} inputs", a.tag, a.n_outputs, b.tag, b.n_inputs),
                });
            }
            Kind::Compose(Box::new(a), Box::new(b))
        }
        "concatenate" => {
            let (a, b) = compound_children(node, path)?;
            let a = parse_node_impl(a, arrays, &format!("{path}/forward[0]"), true)?;
            let b = parse_node_impl(b, arrays, &format!("{path}/forward[1]"), true)?;
            Kind::Concatenate(Box::new(a), Box::new(b))
        }
        "shift" => Kind::Shift(scalar_param(node, "offset", path)?),
        "scale" => Kind::Scale(scalar_param(node, "factor", path)?),
        "polynomial" => polynomial_kind(node, arrays, path)?,
        "remap_axes" => mapping_kind(node, path)?,
        "identity" => Kind::Identity(node.get("n_dims").and_then(Value::as_u64).map_or(1, |n| n as usize)),
        "affine" => affine_kind(node, arrays, path)?,
        "rotate2d" => Kind::Rotation2D { angle_deg: scalar_param(node, "angle", path)? },
        "rotate_sequence_3d" => rotation_sequence_kind(node, path)?,
        "spherical_cartesian" => spherical_cartesian_kind(node, path)?,
        "gnomonic" => gnomonic_kind(node, path)?,
        "rotate3d" => rotate3d_kind(node, path)?,
        "constant" => constant_kind(node, path)?,
        other => match ArithOp::from_tag(other) {
            Some(op) => arithmetic_kind(node, arrays, path, op)?,
            None => return Err(GwcsError::UnsupportedTag { tag: other.to_string(), path: path.to_string() }),
        },
    };
    let mut out = Node::new(kind, &family, path);
    if out.n_inputs > MAX_LANES || out.n_outputs > MAX_LANES {
        return Err(GwcsError::Shape {
            path: path.to_string(),
            reason: format!("'{family}' uses {} -> {} lanes; at most {MAX_LANES} are supported", out.n_inputs, out.n_outputs),
        });
    }
    out.name = node.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if read_inverse {
        if let Some(inverse) = node.get("inverse").filter(|v| !v.is_null()) {
            match parse_node_impl(inverse, arrays, &format!("{path}/inverse"), false) {
                Ok(inv) => out.stored_inverse = Some(Box::new(inv)),
                Err(e) => out.stored_inverse_error = Some(format!("{e:#}")),
            }
        }
    }
    Ok(out)
}

pub fn parse_gwcs(wcs: &Value, arrays: ArrayResolver, source_key: &str) -> Result<GwcsPipeline, GwcsError> {
    let Some(step_nodes) = wcs.get("steps").and_then(Value::as_sequence) else {
        return Err(GwcsError::Parse(format!("node '{source_key}' has no 'steps' sequence")));
    };
    if step_nodes.len() < 2 {
        return Err(GwcsError::Parse(format!("node '{source_key}' has {} steps; at least 2 are required", step_nodes.len())));
    }
    let name = wcs.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let mut steps = Vec::with_capacity(step_nodes.len());
    let mut frames = Vec::with_capacity(step_nodes.len());
    for (k, step) in step_nodes.iter().enumerate() {
        let path = format!("steps[{k}]");
        let Some(frame_node) = step.get("frame") else {
            return Err(GwcsError::Missing { path, field: "frame".into() });
        };
        let parsed = parse_frame(frame_node);
        let transform = match step.get("transform") {
            Some(t) if !t.is_null() => Some(parse_node(t, arrays, &format!("{path}.transform"))?),
            _ => None,
        };
        steps.push(GwcsStep { frame: parsed.frame.clone(), transform });
        frames.push(parsed);
    }
    check_frames(&frames[0], &frames[frames.len() - 1])?;
    let bounding_box = match step_nodes[0].get("transform") {
        Some(t) if !t.is_null() => parse_bounding_box(t, "steps[0].transform")?,
        _ => None,
    };
    GwcsPipeline::new(steps, bounding_box, name, source_key.to_string())
}

struct ParsedFrame {
    frame: GwcsFrame,
    axes_order: Vec<usize>,
    naxes: usize,
}

fn strings_at(node: &Value, key: &str) -> Vec<String> {
    node.get(key)
        .and_then(Value::as_sequence)
        .map(|seq| seq.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

fn usizes_at(node: &Value, key: &str) -> Vec<usize> {
    node.get(key)
        .and_then(Value::as_sequence)
        .map(|seq| seq.iter().filter_map(Value::as_u64).map(|n| n as usize).collect())
        .unwrap_or_default()
}

fn frame_naxes(node: &Value) -> usize {
    if let Some(n) = node.get("naxes").and_then(Value::as_u64) {
        return n as usize;
    }
    if let Some(children) = node.get("frames").and_then(Value::as_sequence) {
        return children.iter().map(frame_naxes).sum();
    }
    let order = usizes_at(node, "axes_order").len();
    if order > 0 {
        return order;
    }
    let names = strings_at(node, "axes_names").len();
    if names > 0 {
        return names;
    }
    match tag_family(node).as_deref() {
        Some("frame2d") | Some("celestial_frame") => 2,
        Some("spectral_frame") | Some("temporal_frame") => 1,
        _ => 0,
    }
}

fn parse_frame(node: &Value) -> ParsedFrame {
    let kind = match tag_family(node).as_deref() {
        Some("frame2d") => FrameKind::Frame2D,
        Some("celestial_frame") => FrameKind::Celestial {
            reference: node.get("reference_frame").map(WcsInfo::model_name).unwrap_or_default(),
        },
        Some(other) => FrameKind::Other(other.to_string()),
        None => FrameKind::Other("untagged".into()),
    };
    let mut axis_physical_types = strings_at(node, "axis_physical_types");
    if axis_physical_types.is_empty() {
        if let Some(children) = node.get("frames").and_then(Value::as_sequence) {
            axis_physical_types = children.iter().flat_map(|c| strings_at(c, "axis_physical_types")).collect();
        }
    }
    ParsedFrame {
        frame: GwcsFrame {
            name: node.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            kind,
            unit: strings_at(node, "unit"),
            axes_names: strings_at(node, "axes_names"),
            axis_physical_types,
        },
        axes_order: usizes_at(node, "axes_order"),
        naxes: frame_naxes(node),
    }
}

fn check_frames(first: &ParsedFrame, last: &ParsedFrame) -> Result<(), GwcsError> {
    if first.naxes != 2 {
        return Err(GwcsError::Frame(format!(
            "first frame '{}' must have 2 pixel axes (found {})",
            first.frame.name, first.naxes
        )));
    }
    if last.naxes != 2 {
        return Err(GwcsError::Frame(format!(
            "world frame has {} axes; only two-axis celestial frames are supported",
            last.naxes
        )));
    }
    if last.axes_order == [1, 0] {
        return Err(GwcsError::Frame(
            "celestial axes are latitude-first (axes_order [1, 0]); only longitude-first frames are supported".into(),
        ));
    }
    let types: Vec<String> = last.frame.axis_physical_types.iter().map(|t| t.to_ascii_lowercase()).collect();
    if types != ["pos.eq.ra", "pos.eq.dec"] {
        return Err(GwcsError::Frame(format!(
            "world frame axis_physical_types [{}] are not pos.eq.ra/pos.eq.dec",
            last.frame.axis_physical_types.join(", ")
        )));
    }
    if last.frame.unit.iter().any(|u| !u.is_empty() && !u.eq_ignore_ascii_case("deg")) {
        return Err(GwcsError::Frame(format!("world frame unit [{}] is not deg", last.frame.unit.join(", "))));
    }
    match &last.frame.kind {
        FrameKind::Celestial { reference } => {
            if !ICRS_LIKE_REFERENCES.contains(&reference.as_str()) {
                return Err(GwcsError::Frame(format!("reference frame '{reference}' is not ICRS")));
            }
        }
        FrameKind::Frame2D => return Err(GwcsError::Frame("world frame is a frame2d, not a celestial frame".into())),
        FrameKind::Other(tag) => return Err(GwcsError::Frame(format!("world frame '{tag}' is not a celestial frame"))),
    }
    Ok(())
}

fn parse_bounding_box(transform: &Value, path: &str) -> Result<Option<BoundingBox>, GwcsError> {
    let Some(bb) = transform.get("bounding_box").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let inputs = {
        let names = strings_at(transform, "inputs");
        if names.is_empty() {
            vec!["x0".to_string(), "x1".to_string()]
        } else {
            names
        }
    };
    if inputs.len() != 2 {
        return Err(GwcsError::Bbox(format!(
            "transform at {path} has {} inputs; a 2-D bounding box needs 2",
            inputs.len()
        )));
    }
    let mut intervals = [[f64::NEG_INFINITY, f64::INFINITY]; 2];
    let mut ignore = [true; 2];
    match untag(bb) {
        Value::Mapping(_) => {
            let Some(map) = bb.get("intervals").and_then(Value::as_mapping) else {
                return Err(GwcsError::Bbox(format!("bounding box at {path} has no 'intervals'")));
            };
            for (k, v) in map {
                let name = k.as_str().unwrap_or("").trim().to_string();
                let Some(idx) = inputs.iter().position(|i| *i == name) else {
                    return Err(GwcsError::Bbox(format!("interval '{name}' names an input the transform does not have")));
                };
                let Some(interval) = pair_at(v) else {
                    return Err(GwcsError::Bbox(format!("interval '{name}' is not [lo, hi]")));
                };
                intervals[idx] = interval;
                ignore[idx] = false;
            }
            for name in strings_at(bb, "ignore") {
                let Some(idx) = inputs.iter().position(|i| *i == name) else {
                    return Err(GwcsError::Bbox(format!("ignore '{name}' names an input the transform does not have")));
                };
                ignore[idx] = true;
            }
        }
        Value::Sequence(seq) => {
            let rows: Vec<[f64; 2]> = seq.iter().filter_map(pair_at).collect();
            if rows.len() != 2 || rows.len() != seq.len() {
                return Err(GwcsError::Bbox(format!(
                    "legacy bounding box at {path} must be [[lo, hi], [lo, hi]]"
                )));
            }
            intervals = [rows[1], rows[0]];
            ignore = [false, false];
        }
        _ => return Err(GwcsError::Bbox(format!("bounding box at {path} has an unsupported form"))),
    }
    Ok(Some(BoundingBox { intervals, ignore }))
}

fn pair_at(v: &Value) -> Option<[f64; 2]> {
    match v.as_sequence()?.as_slice() {
        [a, b] => Some([a.as_f64()?, b.as_f64()?]),
        _ => None,
    }
}

fn pair_pair_at(v: &Value) -> Option<[[f64; 2]; 2]> {
    match v.as_sequence()?.as_slice() {
        [a, b] => Some([pair_at(a)?, pair_at(b)?]),
        _ => None,
    }
}

fn scalar_param(node: &Value, key: &str, path: &str) -> Result<f64, GwcsError> {
    let Some(v) = node.get(key) else {
        return Err(GwcsError::Missing { path: path.to_string(), field: key.to_string() });
    };
    if let Some(f) = v.as_f64() {
        return Ok(f);
    }
    let Some(value) = v.get("value").and_then(Value::as_f64) else {
        return Err(GwcsError::Parse(format!("parameter '{key}' at {path} is not a number")));
    };
    let unit: String = v.get("unit").and_then(Value::as_str).unwrap_or("").split_whitespace().collect();
    if PIXEL_UNITS.iter().any(|u| u.eq_ignore_ascii_case(&unit)) {
        Ok(value)
    } else {
        Err(GwcsError::Parse(format!(
            "quantity unit '{unit}' at {path} is not supported (dimensionless or pixel expected)"
        )))
    }
}

fn string_param<'a>(node: &'a Value, key: &str, default: &'a str) -> &'a str {
    node.get(key).and_then(Value::as_str).map(str::trim).unwrap_or(default)
}

fn resolve_array(node: &Value, key: &str, arrays: ArrayResolver, path: &str) -> Result<(Vec<usize>, Vec<f64>), GwcsError> {
    let Some(v) = node.get(key) else {
        return Err(GwcsError::Missing { path: path.to_string(), field: key.to_string() });
    };
    let (shape, values) = match arrays(v).or_else(|| inline_ndarray(v)) {
        Some(resolved) => resolved,
        None => {
            let mut values = Vec::new();
            let mut dims = Vec::new();
            if flatten_inline(v, &mut values, &mut dims, 0) && !dims.is_empty() {
                (dims, values)
            } else {
                return Err(GwcsError::Arrays {
                    path: path.to_string(),
                    reason: format!("'{key}' array could not be resolved to float values"),
                });
            }
        }
    };
    let expected: usize = shape.iter().product();
    if values.len() != expected {
        return Err(GwcsError::Arrays {
            path: path.to_string(),
            reason: format!("'{key}' has {} values but shape {shape:?} needs {expected}", values.len()),
        });
    }
    Ok((shape, values))
}

fn inline_ndarray(v: &Value) -> Option<(Vec<usize>, Vec<f64>)> {
    if !matches!(untag(v), Value::Mapping(_)) || v.get("data").is_none() {
        return None;
    }
    let meta = NdArrayMeta::from_yaml(v).ok()?;
    match meta.source {
        ArraySource::Inline(values) => Some((meta.shape, values)),
        ArraySource::Block(_) => None,
    }
}

fn shape_text(shape: &[usize]) -> String {
    let parts: Vec<String> = shape.iter().map(usize::to_string).collect();
    format!("[{}]", parts.join(", "))
}

fn polynomial_kind(node: &Value, arrays: ArrayResolver, path: &str) -> Result<Kind, GwcsError> {
    let (shape, coeffs) = resolve_array(node, "coefficients", arrays, path)?;
    let square_error = || GwcsError::Arrays {
        path: path.to_string(),
        reason: format!(
            "coefficient array has shape {}; a square 2-D array or a 1-D array is required",
            shape_text(&shape)
        ),
    };
    match shape.as_slice() {
        [n] if *n >= 1 => Ok(Kind::Poly1D {
            coeffs,
            domain: node.get("domain").and_then(pair_at),
            window: node.get("window").and_then(pair_at),
        }),
        [n, m] if n == m && *n >= 1 => Ok(Kind::Poly2D {
            degree: n - 1,
            coeffs,
            domain: node.get("domain").and_then(pair_pair_at),
            window: node.get("window").and_then(pair_pair_at),
        }),
        _ => Err(square_error()),
    }
}

fn affine_kind(node: &Value, arrays: ArrayResolver, path: &str) -> Result<Kind, GwcsError> {
    let (shape, m) = resolve_array(node, "matrix", arrays, path)?;
    if shape != [2, 2] {
        return Err(GwcsError::Arrays {
            path: path.to_string(),
            reason: format!("affine matrix has shape {}; [2, 2] is required", shape_text(&shape)),
        });
    }
    let translation = match node.get("translation") {
        Some(v) if !v.is_null() => {
            let (shape, t) = resolve_array(node, "translation", arrays, path)?;
            if shape != [2] {
                return Err(GwcsError::Arrays {
                    path: path.to_string(),
                    reason: format!("affine translation has shape {}; [2] is required", shape_text(&shape)),
                });
            }
            [t[0], t[1]]
        }
        _ => [0.0, 0.0],
    };
    Ok(Kind::Affine2D { matrix: [[m[0], m[1]], [m[2], m[3]]], translation })
}

fn rotation_sequence_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    let Some(angles) = node.get("angles").and_then(Value::as_sequence) else {
        return Err(GwcsError::Missing { path: path.to_string(), field: "angles".into() });
    };
    let angles: Vec<f64> = angles.iter().map(|a| a.as_f64().unwrap_or(f64::NAN)).collect();
    let order = string_param(node, "axes_order", "");
    let axes: Vec<Axis> = order.chars().filter_map(Axis::from_char).collect();
    if axes.len() != order.len() || axes.len() != angles.len() || axes.is_empty() {
        return Err(GwcsError::Shape {
            path: path.to_string(),
            reason: format!("axes_order '{order}' does not match {} angles", angles.len()),
        });
    }
    let spherical = match string_param(node, "rotation_type", "cartesian") {
        "cartesian" => false,
        "spherical" => true,
        other => return Err(GwcsError::Parse(format!("rotation_type '{other}' at {path} is not cartesian or spherical"))),
    };
    Ok(Kind::rotation_sequence(angles, axes, spherical))
}

fn spherical_cartesian_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    let wrap = node.get("wrap_lon_at").and_then(Value::as_f64).unwrap_or(360.0);
    let wrap_lon_at = if wrap == 180.0 {
        180
    } else if wrap == 360.0 {
        360
    } else {
        return Err(GwcsError::Parse(format!("wrap_lon_at {wrap} at {path} must be 180 or 360")));
    };
    match string_param(node, "transform_type", "") {
        "spherical_to_cartesian" => Ok(Kind::SphericalToCartesian { wrap_lon_at }),
        "cartesian_to_spherical" => Ok(Kind::CartesianToSpherical { wrap_lon_at }),
        other => Err(GwcsError::Parse(format!("transform_type '{other}' at {path} is not spherical_to_cartesian or cartesian_to_spherical"))),
    }
}

fn rotate3d_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    let lon = scalar_param(node, "phi", path)?;
    let lat = scalar_param(node, "theta", path)?;
    let lon_pole = scalar_param(node, "psi", path)?;
    match string_param(node, "direction", "native2celestial") {
        "native2celestial" => Ok(Kind::rotate_native2celestial(lon, lat, lon_pole)),
        "celestial2native" => Ok(Kind::rotate_celestial2native(lon, lat, lon_pole)),
        other => Err(GwcsError::Parse(format!("rotate3d direction '{other}' at {path} is not supported"))),
    }
}

fn gnomonic_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    match string_param(node, "direction", "pix2sky") {
        "pix2sky" => Ok(Kind::Pix2SkyTan),
        "sky2pix" => Ok(Kind::Sky2PixTan),
        other => Err(GwcsError::Parse(format!("gnomonic direction '{other}' at {path} is not supported"))),
    }
}

fn mapping_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    let Some(seq) = node.get("mapping").and_then(Value::as_sequence) else {
        return Err(GwcsError::Missing { path: path.to_string(), field: "mapping".into() });
    };
    let mapping: Vec<usize> = seq.iter().filter_map(Value::as_u64).map(|n| n as usize).collect();
    if mapping.len() != seq.len() {
        return Err(GwcsError::Parse(format!("mapping at {path} has non-integer entries")));
    }
    let n_inputs = node
        .get("n_inputs")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or_else(|| mapping.iter().max().map_or(0, |m| m + 1));
    if mapping.iter().any(|m| *m >= n_inputs) {
        return Err(GwcsError::Shape {
            path: path.to_string(),
            reason: format!("mapping {mapping:?} indexes beyond its {n_inputs} inputs"),
        });
    }
    Ok(Kind::Mapping { mapping, n_inputs })
}

fn arithmetic_kind(node: &Value, arrays: ArrayResolver, path: &str, op: ArithOp) -> Result<Kind, GwcsError> {
    let (a, b) = compound_children(node, path)?;
    let a = parse_node_impl(a, arrays, &format!("{path}/forward[0]"), true)?;
    let b = parse_node_impl(b, arrays, &format!("{path}/forward[1]"), true)?;
    if a.n_inputs != b.n_inputs || a.n_outputs != b.n_outputs {
        return Err(GwcsError::Shape {
            path: path.to_string(),
            reason: format!(
                "both operands of '{}' must match numbers of inputs and outputs ('{}' maps {} -> {}, '{}' maps {} -> {})",
                op.tag(),
                a.tag,
                a.n_inputs,
                a.n_outputs,
                b.tag,
                b.n_inputs,
                b.n_outputs
            ),
        });
    }
    Ok(Kind::Arithmetic(op, Box::new(a), Box::new(b)))
}

const CONSTANT_DIMENSIONS_SINCE: (u32, u32, u32) = (1, 4, 0);

fn tag_version(node: &Value) -> Option<(u32, u32, u32)> {
    let Value::Tagged(tagged) = node else { return None };
    let tag = tagged.tag.to_string();
    let (_, version) = tag.trim_end_matches('>').rsplit_once('-')?;
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

fn constant_kind(node: &Value, path: &str) -> Result<Kind, GwcsError> {
    let value = scalar_param(node, "value", path)?;
    if tag_version(node).is_some_and(|v| v < CONSTANT_DIMENSIONS_SINCE) {
        return Ok(Kind::Constant { value, n_inputs: 1 });
    }
    let Some(dimensions) = node.get("dimensions") else {
        return Err(GwcsError::Missing { path: path.to_string(), field: "dimensions".into() });
    };
    match dimensions.as_u64() {
        Some(n @ (1 | 2)) => Ok(Kind::Constant { value, n_inputs: n as usize }),
        other => Err(GwcsError::Parse(format!(
            "constant at {path} has dimensions {}; 1 or 2 is required",
            other.map_or_else(|| format!("{:?}", untag(dimensions)), |n| n.to_string())
        ))),
    }
}

fn compound_children<'a>(node: &'a Value, path: &str) -> Result<(&'a Value, &'a Value), GwcsError> {
    let Some(forward) = node.get("forward").and_then(Value::as_sequence) else {
        return Err(GwcsError::Missing { path: path.to_string(), field: "forward".into() });
    };
    match forward.as_slice() {
        [a, b] => Ok((a, b)),
        other => Err(GwcsError::Shape {
            path: path.to_string(),
            reason: format!("forward has {} members; 2 are required", other.len()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::transform::Vals;
    use super::super::NewtonOptions;
    use super::*;

    fn wcs_body(first_frame: &str, transform: &str, last_frame: &str) -> String {
        format!(
            "wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>\n  name: ''\n  steps:\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: {first_frame}\n    transform: {transform}\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: {last_frame}\n    transform: null\n"
        )
    }

    const DETECTOR: &str = "!<tag:stsci.edu:gwcs/frame2d-1.2.0> {axes_names: [x, y], axes_order: [0, 1], axis_physical_types: ['custom:x', 'custom:y'], name: detector, unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]}";
    const SHIFT_PAIR: &str = "!transform/concatenate-1.4.0 {forward: [!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}, !transform/shift-1.4.0 {inputs: [x], offset: 2.0, outputs: [y]}], inputs: [x0, x1], outputs: [y0, y1]}";

    fn world(reference: &str, axes_order: &str, types: &str, unit: &str) -> String {
        format!(
            "!<tag:stsci.edu:gwcs/celestial_frame-1.2.0>\n      axes_names: [lon, lat]\n      axes_order: {axes_order}\n      axis_physical_types: {types}\n      name: world\n      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/{reference}-1.1.0> {{frame_attributes: {{}}}}\n      unit: [!unit/unit-1.0.0 {unit}, !unit/unit-1.0.0 {unit}]"
        )
    }

    fn icrs_world() -> String {
        world("icrs", "[0, 1]", "[pos.eq.ra, pos.eq.dec]", "deg")
    }

    fn parse_wcs_body(body: &str) -> Result<GwcsPipeline, GwcsError> {
        let tree = parse_real_syntax(body);
        let (key, node) = find_wcs_node(&tree).expect("wcs node with steps");
        parse_gwcs(node, &no_arrays, &key.0)
    }

    fn err_text(r: Result<impl std::fmt::Debug, GwcsError>) -> String {
        match r {
            Err(e) => format!("{e:#}"),
            Ok(v) => panic!("expected an error, got {v:?}"),
        }
    }

    #[test]
    fn unsupported_tags_are_refused_by_name() {
        let text = err_text(try_fixture_node("tabular1d_with_inverse.asdf"));
        assert!(text.contains("'tabular'"), "{text}");
        let text = err_text(try_fixture_node("tabular1d_with_inverse_std150.asdf"));
        assert!(text.contains("'tabular'"), "{text}");
        let tree = parse_real_syntax(
            "transform: !transform/fix_inputs-1.2.0\n  forward:\n  - !transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}\n  - !core/constant-1.0.0 {fixed_inputs: {x: 1}}\n  inputs: []\n  outputs: [y]\n",
        );
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert!(text.contains("'fix_inputs'"), "{text}");
        let tree = parse_real_syntax("transform: !<tag:stsci.edu:jwst_pipeline/coords-1.1.0> {model_type: unit}\n");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert!(text.contains("'coords'"), "{text}");
        let tree = parse_real_syntax("transform: !<tag:stsci.edu:jwst_pipeline/grating_equation-1.1.0> {groove_density: 1.0, order: 1, output_type: wavelength}\n");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert!(text.contains("'grating_equation'"), "{text}");
        let nested = format!(
            "!transform/compose-1.4.0\n      forward:\n      - {SHIFT_PAIR}\n      - !transform/concatenate-1.4.0\n        forward:\n        - !transform/tabular-1.2.0 {{lookup_table: [1.0, 2.0], points: [[0.0, 1.0]], inputs: [x], outputs: [y]}}\n        - !transform/shift-1.4.0 {{inputs: [x], offset: 2.0, outputs: [y]}}\n        inputs: [x0, x1]\n        outputs: [y0, y1]\n      inputs: [x0, x1]\n      outputs: [y0, y1]"
        );
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, &nested, &icrs_world())));
        assert_eq!(text, "gWCS transform 'tabular' at steps[0].transform/forward[1]/forward[0] is not supported by the evaluator");
        let tree = parse_real_syntax("transform: {offset: 1.0, inputs: [x], outputs: [y]}\n");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert!(text.contains("'untagged'"), "{text}");
    }

    fn transform_node(yaml: &str) -> Result<Node, GwcsError> {
        let tree = parse_real_syntax(&format!("transform: {yaml}\n"));
        parse_node(&tree["transform"], &no_arrays, "transform")
    }

    #[test]
    fn arithmetic_operator_tags_parse_with_matching_operands() {
        let shift = "!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}";
        let scale = "!transform/scale-1.4.0 {factor: 2.0, inputs: [x], outputs: [y]}";
        for (tag, op, exp) in [
            ("add", ArithOp::Add, 10.0),
            ("subtract", ArithOp::Subtract, -2.0),
            ("multiply", ArithOp::Multiply, 24.0),
            ("divide", ArithOp::Divide, 4.0 / 6.0),
            ("power", ArithOp::Power, 4096.0),
        ] {
            for version in ["1.2.0", "1.4.0"] {
                let node = transform_node(&format!("!transform/{tag}-{version} {{forward: [{shift}, {scale}], inputs: [x], outputs: [y]}}"))
                    .unwrap_or_else(|e| panic!("{tag}-{version}: {e:#}"));
                assert_eq!(node.tag, tag);
                match &node.kind {
                    Kind::Arithmetic(got, a, b) => {
                        assert_eq!(*got, op);
                        assert_eq!((a.path.as_str(), b.path.as_str()), ("transform/forward[0]", "transform/forward[1]"));
                    }
                    other => panic!("{tag}: {other:?}"),
                }
                assert_close(node.eval(Vals::one(3.0)).v[0], exp, 1e-12, &format!("{tag}-{version} of Shift(1) and Scale(2) at 3"));
            }
        }
        let pair = "!transform/concatenate-1.4.0 {forward: [!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}, !transform/shift-1.4.0 {inputs: [x], offset: 2.0, outputs: [y]}], inputs: [x0, x1], outputs: [y0, y1]}";
        let text = err_text(transform_node(&format!("!transform/divide-1.4.0 {{forward: [{shift}, {pair}], inputs: [x], outputs: [y]}}")));
        assert_eq!(
            text,
            "gWCS transform at transform: both operands of 'divide' must match numbers of inputs and outputs ('shift' maps 1 -> 1, 'concatenate' maps 2 -> 2)"
        );
        let text = err_text(transform_node(&format!("!transform/add-1.4.0 {{forward: [{shift}, !transform/tabular-1.2.0 {{lookup_table: [1.0, 2.0], points: [[0.0, 1.0]], inputs: [x], outputs: [y]}}], inputs: [x], outputs: [y]}}")));
        assert_eq!(text, "gWCS transform 'tabular' at transform/forward[1] is not supported by the evaluator");
    }

    #[test]
    fn constant_tags_follow_the_asdf_astropy_dimension_rule() {
        let node = transform_node("!transform/constant-1.6.0 {dimensions: 1, inputs: [x], outputs: [y], value: 299.7}").unwrap();
        assert!(matches!(node.kind, Kind::Constant { value, n_inputs: 1 } if value == 299.7), "{:?}", node.kind);
        assert_eq!(node.tag, "constant");
        let node = transform_node("!transform/constant-1.6.0 {dimensions: 2, inputs: [x, y], outputs: [z], value: -0.5}").unwrap();
        assert_eq!((node.n_inputs, node.n_outputs), (2, 1));
        assert_eq!(node.eval(Vals::pair(3.0, 4.0)).as_slice(), &[-0.5]);
        let node = transform_node("!transform/constant-1.2.0 {inputs: [x], outputs: [y], value: 4.0}").unwrap();
        assert!(matches!(node.kind, Kind::Constant { value, n_inputs: 1 } if value == 4.0), "{:?}", node.kind);
        let node = transform_node("!transform/constant-1.3.0 {dimensions: 2, inputs: [x], outputs: [y], value: 4.0}").unwrap();
        assert_eq!(node.n_inputs, 1, "asdf-astropy reads every constant before 1.4.0 as Const1D");
        let text = err_text(transform_node("!transform/constant-1.6.0 {dimensions: 3, inputs: [x], outputs: [y], value: 4.0}"));
        assert_eq!(text, "gWCS: constant at transform has dimensions 3; 1 or 2 is required");
        let text = err_text(transform_node("!transform/constant-1.6.0 {inputs: [x], outputs: [y], value: 4.0}"));
        assert_eq!(text, "gWCS node at transform lacks 'dimensions'");
        let text = err_text(transform_node("!transform/constant-1.6.0 {dimensions: 1, inputs: [x], outputs: [y], value: !unit/quantity-1.1.0 {value: 299.7, unit: !unit/unit-1.0.0 um}}"));
        assert_eq!(text, "gWCS: quantity unit 'um' at transform is not supported (dimensionless or pixel expected)");
        let text = err_text(transform_node("!transform/constant-1.6.0 {dimensions: 1, inputs: [x], outputs: [y]}"));
        assert_eq!(text, "gWCS node at transform lacks 'value'");
        for name in ["const1d_with_inverse.asdf", "const1d_with_inverse_std150.asdf"] {
            let node = fixture_node(name);
            assert!(matches!(node.kind, Kind::Constant { value, n_inputs: 1 } if value == 299.7), "{name}: {:?}", node.kind);
            let inverse = node.stored_inverse.as_ref().unwrap_or_else(|| panic!("{name}: stored inverse"));
            assert!(matches!(inverse.kind, Kind::Constant { value, n_inputs: 1 } if value == 299.7), "{name}: {:?}", inverse.kind);
        }
    }

    #[test]
    fn stored_inverse_with_an_unsupported_tag_does_not_refuse_the_forward() {
        let poly = "!transform/polynomial-1.3.0 {coefficients: !core/ndarray-1.1.0 {data: [10.0, 0.001], datatype: float64, shape: [2]}, inputs: [x], outputs: [y], inverse: !transform/tabular-1.2.0 {lookup_table: [1.0, 2.0], points: [[0.0, 1.0]], inputs: [x], outputs: [y]}}";
        let tree = parse_real_syntax(&format!("transform: {poly}\n"));
        let node = parse_node(&tree["transform"], &no_arrays, "transform").unwrap();
        assert!(node.stored_inverse.is_none());
        let err = node.stored_inverse_error.clone().expect("stored inverse error recorded");
        assert!(err.contains("'tabular'"), "{err}");
        assert!(err.contains("transform/inverse"), "{err}");
        assert_close(node.eval(Vals::one(3.0)).v[0], 10.003, 1e-12, "forward evaluates");
        let text = err_text(node.inverse());
        assert!(text.contains("has no inverse (none stored, none derivable)"), "{text}");
        assert!(text.contains("(stored inverse: "), "{text}");
        let poly_b = poly.replace("10.0, 0.001", "20.0, 0.001");
        let transform = format!(
            "!transform/concatenate-1.4.0\n      bounding_box: !transform/property/bounding_box-1.2.0 {{ignore: [], intervals: {{x0: [-0.5, 99.5], x1: [-0.5, 99.5]}}, order: F}}\n      forward: [{poly}, {poly_b}]\n      inputs: [x0, x1]\n      outputs: [y0, y1]"
        );
        let pipeline = parse_wcs_body(&wcs_body(DETECTOR, &transform, &icrs_world())).unwrap();
        assert!(pipeline.backward_analytic(10.03, 20.04).is_err());
        assert_eq!(pipeline.bbox_centre(), Some([49.5, 49.5]));
        let w = pipeline.forward(30.0, 40.0, true);
        assert_close_slice(&w, &[10.03, 20.04], 1e-12, "forward");
        let r = pipeline.backward_exact(w[0], w[1], &NewtonOptions::default());
        assert!(r.converged, "{r:?}");
        assert_close_slice(&r.pixel, &[30.0, 40.0], 1e-6, "Newton from the bbox centre");
    }

    #[test]
    fn arrays_need_a_shape() {
        let block = "!transform/polynomial-1.3.0 {coefficients: !core/ndarray-1.1.0 {source: 0, datatype: float64, byteorder: little, shape: [4]}, inputs: [x], outputs: [y]}";
        let tree = parse_real_syntax(&format!("transform: {block}\n"));
        let values = [1.0, 2.0, 3.0, 4.0];
        let one_d = parse_node(&tree["transform"], &|_| Some((vec![4], values.to_vec())), "steps[0].transform/forward[1]").unwrap();
        match &one_d.kind {
            Kind::Poly1D { coeffs, .. } => assert_eq!(coeffs.len(), 4),
            other => panic!("{other:?}"),
        }
        assert_close(one_d.eval(Vals::one(2.0)).v[0], 49.0, 1e-12, "cubic");
        let two_d = parse_node(&tree["transform"], &|_| Some((vec![2, 2], values.to_vec())), "steps[0].transform/forward[1]").unwrap();
        match &two_d.kind {
            Kind::Poly2D { degree, .. } => assert_eq!(*degree, 1),
            other => panic!("{other:?}"),
        }
        assert_eq!((two_d.n_inputs, two_d.n_outputs), (2, 1));
        assert_close(two_d.eval(Vals::pair(2.0, 5.0)).v[0], 17.0, 1e-12, "c00 + c10 x + c01 y with [[c00, c01], [c10, c11]]");
        let text = err_text(parse_node(&tree["transform"], &|_| Some((vec![3, 4], vec![0.0; 12])), "steps[0].transform/forward[1]"));
        assert_eq!(text, "gWCS array at steps[0].transform/forward[1]: coefficient array has shape [3, 4]; a square 2-D array or a 1-D array is required");
        let plain = parse_real_syntax("transform: !transform/polynomial-1.3.0 {coefficients: [[1, 2], [3, 4]], inputs: [x, y], outputs: [z]}\n");
        let node = parse_node(&plain["transform"], &no_arrays, "transform").unwrap();
        match &node.kind {
            Kind::Poly2D { degree, coeffs, .. } => {
                assert_eq!(*degree, 1);
                assert_eq!(coeffs, &[1.0, 2.0, 3.0, 4.0]);
            }
            other => panic!("{other:?}"),
        }
        let text = err_text(parse_node(&tree["transform"], &|_| Some((vec![4], vec![1.0, 2.0])), "transform"));
        assert!(text.contains("has 2 values but shape [4] needs 4"), "{text}");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert!(text.contains("could not be resolved"), "{text}");
    }

    #[test]
    fn aliases_are_evaluated_as_copies() {
        let transform = "!transform/concatenate-1.4.0\n      forward:\n      - &id001 !transform/shift-1.4.0\n        inputs: [x]\n        offset: 1.0\n        outputs: [y]\n        inverse: &id002 !transform/polynomial-1.3.0\n          coefficients: !core/ndarray-1.1.0 {data: [-1.0, 1.0], datatype: float64, shape: [2]}\n          inputs: [x]\n          outputs: [y]\n      - *id001\n      inputs: [x0, x1]\n      outputs: [y0, y1]";
        let body = wcs_body(DETECTOR, transform, &icrs_world());
        let tree = parse_real_syntax(&body);
        let raw = &tree["wcs"]["steps"][0]["transform"]["forward"][1];
        assert_eq!(tag_family(raw).as_deref(), Some("shift"), "the alias copy carries the tag");
        assert_eq!(tag_family(&raw["inverse"]).as_deref(), Some("polynomial"));
        let pipeline = parse_wcs_body(&body).unwrap();
        let forward = pipeline.forward_node();
        let Kind::Concatenate(a, b) = &forward.kind else { panic!("{:?}", forward.kind) };
        assert_eq!((a.tag.as_str(), b.tag.as_str()), ("shift", "shift"));
        let inv = b.stored_inverse.as_ref().expect("alias copy keeps the stored inverse");
        assert_eq!(inv.tag, "polynomial");
        assert_close_slice(&pipeline.forward(1.0, 2.0, false), &[2.0, 3.0], 0.0, "both shifts evaluate");
        assert_close_slice(&pipeline.backward_analytic(2.0, 3.0).unwrap(), &[1.0, 2.0], 0.0, "stored polynomial inverses through the alias");
    }

    #[test]
    fn refusal_texts_name_frames() {
        let composite = "!<tag:stsci.edu:gwcs/composite_frame-1.1.0>\n      frames:\n      - !<tag:stsci.edu:gwcs/celestial_frame-1.2.0> {axes_names: [lon, lat], axes_order: [0, 1], axis_physical_types: [pos.eq.ra, pos.eq.dec], name: sky, reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0> {frame_attributes: {}}, unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]}\n      - !<tag:stsci.edu:gwcs/spectral_frame-1.1.0> {axes_names: [wavelength], axes_order: [2], axis_physical_types: [em.wl], name: spectral, unit: [!unit/unit-1.0.0 um]}\n      name: world";
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, composite)));
        assert_eq!(text, "gWCS frame: world frame has 3 axes; only two-axis celestial frames are supported");
        let custom = world("icrs", "[0, 1]", "['custom:x', 'custom:y']", "deg");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &custom)));
        assert_eq!(text, "gWCS frame: world frame axis_physical_types [custom:x, custom:y] are not pos.eq.ra/pos.eq.dec");
        let lat_first = world("icrs", "[1, 0]", "[pos.eq.ra, pos.eq.dec]", "deg");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &lat_first)));
        assert_eq!(text, "gWCS frame: celestial axes are latitude-first (axes_order [1, 0]); only longitude-first frames are supported");
        let galactic = world("galactic", "[0, 1]", "[pos.eq.ra, pos.eq.dec]", "deg");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &galactic)));
        assert_eq!(text, "gWCS frame: reference frame 'galactic' is not ICRS");
        let arcsec = world("icrs", "[0, 1]", "[pos.eq.ra, pos.eq.dec]", "arcsec");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &arcsec)));
        assert_eq!(text, "gWCS frame: world frame unit [arcsec, arcsec] is not deg");
        let cube = "!<tag:stsci.edu:gwcs/frame-1.2.0> {axes_names: [x, y, z], axes_order: [0, 1, 2], axis_physical_types: ['custom:x', 'custom:y', 'custom:z'], naxes: 3, name: detector, unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]}";
        let text = err_text(parse_wcs_body(&wcs_body(cube, SHIFT_PAIR, &icrs_world())));
        assert_eq!(text, "gWCS frame: first frame 'detector' must have 2 pixel axes (found 3)");
        let ok = parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &icrs_world())).unwrap();
        assert_eq!(ok.frames_summary(), "detector->world");
        assert_eq!(ok.steps[1].frame.kind, FrameKind::Celestial { reference: "icrs".into() });
        let fk5 = world("fk5", "[0, 1]", "[pos.eq.ra, pos.eq.dec]", "deg");
        assert!(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &fk5)).is_ok());
        let untagged_galactic = "!<tag:stsci.edu:gwcs/celestial_frame-1.2.0>\n      axes_names: [lon, lat]\n      axes_order: [0, 1]\n      axis_physical_types: [pos.eq.ra, pos.eq.dec]\n      name: world\n      reference_frame: {type: galactic}\n      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]";
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, untagged_galactic)));
        assert_eq!(text, "gWCS frame: reference frame 'galactic' is not ICRS");
        let untagged_fk4 = untagged_galactic.replace("type: galactic", "type: FK4");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &untagged_fk4)));
        assert_eq!(text, "gWCS frame: reference frame 'fk4' is not ICRS");
        let untagged_icrs = untagged_galactic.replace("type: galactic", "type: ICRS");
        let ok = parse_wcs_body(&wcs_body(DETECTOR, SHIFT_PAIR, &untagged_icrs)).unwrap();
        assert_eq!(ok.steps[1].frame.kind, FrameKind::Celestial { reference: "icrs".into() });
    }

    #[track_caller]
    fn assert_matches_case(got: &[f64], exp: &serde_json::Value, abs_tol: f64, what: &str) {
        let exp = exp.as_array().unwrap();
        assert_eq!(got.len(), exp.len(), "{what}: lanes");
        for (i, (g, e)) in got.iter().zip(exp).enumerate() {
            match e.as_f64() {
                None => assert!(g.is_nan(), "{what}[{i}]: got {g}, expected NaN"),
                Some(e) => assert!(
                    (g - e).abs() <= f64::max(abs_tol, 1e-10 * e.abs()),
                    "{what}[{i}]: got {g:?}, expected {e:?} (|diff| {:e})",
                    (g - e).abs()
                ),
            }
        }
    }

    #[test]
    fn every_committed_fixture_parses_or_refuses_as_expected_json_says() {
        let expected = expected().as_object().unwrap();
        let mut transforms = 0;
        let mut wcs_files = 0;
        let mut refused = 0;
        for (name, entry) in expected {
            if name == "_meta" {
                continue;
            }
            assert!(fixtures_dir().join(name).is_file(), "{name} listed in expected.json is missing on disk");
            let cases = entry["cases"].as_array().unwrap();
            match entry["key"].as_str().unwrap() {
                "transform" => {
                    let model = entry["model"].as_str().unwrap();
                    if model == "Tabular1D" {
                        let text = err_text(try_fixture_node(name));
                        assert!(text.contains("'tabular'"), "{name}: {text}");
                        refused += 1;
                        continue;
                    }
                    let node = fixture_node(name);
                    assert_eq!(node.n_inputs as u64, entry["n_inputs"].as_u64().unwrap(), "{name} n_inputs");
                    assert_eq!(node.n_outputs as u64, entry["n_outputs"].as_u64().unwrap(), "{name} n_outputs");
                    if entry["inverse"].as_str() == Some("none") {
                        assert!(matches!(node.inverse(), Err(GwcsError::NoInverse { .. })), "{name}: astropy has no inverse, got {:?}", node.inverse());
                    }
                    for case in cases {
                        let out = node.eval(Vals::from_slice(&json_f64s(&case["input"])));
                        assert_matches_case(out.as_slice(), &case["output"], 1e-12, &format!("{name} {}", case["input"]));
                        if let Some(inv_in) = case.get("inverse_input") {
                            let inv = node.inverse().unwrap_or_else(|e| panic!("{name}: {e:#}"));
                            let got = inv.eval(Vals::from_slice(&json_f64s(inv_in)));
                            assert_matches_case(got.as_slice(), &case["inverse_output"], 1e-9, &format!("{name} inverse {inv_in}"));
                        }
                    }
                    transforms += 1;
                }
                "wcs" => {
                    let pipeline = fixture_pipeline(name);
                    let frames: Vec<&str> = entry["frames"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
                    assert_eq!(pipeline.frame_names(), frames, "{name} frames");
                    match entry["bounding_box"].as_array() {
                        Some(bb) => {
                            let got = pipeline.bounding_box.expect("bbox");
                            assert_eq!(got.intervals, [[bb[0][0].as_f64().unwrap(), bb[0][1].as_f64().unwrap()], [bb[1][0].as_f64().unwrap(), bb[1][1].as_f64().unwrap()]], "{name} bbox");
                        }
                        None => assert_eq!(pipeline.bounding_box, None, "{name} bbox"),
                    }
                    for case in cases {
                        let input = json_f64s(&case["input"]);
                        let what = format!("{name} {input:?}");
                        assert_matches_case(&pipeline.forward(input[0], input[1], true), &case["output"], 1e-12, &format!("{what} output"));
                        let own_world = pipeline.forward(input[0], input[1], false);
                        assert_matches_case(&own_world, &case["output_no_bbox"], 1e-12, &format!("{what} output_no_bbox"));
                        for (frame, vals) in pipeline.forward_frames(input[0], input[1]) {
                            assert_matches_case(vals.as_slice(), &case["frames"][&frame], 1e-12, &format!("{what} frame {frame}"));
                        }
                        let world = json_f64s(&case["output_no_bbox"]);
                        let analytic = pipeline.backward_analytic(world[0], world[1]).unwrap();
                        assert_nan_pattern(analytic, json_opt_pair(&case["invert_no_bbox"]), 1e-7, &format!("{what} invert_no_bbox"));
                        let gwcs = pipeline.invert_gwcs_semantics(own_world[0], own_world[1]).unwrap();
                        assert_nan_pattern(gwcs, json_opt_pair(&case["invert"]), 1e-7, &format!("{what} invert"));
                        if let Some(exact) = case.get("exact_inverse_newton") {
                            let r = pipeline.backward_exact(world[0], world[1], &NewtonOptions::default());
                            assert!(r.converged && r.iterations <= 4, "{what}: {r:?}");
                            assert_nan_pattern(r.pixel, json_opt_pair(exact), 1e-6, &format!("{what} exact_inverse_newton"));
                        }
                    }
                    wcs_files += 1;
                }
                other => panic!("{name}: unknown key {other}"),
            }
        }
        assert_eq!((transforms, wcs_files, refused), (34, 8, 2));
    }

    #[test]
    fn find_wcs_node_follows_the_search_order_and_classifies() {
        let steps = "{steps: [{frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {axes_names: [x, y], axes_order: [0, 1], name: detector}, transform: null}]}";
        let tree = parse_real_syntax(&format!("roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_wcs-2.0.0>\n  wcs_l2: !<tag:stsci.edu:gwcs/wcs-1.4.0> {steps}\n  wcs_l1: !<tag:stsci.edu:gwcs/wcs-1.4.0> {steps}\n"));
        let (key, node) = find_wcs_node(&tree).unwrap();
        assert_eq!(key, WcsNodeKey("roman.wcs_l2".into()));
        assert!(node.get("steps").is_some());
        assert!(wcs_node_at(&tree, "roman.wcs_l1").is_some());
        assert!(wcs_node_at(&tree, "roman.meta.wcs").is_none());
        let tree = parse_real_syntax(&format!("meta:\n  wcs: {steps}\nwcs: {{name: shadowed}}\n"));
        assert_eq!(find_wcs_node(&tree).unwrap().0, WcsNodeKey("meta.wcs".into()));
        let tree = parse_real_syntax("meta:\n  wcs: {name: no-steps}\nwcs: {name: none}\n");
        assert!(find_wcs_node(&tree).is_none());
        assert_eq!(
            format!("{:#}", missing_wcs_node()),
            "gWCS: tree has no node with 'steps' under meta.wcs, roman.meta.wcs, wcs, gwcs, roman.wcs_l2 or roman.wcs_l1"
        );

        let imaging = parse_real_syntax(&wcs_body(DETECTOR, SHIFT_PAIR, &icrs_world()));
        assert_eq!(classify_wcs_node(&imaging["wcs"]), WcsNodeClass::Imaging2D);
        let fits_like = "!<tag:stsci.edu:gwcs/fitswcs_imaging-1.0.0> {crpix: [10.0, 20.0], crval: [1.0, 2.0], cdelt: [1.0e-5, 1.0e-5], pc: [[1, 0], [0, 1]], projection: !transform/gnomonic-1.3.0 {direction: pix2sky}}";
        let i2d = parse_real_syntax(&wcs_body(DETECTOR, fits_like, &icrs_world()));
        assert_eq!(classify_wcs_node(&i2d["wcs"]), WcsNodeClass::FitsEquivalent);
        assert!(is_fitswcs_imaging(&i2d["wcs"]["steps"][0]["transform"]));
        let structural = parse_real_syntax(&wcs_body(DETECTOR, "{crpix: [10.0, 20.0], crval: [1.0, 2.0], projection: !transform/gnomonic-1.3.0 {direction: pix2sky}}", &icrs_world()));
        assert_eq!(classify_wcs_node(&structural["wcs"]), WcsNodeClass::FitsEquivalent);
        let cube = "!<tag:stsci.edu:gwcs/frame-1.2.0> {axes_names: [x, y, z], axes_order: [0, 1, 2], naxes: 3, name: detector}";
        let s3d = parse_real_syntax(&wcs_body(cube, SHIFT_PAIR, &icrs_world()));
        assert!(matches!(classify_wcs_node(&s3d["wcs"]), WcsNodeClass::NotImaging2D(ref r) if r.contains("3")), "{:?}", classify_wcs_node(&s3d["wcs"]));
        let composite = "!<tag:stsci.edu:gwcs/composite_frame-1.1.0> {frames: [!<tag:stsci.edu:gwcs/celestial_frame-1.2.0> {axes_order: [0, 1], name: sky}, !<tag:stsci.edu:gwcs/spectral_frame-1.1.0> {axes_order: [2], name: spec}], name: world}";
        let s2d = parse_real_syntax(&wcs_body(DETECTOR, SHIFT_PAIR, composite));
        assert!(matches!(classify_wcs_node(&s2d["wcs"]), WcsNodeClass::NotImaging2D(_)));
        let galactic = parse_real_syntax(&wcs_body(DETECTOR, SHIFT_PAIR, &world("galactic", "[0, 1]", "[pos.eq.ra, pos.eq.dec]", "deg")));
        assert_eq!(classify_wcs_node(&galactic["wcs"]), WcsNodeClass::Imaging2D, "a galactic 2-axis frame is Imaging2D and refused later by parse_gwcs");
        let frame_naxes2 = "!<tag:stsci.edu:gwcs/frame-1.2.0> {axes_names: [x, y], axes_order: [0, 1], naxes: 2, name: detector}";
        let generic = parse_real_syntax(&wcs_body(frame_naxes2, SHIFT_PAIR, &icrs_world()));
        assert_eq!(classify_wcs_node(&generic["wcs"]), WcsNodeClass::Imaging2D);
        assert!(parse_wcs_body(&wcs_body(frame_naxes2, SHIFT_PAIR, &icrs_world())).is_ok());
        let no_steps = parse_real_syntax("wcs: {name: x}\n");
        assert!(matches!(classify_wcs_node(&no_steps["wcs"]), WcsNodeClass::NotImaging2D(_)));
    }

    #[test]
    fn wcsinfo_sip_residuals_read_meta() {
        let tree = parse_real_syntax("meta:\n  wcsinfo: {sipmxerr: 0.0087, sipiverr: 0.0088, a_order: 4}\n");
        assert_eq!(wcsinfo_sip_residuals(&tree), (Some(0.0087), Some(0.0088)));
        let tree = parse_real_syntax("roman:\n  meta:\n    wcsinfo: {sipmxerr: 3.5e-7}\n");
        assert_eq!(wcsinfo_sip_residuals(&tree), (Some(3.5e-7), None));
        let tree = parse_real_syntax("meta:\n  wcsinfo: {sipmxerr: -1.0, sipiverr: .nan}\n");
        assert_eq!(wcsinfo_sip_residuals(&tree), (None, None));
        let tree = parse_real_syntax("wcs: {name: x}\n");
        assert_eq!(wcsinfo_sip_residuals(&tree), (None, None));
    }

    #[test]
    fn quantity_units_restricted_to_pixels() {
        let tree = parse_real_syntax("transform: !transform/shift-1.4.0 {offset: !unit/quantity-1.1.0 {value: 2.5, unit: !unit/unit-1.0.0 pixel}, inputs: [x], outputs: [y]}\n");
        let node = parse_node(&tree["transform"], &no_arrays, "transform").unwrap();
        assert_close(node.eval(Vals::one(1.0)).v[0], 3.5, 0.0, "pixel quantity");
        let tree = parse_real_syntax("transform: !transform/scale-1.4.0 {factor: !unit/quantity-1.1.0 {value: 2.0, unit: !unit/unit-1.0.0 arcsec}, inputs: [x], outputs: [y]}\n");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "steps[0].transform/forward[0]"));
        assert_eq!(text, "gWCS: quantity unit 'arcsec' at steps[0].transform/forward[0] is not supported (dimensionless or pixel expected)");
        let tree = parse_real_syntax("transform: !transform/shift-1.4.0 {inputs: [x], outputs: [y]}\n");
        let text = err_text(parse_node(&tree["transform"], &no_arrays, "transform"));
        assert_eq!(text, "gWCS node at transform lacks 'offset'");
    }

    #[test]
    fn bounding_box_names_must_match_the_inputs() {
        let bad = format!("!transform/concatenate-1.4.0 {{bounding_box: !transform/property/bounding_box-1.2.0 {{ignore: [], intervals: {{x2: [-0.5, 10.5], x1: [-0.5, 20.5]}}, order: F}}, forward: [!transform/shift-1.4.0 {{inputs: [x], offset: 1.0, outputs: [y]}}, !transform/shift-1.4.0 {{inputs: [x], offset: 2.0, outputs: [y]}}], inputs: [x0, x1], outputs: [y0, y1]}}");
        let text = err_text(parse_wcs_body(&wcs_body(DETECTOR, &bad, &icrs_world())));
        assert_eq!(text, "gWCS bounding box: interval 'x2' names an input the transform does not have");
        let legacy = "!transform/concatenate-1.4.0 {bounding_box: [[-0.5, 10.5], [-0.5, 20.5]], forward: [!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}, !transform/shift-1.4.0 {inputs: [x], offset: 2.0, outputs: [y]}], inputs: [x0, x1], outputs: [y0, y1]}";
        let pipeline = parse_wcs_body(&wcs_body(DETECTOR, legacy, &icrs_world())).unwrap();
        assert_eq!(pipeline.bounding_box, Some(BoundingBox { intervals: [[-0.5, 20.5], [-0.5, 10.5]], ignore: [false, false] }));
        let ignored = "!transform/concatenate-1.4.0 {bounding_box: !transform/property/bounding_box-1.2.0 {ignore: [x1], intervals: {x0: [-0.5, 10.5]}, order: F}, forward: [!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}, !transform/shift-1.4.0 {inputs: [x], offset: 2.0, outputs: [y]}], inputs: [x0, x1], outputs: [y0, y1]}";
        let pipeline = parse_wcs_body(&wcs_body(DETECTOR, ignored, &icrs_world())).unwrap();
        let bb = pipeline.bounding_box.unwrap();
        assert_eq!((bb.intervals[0], bb.ignore), ([-0.5, 10.5], [false, true]));
        assert!(pipeline.in_bbox(3.0, 1e6));
        assert!(!pipeline.in_bbox(11.0, 0.0));
    }

    #[test]
    fn nested_bounding_boxes_are_ignored_and_names_are_kept() {
        let pipeline = fixture_pipeline("wcs_miri_gwcs_fixture.asdf");
        let bb = pipeline.bounding_box.unwrap();
        assert_eq!(bb.intervals, [[3.5, 1027.5], [-0.5, 1023.5]]);
        let inner_shift = "!transform/shift-1.4.0 {inputs: [x], offset: 1.0, outputs: [y]}";
        let nested = format!(
            "!transform/compose-1.4.0\n      bounding_box: !transform/property/bounding_box-1.2.0 {{ignore: [], intervals: {{x0: [-0.5, 99.5], x1: [-0.5, 99.5]}}, order: F}}\n      forward:\n      - {SHIFT_PAIR}\n      - !transform/concatenate-1.4.0\n        bounding_box: !transform/property/bounding_box-1.2.0 {{ignore: [], intervals: {{x0: [-0.5, 9.5], x1: [-0.5, 9.5]}}, order: F}}\n        forward: [{inner_shift}, {inner_shift}]\n        inputs: [x0, x1]\n        outputs: [y0, y1]\n      inputs: [x0, x1]\n      outputs: [y0, y1]"
        );
        let pipeline = parse_wcs_body(&wcs_body(DETECTOR, &nested, &icrs_world())).unwrap();
        assert_eq!(pipeline.bounding_box, Some(BoundingBox::square(-0.5, 99.5)), "only the outermost box of steps[0] is read");
        assert_close_slice(&pipeline.forward(50.0, 60.0, true), &[52.0, 63.0], 0.0, "inside the outer box, outside the nested one: finite");
        assert!(pipeline.forward(100.0, 60.0, true)[0].is_nan(), "outside the outer box: NaN");
        let poly = fixture_node("poly1d_with_inverse.asdf");
        assert_eq!(poly.name.as_deref(), Some("M_column_correction"));
        assert_eq!(fixture_node("v23tosky_jwst.asdf").name.as_deref(), Some("v23tosky"));
    }
}
