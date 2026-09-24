use serde_yaml::Value;

use super::parser::AsdfError;

#[derive(Debug, Clone, PartialEq)]
pub enum ArraySource {
    Block(usize),
    Inline(Vec<f64>),
}

#[derive(Debug, Clone)]
pub struct NdArrayMeta {
    pub source: ArraySource,
    pub shape: Vec<usize>,
    pub streamed_first_dim: bool,
    pub dtype: DType,
    pub byteorder: ByteOrder,
    pub offset: usize,
    pub strides: Option<Vec<isize>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DType {
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Bool8,
    Complex64,
    Complex128,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ByteOrder {
    Big,
    Little,
}

#[derive(Debug, Clone)]
pub struct WcsInfo {
    pub crpix: [f64; 2],
    pub crval: [f64; 2],
    pub cdelt: [f64; 2],
    pub pc: [[f64; 2]; 2],
    pub ctype: [String; 2],
    pub cunit: [String; 2],
    pub lonpole: Option<f64>,
}

fn field_usize(node: &Value, key: &str) -> Option<usize> {
    node.get(key).and_then(|v| v.as_u64()).map(|n| n as usize)
}

fn field_usize_or(node: &Value, key: &str, default: usize) -> usize {
    field_usize(node, key).unwrap_or(default)
}

fn parse_shape(seq: &[Value]) -> Result<(Vec<usize>, bool), AsdfError> {
    let mut shape = Vec::with_capacity(seq.len());
    let mut streamed = false;
    for (i, v) in seq.iter().enumerate() {
        if let Some(n) = v.as_u64() {
            shape.push(n as usize);
        } else if i == 0 && v.as_str() == Some("*") {
            streamed = true;
            shape.push(0);
        } else {
            return Err(AsdfError::MissingField("shape".into()));
        }
    }
    Ok((shape, streamed))
}

fn parse_strides(node: &Value, rank: usize) -> Option<Vec<isize>> {
    let seq = node.get("strides")?.as_sequence()?;
    let parsed: Vec<isize> = seq
        .iter()
        .filter_map(|v| v.as_i64().map(|n| n as isize))
        .collect();
    (parsed.len() == seq.len() && parsed.len() == rank).then_some(parsed)
}

fn field_byteorder(node: &Value, fallback: ByteOrder) -> ByteOrder {
    node.get("byteorder")
        .and_then(|v| v.as_str())
        .map(ByteOrder::from_label)
        .unwrap_or(fallback)
}

fn split_byteorder_prefix(s: &str) -> (ByteOrder, &str) {
    match s.as_bytes().first() {
        Some(b'>') => (ByteOrder::Big, &s[1..]),
        Some(b'<') | Some(b'=') | Some(b'|') => (ByteOrder::Little, &s[1..]),
        _ => (ByteOrder::Little, s),
    }
}

pub(crate) fn untag(value: &Value) -> &Value {
    match value {
        Value::Tagged(tagged) => untag(&tagged.value),
        other => other,
    }
}

fn flatten_inline(node: &Value, out: &mut Vec<f64>, dims: &mut Vec<usize>, depth: usize) -> bool {
    match untag(node) {
        Value::Sequence(seq) => {
            if dims.len() == depth {
                dims.push(seq.len());
            } else if dims[depth] != seq.len() {
                return false;
            }
            seq.iter()
                .all(|child| flatten_inline(child, out, dims, depth + 1))
        }
        Value::Number(n) => {
            out.push(n.as_f64().unwrap_or(f64::NAN));
            true
        }
        Value::Bool(b) => {
            out.push(f64::from(u8::from(*b)));
            true
        }
        Value::Null => {
            out.push(f64::NAN);
            true
        }
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "nan" | ".nan" => {
                out.push(f64::NAN);
                true
            }
            "inf" | ".inf" | "+inf" => {
                out.push(f64::INFINITY);
                true
            }
            "-inf" | "-.inf" => {
                out.push(f64::NEG_INFINITY);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

impl DType {
    fn parse(s: &str) -> Result<(DType, ByteOrder), AsdfError> {
        let (order, type_str) = split_byteorder_prefix(s);
        let dtype = match type_str {
            "i1" | "int8" => DType::Int8,
            "i2" | "int16" => DType::Int16,
            "i4" | "int32" => DType::Int32,
            "i8" | "int64" => DType::Int64,
            "u1" | "uint8" => DType::UInt8,
            "u2" | "uint16" => DType::UInt16,
            "u4" | "uint32" => DType::UInt32,
            "u8" | "uint64" => DType::UInt64,
            "f4" | "float32" => DType::Float32,
            "f8" | "float64" => DType::Float64,
            "b1" | "bool8" | "bool" => DType::Bool8,
            "c8" | "complex64" => DType::Complex64,
            "c16" | "complex128" => DType::Complex128,
            other => return Err(AsdfError::InvalidDtype(other.into())),
        };
        Ok((dtype, order))
    }

    pub fn byte_size(&self) -> usize {
        match self {
            DType::Complex128 => 16,
            DType::Float64 | DType::Complex64 | DType::Int64 | DType::UInt64 => 8,
            DType::Float32 | DType::Int32 | DType::UInt32 => 4,
            DType::Int16 | DType::UInt16 => 2,
            DType::Int8 | DType::UInt8 | DType::Bool8 => 1,
        }
    }
}

impl ByteOrder {
    fn from_label(s: &str) -> Self {
        match s {
            "big" => ByteOrder::Big,
            _ => ByteOrder::Little,
        }
    }
}

impl NdArrayMeta {
    pub fn from_yaml(node: &Value) -> Result<Self, AsdfError> {
        if let Some(inline) = node.get("data") {
            return Self::from_inline(node, inline);
        }

        let source_node = node
            .get("source")
            .ok_or_else(|| AsdfError::MissingField("source".into()))?;
        let source = match source_node.as_u64() {
            Some(n) => ArraySource::Block(n as usize),
            None => {
                let uri = source_node
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{:?}", source_node));
                return Err(AsdfError::ExternalBlock(uri));
            }
        };

        let (shape, streamed_first_dim) = node
            .get("shape")
            .and_then(|v| v.as_sequence())
            .map(|seq| parse_shape(seq))
            .ok_or_else(|| AsdfError::MissingField("shape".into()))??;

        let dtype_str = node
            .get("datatype")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AsdfError::MissingField("datatype".into()))?;

        let (dtype, prefix_order) = DType::parse(dtype_str)?;
        let byteorder = field_byteorder(node, prefix_order);
        let offset = field_usize_or(node, "offset", 0);
        let strides = parse_strides(node, shape.len());

        Ok(Self {
            source,
            shape,
            streamed_first_dim,
            dtype,
            byteorder,
            offset,
            strides,
        })
    }

    fn from_inline(node: &Value, inline: &Value) -> Result<Self, AsdfError> {
        let mut values = Vec::new();
        let mut dims = Vec::new();
        if !flatten_inline(inline, &mut values, &mut dims, 0) {
            return Err(AsdfError::InvalidDtype("inline data".into()));
        }
        let shape = node
            .get("shape")
            .and_then(|v| v.as_sequence())
            .map(|seq| parse_shape(seq).map(|(s, _)| s))
            .transpose()?
            .unwrap_or(dims);
        let dtype = node
            .get("datatype")
            .and_then(|v| v.as_str())
            .map(DType::parse)
            .transpose()?
            .map(|(d, _)| d)
            .unwrap_or(DType::Float32);
        Ok(Self {
            source: ArraySource::Inline(values),
            shape,
            streamed_first_dim: false,
            dtype,
            byteorder: ByteOrder::Little,
            offset: 0,
            strides: None,
        })
    }

    pub fn resolve_streamed_shape(&mut self, block_len: usize) {
        if !self.streamed_first_dim || self.shape.is_empty() {
            return;
        }
        let rest: usize = self.shape[1..].iter().product::<usize>().max(1);
        let elem = self.byte_size_per_element().max(1);
        let usable = block_len.saturating_sub(self.offset);
        self.shape[0] = usable / (rest * elem);
        self.streamed_first_dim = false;
    }

    pub fn byte_size_per_element(&self) -> usize {
        self.dtype.byte_size()
    }

    pub fn contiguous_strides(shape: &[usize], element_size: usize) -> Vec<isize> {
        let mut strides = vec![element_size as isize; shape.len()];
        let mut acc = element_size as isize;
        for axis in (0..shape.len()).rev() {
            strides[axis] = acc;
            acc = acc.saturating_mul(shape[axis] as isize);
        }
        strides
    }

    pub fn effective_strides(&self) -> Vec<isize> {
        match &self.strides {
            Some(s) if s.len() == self.shape.len() => s.clone(),
            _ => Self::contiguous_strides(&self.shape, self.byte_size_per_element()),
        }
    }
}

const GWCS_PIXEL_ORIGIN_TO_FITS: f64 = 1.0;
const GWCS_LONPOLE: f64 = 180.0;
const GWCS_ANGLE_TOLERANCE_DEG: f64 = 1e-9;
const IDENTITY_2X2: [[f64; 2]; 2] = [[1.0, 0.0], [0.0, 1.0]];
const PIXEL_UNITS: &[(&str, f64)] = &[("", 1.0), ("pix", 1.0), ("pixel", 1.0)];
const SCALE_UNITS: &[(&str, f64)] = &[
    ("", 1.0),
    ("deg", 1.0),
    ("deg/pix", 1.0),
    ("deg/pixel", 1.0),
    ("arcsec", 1.0 / 3600.0),
    ("arcsec/pix", 1.0 / 3600.0),
    ("arcsec/pixel", 1.0 / 3600.0),
];
const ANGLE_UNITS: &[(&str, f64)] = &[
    ("", 1.0),
    ("deg", 1.0),
    ("degree", 1.0),
    ("rad", 180.0 / std::f64::consts::PI),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GwcsStage {
    Pixel,
    Linear,
    Projection,
    Celestial,
}

struct GwcsChain<'a> {
    arrays: &'a dyn Fn(&Value) -> Option<Vec<f64>>,
    stage: GwcsStage,
    shifts: Vec<f64>,
    scales: Vec<f64>,
    matrix: Option<[[f64; 2]; 2]>,
    scale_before_matrix: bool,
    projected: bool,
    crval: Option<[f64; 2]>,
}

fn gwcs_param(leaf: &Value, keys: &[&str], units: &[(&str, f64)]) -> Result<Option<f64>, String> {
    let Some((key, node)) = keys.iter().find_map(|k| leaf.get(*k).map(|v| (*k, v))) else {
        return Ok(None);
    };
    let (value, unit) = match node.as_f64() {
        Some(v) => (v, String::new()),
        None => {
            let value = node
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("gwcs parameter '{key}' is not a number"))?;
            let unit: String = node
                .get("unit")
                .and_then(Value::as_str)
                .unwrap_or("")
                .split_whitespace()
                .collect();
            (value, unit)
        }
    };
    let factor = units
        .iter()
        .find(|(u, _)| u.eq_ignore_ascii_case(&unit))
        .map(|&(_, f)| f)
        .ok_or_else(|| format!("gwcs parameter '{key}' has unsupported unit '{unit}'"))?;
    let scaled = value * factor;
    if scaled.is_finite() {
        Ok(Some(scaled))
    } else {
        Err(format!("gwcs parameter '{key}' is not finite"))
    }
}

fn gwcs_required(leaf: &Value, keys: &[&str], units: &[(&str, f64)], name: &str) -> Result<f64, String> {
    gwcs_param(leaf, keys, units)?
        .ok_or_else(|| format!("gwcs step '{name}' lacks '{}'", keys.join("/")))
}

fn gwcs_direction(leaf: &Value) -> String {
    leaf.get("direction")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn push_axis(axes: &mut Vec<f64>, value: f64, name: &str) -> Result<(), String> {
    if axes.len() == 2 {
        return Err(format!("gwcs has more than two '{name}' steps"));
    }
    axes.push(value);
    Ok(())
}

fn angle_is(value: f64, target: f64) -> bool {
    let d = (value - target).rem_euclid(360.0);
    d < GWCS_ANGLE_TOLERANCE_DEG || 360.0 - d < GWCS_ANGLE_TOLERANCE_DEG
}

impl<'a> GwcsChain<'a> {
    fn new(arrays: &'a dyn Fn(&Value) -> Option<Vec<f64>>) -> Self {
        Self {
            arrays,
            stage: GwcsStage::Pixel,
            shifts: Vec::new(),
            scales: Vec::new(),
            matrix: None,
            scale_before_matrix: false,
            projected: false,
            crval: None,
        }
    }

    fn enter(&mut self, stage: GwcsStage, name: &str) -> Result<(), String> {
        if stage < self.stage {
            return Err(format!("gwcs step '{name}' is out of FITS order"));
        }
        self.stage = stage;
        Ok(())
    }

    fn apply(&mut self, leaf: &Value) -> Result<(), String> {
        let name = WcsInfo::model_name(leaf);
        match name.as_str() {
            "shift" => {
                self.enter(GwcsStage::Pixel, &name)?;
                let offset = gwcs_required(leaf, &["offset"], PIXEL_UNITS, &name)?;
                push_axis(&mut self.shifts, offset, &name)
            }
            "scale" => {
                self.enter(GwcsStage::Linear, &name)?;
                let factor = gwcs_required(leaf, &["factor"], SCALE_UNITS, &name)?;
                push_axis(&mut self.scales, factor, &name)
            }
            "affine" | "affinetransformation" | "affinetransformation2d" => {
                self.apply_affine(leaf, &name)
            }
            "gnomonic" | "tan" | "pix2sky_tan" | "pix2sky_gnomonic" => {
                self.apply_projection(leaf, &name)
            }
            "rotate3d" | "rotatenative2celestial" => self.apply_rotation(leaf, &name),
            "" => Err("gwcs step has no readable transform tag".into()),
            other => Err(format!("gwcs step '{other}' is not TAN-convertible")),
        }
    }

    fn numbers(&self, leaf: &Value, key: &str, rank: usize) -> Result<Option<Vec<f64>>, String> {
        let Some(node) = leaf.get(key) else {
            return Ok(None);
        };
        let mut values = Vec::new();
        let mut dims = Vec::new();
        let plain = matches!(untag(node), Value::Sequence(_))
            && flatten_inline(node, &mut values, &mut dims, 0)
            && dims.len() == rank;
        let values = if plain {
            values
        } else {
            (self.arrays)(node).ok_or_else(|| format!("gwcs '{key}' values are not readable"))?
        };
        if values.iter().all(|v| v.is_finite()) {
            Ok(Some(values))
        } else {
            Err(format!("gwcs '{key}' values are not finite"))
        }
    }

    fn apply_affine(&mut self, leaf: &Value, name: &str) -> Result<(), String> {
        self.enter(GwcsStage::Linear, name)?;
        if self.matrix.is_some() || self.scales.len() == 1 {
            return Err(format!("gwcs '{name}' cannot be folded into PC/CDELT"));
        }
        let values = self
            .numbers(leaf, "matrix", 2)?
            .ok_or_else(|| format!("gwcs '{name}' has no matrix"))?;
        let &[a, b, c, d] = values.as_slice() else {
            return Err(format!("gwcs '{name}' matrix is not 2x2"));
        };
        if let Some(translation) = self.numbers(leaf, "translation", 1)? {
            if translation.iter().any(|v| *v != 0.0) {
                return Err(format!("gwcs '{name}' translation is not supported"));
            }
        }
        self.scale_before_matrix = self.scales.len() == 2;
        self.matrix = Some([[a, b], [c, d]]);
        Ok(())
    }

    fn apply_projection(&mut self, leaf: &Value, name: &str) -> Result<(), String> {
        self.enter(GwcsStage::Projection, name)?;
        if self.projected {
            return Err(format!("gwcs has more than one '{name}' projection"));
        }
        if !matches!(gwcs_direction(leaf).as_str(), "" | "pix2sky") {
            return Err(format!("gwcs '{name}' is not a pix2sky projection"));
        }
        self.projected = true;
        let lon = gwcs_param(leaf, &["lon_0"], ANGLE_UNITS)?;
        let lat = gwcs_param(leaf, &["lat_0"], ANGLE_UNITS)?;
        match (lon, lat) {
            (Some(lon), Some(lat)) => self.anchor(lon, lat, name),
            _ => Ok(()),
        }
    }

    fn apply_rotation(&mut self, leaf: &Value, name: &str) -> Result<(), String> {
        self.enter(GwcsStage::Celestial, name)?;
        if !matches!(gwcs_direction(leaf).as_str(), "" | "native2celestial") {
            return Err(format!("gwcs '{name}' is not a native-to-celestial rotation"));
        }
        let lon = gwcs_required(leaf, &["phi", "lon"], ANGLE_UNITS, name)?;
        let lat = gwcs_required(leaf, &["theta", "lat"], ANGLE_UNITS, name)?;
        let pole = gwcs_param(leaf, &["psi", "lon_pole"], ANGLE_UNITS)?.unwrap_or(GWCS_LONPOLE);
        if !angle_is(pole, GWCS_LONPOLE) {
            return Err(format!("gwcs '{name}' pole longitude {pole} needs LONPOLE"));
        }
        self.anchor(lon, lat, name)
    }

    fn anchor(&mut self, lon: f64, lat: f64, name: &str) -> Result<(), String> {
        if self.crval.is_some() {
            return Err(format!("gwcs '{name}' sets the celestial reference twice"));
        }
        if lat.abs() > 90.0 {
            return Err(format!("gwcs '{name}' latitude {lat} is outside [-90, 90]"));
        }
        self.crval = Some([lon, lat]);
        Ok(())
    }

    fn finish(self) -> Result<WcsInfo, String> {
        let crval = self
            .crval
            .ok_or_else(|| "gwcs has no celestial reference (rotate3d phi/theta)".to_string())?;
        if !self.projected {
            return Err("gwcs has no gnomonic (TAN) projection".into());
        }
        let crpix = match self.shifts.as_slice() {
            [] => [GWCS_PIXEL_ORIGIN_TO_FITS; 2],
            &[x, y] => [GWCS_PIXEL_ORIGIN_TO_FITS - x, GWCS_PIXEL_ORIGIN_TO_FITS - y],
            _ => return Err("gwcs shifts only one pixel axis".into()),
        };
        let scale = match self.scales.as_slice() {
            [] => [1.0, 1.0],
            &[x, y] => [x, y],
            _ => return Err("gwcs scales only one pixel axis".into()),
        };
        let m = self.matrix.unwrap_or(IDENTITY_2X2);
        let (cdelt, pc) = if self.scale_before_matrix && scale[0] != scale[1] {
            (
                [1.0, 1.0],
                [
                    [m[0][0] * scale[0], m[0][1] * scale[1]],
                    [m[1][0] * scale[0], m[1][1] * scale[1]],
                ],
            )
        } else {
            (scale, m)
        };
        let det = cdelt[0] * cdelt[1] * (pc[0][0] * pc[1][1] - pc[0][1] * pc[1][0]);
        if !det.is_finite() || det == 0.0 {
            return Err("gwcs linear transform is singular".into());
        }
        Ok(WcsInfo {
            crpix,
            crval,
            cdelt,
            pc,
            ctype: ["RA---TAN".into(), "DEC--TAN".into()],
            cunit: ["deg".into(), "deg".into()],
            lonpole: Some(GWCS_LONPOLE),
        })
    }
}

impl WcsInfo {
    fn wcs_nodes(tree: &Value) -> impl Iterator<Item = &Value> {
        [
            tree.get("wcs"),
            tree.get("meta").and_then(|m| m.get("wcs")),
            tree.get("roman")
                .and_then(|r| r.get("meta"))
                .and_then(|m| m.get("wcs")),
        ]
        .into_iter()
        .flatten()
    }

    pub fn from_yaml(tree: &Value) -> Option<Self> {
        Self::wcs_nodes(tree).find_map(Self::from_fits_like)
    }

    fn from_fits_like(wcs: &Value) -> Option<Self> {
        let crpix = Self::extract_pair(wcs, "crpix")?;
        let crval = Self::extract_pair(wcs, "crval")?;
        let cdelt = Self::extract_pair(wcs, "cdelt").unwrap_or([1.0, 1.0]);
        let pc = Self::extract_matrix(wcs, "pc").unwrap_or(IDENTITY_2X2);
        let ctype = Self::extract_string_pair(wcs, "ctype")
            .unwrap_or_else(|| ["RA---TAN".into(), "DEC--TAN".into()]);
        let cunit =
            Self::extract_string_pair(wcs, "cunit").unwrap_or_else(|| ["deg".into(), "deg".into()]);

        Some(Self {
            crpix,
            crval,
            cdelt,
            pc,
            ctype,
            cunit,
            lonpole: None,
        })
    }

    pub fn from_gwcs(
        tree: &Value,
        arrays: &dyn Fn(&Value) -> Option<Vec<f64>>,
    ) -> Result<Option<Self>, String> {
        let Some(gwcs) = tree
            .get("gwcs")
            .into_iter()
            .chain(Self::wcs_nodes(tree))
            .find(|w| w.get("steps").is_some())
        else {
            return Ok(None);
        };
        let steps = gwcs
            .get("steps")
            .and_then(Value::as_sequence)
            .ok_or_else(|| "gwcs steps are not a list".to_string())?;

        let mut leaves = Vec::new();
        for step in steps {
            if let Some(transform) = step.get("transform").filter(|t| !t.is_null()) {
                Self::flatten_transform(transform, &mut leaves);
            }
        }

        let mut chain = GwcsChain::new(arrays);
        for leaf in leaves {
            chain.apply(leaf)?;
        }
        Self::check_world_frame(steps)?;
        chain.finish().map(Some)
    }

    fn check_world_frame(steps: &[Value]) -> Result<(), String> {
        let Some(frame) = steps.last().and_then(|s| s.get("frame")) else {
            return Ok(());
        };
        let axis_types: Vec<String> = frame
            .get("axis_physical_types")
            .and_then(Value::as_sequence)
            .map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|t| t.trim().to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        if axis_types.iter().any(|t| !t.starts_with("pos.eq.")) {
            return Err(format!("gwcs world axes {} are not RA/Dec", axis_types.join(", ")));
        }
        let reference = frame.get("reference_frame").map(Self::model_name).unwrap_or_default();
        if reference.is_empty() || reference == "icrs" || reference == "fk5" {
            Ok(())
        } else {
            Err(format!("gwcs world frame '{reference}' is not ICRS"))
        }
    }

    fn flatten_transform<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        let id = Self::model_id(node);
        if id.contains("compose") || id.contains("concatenate") {
            if let Some(forward) = node.get("forward").and_then(|f| f.as_sequence()) {
                for child in forward {
                    Self::flatten_transform(child, out);
                }
                return;
            }
        }
        out.push(node);
    }

    fn model_id(node: &Value) -> String {
        let tag = match node {
            Value::Tagged(tagged) => tagged.tag.to_string(),
            _ => String::new(),
        };
        let field = node
            .get("transform_type")
            .or_else(|| node.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        format!("{} {}", tag, field).to_lowercase()
    }

    fn model_name(node: &Value) -> String {
        let from_tag = match node {
            Value::Tagged(tagged) => {
                let tag = tagged.tag.to_string();
                let last = tag
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('!')
                    .trim_end_matches('>');
                match last.rfind('-') {
                    Some(i) if last[i + 1..].starts_with(|c: char| c.is_ascii_digit()) => {
                        last[..i].to_string()
                    }
                    _ => last.to_string(),
                }
            }
            _ => String::new(),
        };
        let name = if from_tag.is_empty() {
            node.get("transform_type")
                .or_else(|| node.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        } else {
            from_tag
        };
        name.trim().to_ascii_lowercase()
    }

    fn extract_pair(node: &Value, key: &str) -> Option<[f64; 2]> {
        match node.get(key)?.as_sequence()?.as_slice() {
            [a, b, ..] => Some([a.as_f64()?, b.as_f64()?]),
            _ => None,
        }
    }

    fn extract_matrix(node: &Value, key: &str) -> Option<[[f64; 2]; 2]> {
        let [row0, row1, ..] = node.get(key)?.as_sequence()?.as_slice() else {
            return None;
        };
        match (
            row0.as_sequence()?.as_slice(),
            row1.as_sequence()?.as_slice(),
        ) {
            ([a, b, ..], [c, d, ..]) => {
                Some([[a.as_f64()?, b.as_f64()?], [c.as_f64()?, d.as_f64()?]])
            }
            _ => None,
        }
    }

    fn extract_string_pair(node: &Value, key: &str) -> Option<[String; 2]> {
        match node.get(key)?.as_sequence()?.as_slice() {
            [a, b, ..] => Some([a.as_str()?.to_string(), b.as_str()?.to_string()]),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_meta(shape: Vec<usize>, dtype: DType, byteorder: ByteOrder) -> NdArrayMeta {
        NdArrayMeta {
            source: ArraySource::Block(0),
            shape,
            streamed_first_dim: false,
            dtype,
            byteorder,
            offset: 0,
            strides: None,
        }
    }

    #[test]
    fn test_parse_dtype_full_set() {
        let cases = [
            ("int8", DType::Int8, 1),
            ("int16", DType::Int16, 2),
            ("int32", DType::Int32, 4),
            ("int64", DType::Int64, 8),
            ("uint8", DType::UInt8, 1),
            ("uint16", DType::UInt16, 2),
            ("uint32", DType::UInt32, 4),
            ("uint64", DType::UInt64, 8),
            ("float32", DType::Float32, 4),
            ("float64", DType::Float64, 8),
            ("bool8", DType::Bool8, 1),
            ("complex64", DType::Complex64, 8),
            ("complex128", DType::Complex128, 16),
        ];
        for (name, expected, size) in cases {
            let (dtype, _) = DType::parse(name).unwrap();
            assert_eq!(dtype, expected, "dtype mismatch for {}", name);
            let meta = block_meta(vec![1], dtype, ByteOrder::Little);
            assert_eq!(
                meta.byte_size_per_element(),
                size,
                "size mismatch for {}",
                name
            );
        }
    }

    #[test]
    fn test_byteorder_field_overrides_prefix() {
        let node: Value =
            serde_yaml::from_str("source: 0\nshape: [4]\ndatatype: int32\nbyteorder: big\n")
                .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.byteorder, ByteOrder::Big);
        assert_eq!(meta.dtype, DType::Int32);
    }

    #[test]
    fn test_unsupported_dtype_errors() {
        assert!(DType::parse("float16").is_err());
    }

    #[test]
    fn test_contiguous_strides_row_major() {
        assert_eq!(NdArrayMeta::contiguous_strides(&[2, 3], 8), vec![24, 8]);
        assert_eq!(NdArrayMeta::contiguous_strides(&[4], 4), vec![4]);
        assert_eq!(
            NdArrayMeta::contiguous_strides(&[2, 2, 2, 2], 4),
            vec![32, 16, 8, 4]
        );
        assert_eq!(NdArrayMeta::contiguous_strides(&[], 8), Vec::<isize>::new());
    }

    #[test]
    fn test_default_offset_and_strides() {
        let node: Value =
            serde_yaml::from_str("source: 0\nshape: [100]\ndatatype: float64\nbyteorder: little\n")
                .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.offset, 0);
        assert_eq!(meta.strides, None);
        assert_eq!(meta.effective_strides(), vec![8]);
    }

    #[test]
    fn test_explicit_strides_parsed() {
        let node: Value = serde_yaml::from_str(
            "source: 0\nshape: [50]\ndatatype: float64\nbyteorder: little\nstrides: [16]\noffset: 8\n",
        )
            .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.offset, 8);
        assert_eq!(meta.strides, Some(vec![16]));
        assert_eq!(meta.effective_strides(), vec![16]);
    }

    #[test]
    fn test_negative_strides_parsed() {
        let node: Value = serde_yaml::from_str(
            "source: 0\nshape: [2, 3]\ndatatype: float32\nbyteorder: little\nstrides: [-12, 4]\noffset: 12\n",
        )
        .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.strides, Some(vec![-12, 4]));
        assert_eq!(meta.effective_strides(), vec![-12, 4]);
    }

    #[test]
    fn test_strides_wrong_length_ignored() {
        let node: Value = serde_yaml::from_str(
            "source: 0\nshape: [4, 4]\ndatatype: float32\nbyteorder: little\nstrides: [4]\n",
        )
            .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.strides, None);
        assert_eq!(meta.effective_strides(), vec![16, 4]);
    }

    #[test]
    fn test_streamed_shape_wildcard() {
        let node: Value =
            serde_yaml::from_str("source: 0\nshape: ['*', 4]\ndatatype: float32\nbyteorder: little\n")
                .unwrap();
        let mut meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert!(meta.streamed_first_dim);
        meta.resolve_streamed_shape(3 * 4 * 4);
        assert_eq!(meta.shape, vec![3, 4]);
        assert!(!meta.streamed_first_dim);
    }

    #[test]
    fn test_external_source_is_loud_error() {
        let node: Value =
            serde_yaml::from_str("source: external0.asdf\nshape: [4]\ndatatype: float32\n").unwrap();
        match NdArrayMeta::from_yaml(&node) {
            Err(AsdfError::ExternalBlock(uri)) => assert_eq!(uri, "external0.asdf"),
            other => panic!("expected ExternalBlock, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_data_parsed() {
        let node: Value = serde_yaml::from_str("data: [[1, 2, 3], [4, 5, 6]]\n").unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.shape, vec![2, 3]);
        match meta.source {
            ArraySource::Inline(v) => assert_eq!(v, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            other => panic!("expected inline, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_ragged_rejected() {
        let node: Value = serde_yaml::from_str("data: [[1, 2, 3], [4, 5]]\n").unwrap();
        assert!(NdArrayMeta::from_yaml(&node).is_err());
    }

    fn inline_arrays(node: &Value) -> Option<Vec<f64>> {
        match NdArrayMeta::from_yaml(node).ok()?.source {
            ArraySource::Inline(values) => Some(values),
            ArraySource::Block(_) => None,
        }
    }

    fn gwcs(yaml: &str) -> Result<Option<WcsInfo>, String> {
        let tree: Value = serde_yaml::from_str(yaml).unwrap();
        WcsInfo::from_gwcs(&tree, &inline_arrays)
    }

    #[test]
    fn test_inline_integers_keep_every_bit() {
        let node: Value =
            serde_yaml::from_str("data: [[2147483649, 16777217], [4294967295, 0]]\ndatatype: uint32\n").unwrap();
        match NdArrayMeta::from_yaml(&node).unwrap().source {
            ArraySource::Inline(v) => assert_eq!(v, vec![2147483649.0, 16777217.0, 4294967295.0, 0.0]),
            other => panic!("expected inline, got {:?}", other),
        }
    }

    #[test]
    fn test_gwcs_tagged_compose_chain() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: !transform/compose-1.2.0
          forward:
            - !transform/concatenate-1.2.0
                forward:
                  - !transform/shift-1.2.0 {offset: -1024.5}
                  - !transform/shift-1.2.0 {offset: -1020.5}
            - !transform/affine-1.3.0 {matrix: [[1.1, 0.2], [0.3, 1.2]]}
            - !transform/concatenate-1.2.0
                forward:
                  - !transform/scale-1.2.0 {factor: 0.0001}
                  - !transform/scale-1.2.0 {factor: 0.0002}
            - !transform/gnomonic-1.2.0 {direction: pix2sky}
            - !transform/rotate3d-1.3.0 {phi: 202.4695, theta: 47.1953, psi: 180.0, direction: native2celestial}
      - frame: {name: world}
"#;
        let wcs = gwcs(yaml).unwrap().expect("gwcs chain parsed");
        assert_eq!(wcs.crpix, [1025.5, 1021.5]);
        assert_eq!(wcs.cdelt, [0.0001, 0.0002]);
        assert_eq!(wcs.pc, [[1.1, 0.2], [0.3, 1.2]]);
        assert_eq!(wcs.crval, [202.4695, 47.1953]);
        assert_eq!(wcs.ctype, ["RA---TAN".to_string(), "DEC--TAN".to_string()]);
        assert_eq!(wcs.lonpole, Some(180.0));
    }

    #[test]
    fn test_gwcs_quantity_angles_and_ndarray_matrix_resolve() {
        let yaml = r#"
wcs:
  steps:
    - frame: detector
      transform: !transform/compose-1.2.0
        forward:
          - !transform/concatenate-1.2.0
              forward:
                - !transform/shift-1.2.0 {offset: -99.0}
                - !transform/shift-1.2.0 {offset: !unit/quantity-1.1.0 {value: -49.0, unit: !unit/unit-1.0.0 pixel}}
          - !transform/affine-1.3.0
              matrix: !core/ndarray-1.0.0 {data: [[0.0, -1.0], [1.0, 0.0]], datatype: float64, shape: [2, 2]}
              translation: !core/ndarray-1.0.0 {data: [0.0, 0.0], datatype: float64, shape: [2]}
          - !transform/concatenate-1.2.0
              forward:
                - !transform/scale-1.2.0 {factor: !unit/quantity-1.1.0 {value: 0.1, unit: !unit/unit-1.0.0 arcsec / pix}}
                - !transform/scale-1.2.0 {factor: !unit/quantity-1.1.0 {value: 0.1, unit: !unit/unit-1.0.0 arcsec / pix}}
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate3d-1.3.0
              direction: native2celestial
              phi: !unit/quantity-1.1.0 {value: 270.1, unit: !unit/unit-1.0.0 deg}
              theta: !unit/quantity-1.1.0 {value: -30.25, unit: !unit/unit-1.0.0 deg}
              psi: !unit/quantity-1.1.0 {value: 180.0, unit: !unit/unit-1.0.0 deg}
    - frame: !<tag:stsci.edu:gwcs/celestial_frame-1.0.0>
        name: world
        reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0> {frame_attributes: {}}
      transform: null
"#;
        let wcs = gwcs(yaml).unwrap().expect("wcs_from_fiducial chain with quantities");
        assert_eq!(wcs.crval, [270.1, -30.25]);
        assert_eq!(wcs.crpix, [100.0, 50.0]);
        assert_eq!(wcs.pc, [[0.0, -1.0], [1.0, 0.0]]);
        assert!((wcs.cdelt[0] - 0.1 / 3600.0).abs() < 1e-18);
        assert!((wcs.cdelt[1] - 0.1 / 3600.0).abs() < 1e-18);
    }

    #[test]
    fn test_gwcs_scale_before_anisotropic_affine_folds_into_pc() {
        let yaml = r#"
wcs:
  steps:
    - transform: !transform/compose-1.2.0
        forward:
          - !transform/scale-1.2.0 {factor: 2.0}
          - !transform/scale-1.2.0 {factor: 3.0}
          - !transform/affine-1.3.0 {matrix: [[1.0, 0.5], [0.25, 1.0]]}
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate3d-1.3.0 {phi: 10.0, theta: 20.0, psi: 180.0}
"#;
        let wcs = gwcs(yaml).unwrap().unwrap();
        assert_eq!(wcs.cdelt, [1.0, 1.0]);
        assert_eq!(wcs.pc, [[2.0, 1.5], [0.5, 3.0]]);
        assert_eq!(wcs.crpix, [1.0, 1.0]);
    }

    #[test]
    fn test_gwcs_field_type_chain_with_projection_and_rotation() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: {transform_type: Shift, offset: -5.0}
      - transform: {transform_type: Shift, offset: -7.0}
      - transform: {transform_type: AffineTransformation, matrix: [[2.0, 0.0], [0.0, 3.0]]}
      - transform: {transform_type: Pix2Sky_TAN}
      - transform: {transform_type: RotateNative2Celestial, lon: 12.5, lat: -45.0, lon_pole: 180.0}
      - frame: {name: world}
"#;
        let wcs = gwcs(yaml).unwrap().expect("gwcs field fallback parsed");
        assert_eq!(wcs.crpix, [6.0, 8.0]);
        assert_eq!(wcs.pc, [[2.0, 0.0], [0.0, 3.0]]);
        assert_eq!(wcs.crval, [12.5, -45.0]);
        assert_eq!(wcs.cdelt, [1.0, 1.0]);
    }

    #[test]
    fn test_gwcs_without_celestial_step_is_refused() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: {transform_type: Shift, offset: -5.0}
      - transform: {transform_type: Shift, offset: -7.0}
      - transform: {transform_type: AffineTransformation, matrix: [[2.0, 0.0], [0.0, 3.0]]}
      - frame: {name: world}
"#;
        let reason = gwcs(yaml).expect_err("a pixel-only chain has no sky position");
        assert!(reason.contains("celestial reference"), "{reason}");
    }

    #[test]
    fn test_gwcs_under_roman_meta_without_anchor_is_refused() {
        let yaml = r#"
roman:
  meta:
    wcs:
      steps:
        - transform: {transform_type: Shift, offset: -2043.5}
        - transform: {transform_type: Shift, offset: -2043.5}
        - transform: {transform_type: Scale, factor: 0.00003}
        - transform: {transform_type: Scale, factor: 0.00003}
        - frame: {name: world}
"#;
        let tree: Value = serde_yaml::from_str(yaml).unwrap();
        assert!(WcsInfo::from_yaml(&tree).is_none());
        let reason = WcsInfo::from_gwcs(&tree, &inline_arrays).expect_err("no CRVAL anywhere in the chain");
        assert!(reason.contains("celestial reference"), "{reason}");
    }

    #[test]
    fn test_gwcs_v2v3_chain_is_refused_instead_of_faked_at_crval_zero() {
        let yaml = r#"
roman:
  meta:
    wcs:
      steps:
        - frame: detector
          transform: !transform/compose-1.2.0
            forward:
              - !transform/concatenate-1.2.0
                  forward:
                    - !transform/shift-1.2.0 {offset: 1.0}
                    - !transform/shift-1.2.0 {offset: 1.0}
              - !transform/concatenate-1.2.0
                  forward:
                    - !transform/polynomial-1.2.0 {coefficients: [[0.0, 0.11], [0.11, 0.0]]}
                    - !transform/polynomial-1.2.0 {coefficients: [[0.0, 0.11], [0.11, 0.0]]}
        - frame: v2v3
          transform: !transform/compose-1.2.0
            forward:
              - !transform/concatenate-1.2.0
                  forward:
                    - !transform/scale-1.2.0 {factor: 0.0002777777777777778}
                    - !transform/scale-1.2.0 {factor: 0.0002777777777777778}
              - !transform/spherical_cartesian-1.2.0 {transform_type: spherical_to_cartesian}
              - !transform/rotate_sequence_3d-1.0.0 {angles: [0.1, -0.2, 60.0, -30.0, -270.0], axes_order: zyxyz}
        - frame: world
          transform: null
"#;
        let reason = gwcs(yaml).expect_err("a V2/V3 chain has no FITS TAN form");
        assert!(reason.contains("polynomial"), "{reason}");
    }

    #[test]
    fn test_gwcs_rotation_sequence_alone_is_not_an_anchor() {
        let yaml = r#"
wcs:
  steps:
    - transform: !transform/compose-1.2.0
        forward:
          - !transform/shift-1.2.0 {offset: -10.0}
          - !transform/shift-1.2.0 {offset: -10.0}
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate_sequence_3d-1.0.0 {angles: [0.1, -0.2, 60.0, -30.0, -270.0], axes_order: zyxyz}
"#;
        let reason = gwcs(yaml).expect_err("angles are not phi/theta");
        assert!(reason.contains("rotate_sequence_3d"), "{reason}");
    }

    #[test]
    fn test_gwcs_rejects_non_default_lonpole_and_non_icrs_frame() {
        let rotated_pole = r#"
wcs:
  steps:
    - transform: !transform/compose-1.2.0
        forward:
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate3d-1.3.0 {phi: 10.0, theta: 20.0, psi: 90.0}
"#;
        assert!(gwcs(rotated_pole).unwrap_err().contains("LONPOLE"));

        let galactic = r#"
wcs:
  steps:
    - transform: !transform/compose-1.2.0
        forward:
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate3d-1.3.0 {phi: 10.0, theta: 20.0, psi: 180.0}
    - frame: !<tag:stsci.edu:gwcs/celestial_frame-1.0.0>
        axes_names: [l, b]
        axis_physical_types: [pos.galactic.lon, pos.galactic.lat]
        reference_frame: !<tag:astropy.org:astropy/coordinates/frames/galactic-1.0.0> {frame_attributes: {}}
      transform: null
"#;
        assert!(gwcs(galactic).unwrap_err().contains("pos.galactic.lon"));

        let local_tag = r#"
wcs:
  steps:
    - transform: !transform/compose-1.2.0
        forward:
          - !transform/gnomonic-1.2.0 {direction: pix2sky}
          - !transform/rotate3d-1.3.0 {phi: 10.0, theta: 20.0, psi: 180.0}
    - frame:
        reference_frame: !frames/galactic-1.0.0 {}
      transform: null
"#;
        assert!(gwcs(local_tag).unwrap_err().contains("galactic"));

        let equatorial = galactic
            .replace("[l, b]", "[ra, dec]")
            .replace("pos.galactic.lon, pos.galactic.lat", "pos.eq.ra, pos.eq.dec")
            .replace("galactic-1.0.0", "icrs-1.1.0");
        assert_eq!(gwcs(&equatorial).unwrap().unwrap().crval, [10.0, 20.0]);
    }

    #[test]
    fn test_gwcs_unrecognized_is_refused_with_reason() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: !transform/identity-1.2.0 {}
      - frame: {name: world}
"#;
        let reason = gwcs(yaml).expect_err("identity carries no sky position");
        assert!(reason.contains("identity"), "{reason}");
    }

    #[test]
    fn test_no_gwcs_node_is_not_an_error() {
        assert!(gwcs("meta:\n  telescope: JWST\n").unwrap().is_none());
    }
}
