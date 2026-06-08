use serde_yaml::Value;

use super::parser::AsdfError;

#[derive(Debug, Clone)]
pub struct NdArrayMeta {
    pub source: usize,
    pub shape: Vec<usize>,
    pub dtype: DType,
    pub byteorder: ByteOrder,
    pub offset: usize,
    pub strides: Option<Vec<usize>>,
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
}

fn field_usize(node: &Value, key: &str) -> Option<usize> {
    node.get(key).and_then(|v| v.as_u64()).map(|n| n as usize)
}

fn field_usize_or(node: &Value, key: &str, default: usize) -> usize {
    field_usize(node, key).unwrap_or(default)
}

fn collect_usize(seq: &[Value]) -> Vec<usize> {
    seq.iter()
        .filter_map(|v| v.as_u64().map(|n| n as usize))
        .collect()
}

fn parse_strides(node: &Value, rank: usize) -> Option<Vec<usize>> {
    let seq = node.get("strides")?.as_sequence()?;
    let parsed = collect_usize(seq);
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
        let source =
            field_usize(node, "source").ok_or_else(|| AsdfError::MissingField("source".into()))?;

        let shape = node
            .get("shape")
            .and_then(|v| v.as_sequence())
            .map(|seq| collect_usize(seq))
            .ok_or_else(|| AsdfError::MissingField("shape".into()))?;

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
            dtype,
            byteorder,
            offset,
            strides,
        })
    }

    pub fn byte_size_per_element(&self) -> usize {
        self.dtype.byte_size()
    }

    pub fn expected_byte_size(&self) -> usize {
        self.shape.iter().product::<usize>() * self.byte_size_per_element()
    }

    pub fn element_count(&self) -> usize {
        self.shape.iter().product()
    }

    pub fn contiguous_strides(shape: &[usize], element_size: usize) -> Vec<usize> {
        let mut strides = vec![element_size; shape.len()];
        let mut acc = element_size;
        for axis in (0..shape.len()).rev() {
            strides[axis] = acc;
            acc = acc.saturating_mul(shape[axis]);
        }
        strides
    }

    pub fn effective_strides(&self) -> Vec<usize> {
        match &self.strides {
            Some(s) if s.len() == self.shape.len() => s.clone(),
            _ => Self::contiguous_strides(&self.shape, self.byte_size_per_element()),
        }
    }

    pub fn is_contiguous(&self) -> bool {
        self.offset == 0
            && self.effective_strides()
            == Self::contiguous_strides(&self.shape, self.byte_size_per_element())
    }
}

struct GwcsParams {
    crpix: [f64; 2],
    crval: [f64; 2],
    cdelt: [f64; 2],
    pc: [[f64; 2]; 2],
    shift_axis: usize,
    scale_axis: usize,
    recognized: bool,
}

type GwcsHandler = fn(&mut GwcsParams, &Value);

const GWCS_HANDLERS: &[(&[&str], GwcsHandler)] = &[
    (&["shift"], GwcsParams::apply_shift),
    (&["scale"], GwcsParams::apply_scale),
    (&["affine"], GwcsParams::apply_affine),
    (&["rotat"], GwcsParams::apply_rotation),
    (
        &["gnomonic", "pix2sky", "tan"],
        GwcsParams::apply_projection,
    ),
];

impl GwcsParams {
    fn new() -> Self {
        Self {
            crpix: [0.0, 0.0],
            crval: [0.0, 0.0],
            cdelt: [1.0, 1.0],
            pc: [[1.0, 0.0], [0.0, 1.0]],
            shift_axis: 0,
            scale_axis: 0,
            recognized: false,
        }
    }

    fn apply(&mut self, leaf: &Value) {
        let id = WcsInfo::model_id(leaf);
        if let Some(&(_, handler)) = GWCS_HANDLERS
            .iter()
            .find(|(keywords, _)| keywords.iter().any(|&kw| id.contains(kw)))
        {
            handler(self, leaf);
        }
    }

    fn write_next_axis(slot: &mut [f64; 2], axis: &mut usize, recognized: &mut bool, value: f64) {
        if *axis < slot.len() {
            slot[*axis] = value;
            *axis += 1;
            *recognized = true;
        }
    }

    fn apply_shift(&mut self, leaf: &Value) {
        if let Some(offset) = leaf.get("offset").and_then(Value::as_f64) {
            Self::write_next_axis(
                &mut self.crpix,
                &mut self.shift_axis,
                &mut self.recognized,
                -offset,
            );
        }
    }

    fn apply_scale(&mut self, leaf: &Value) {
        if let Some(factor) = leaf.get("factor").and_then(Value::as_f64) {
            Self::write_next_axis(
                &mut self.cdelt,
                &mut self.scale_axis,
                &mut self.recognized,
                factor,
            );
        }
    }

    fn apply_affine(&mut self, leaf: &Value) {
        if let Some(matrix) = WcsInfo::extract_matrix(leaf, "matrix") {
            self.pc = matrix;
            self.recognized = true;
        }
    }

    fn apply_rotation(&mut self, leaf: &Value) {
        if let Some(lon) = WcsInfo::first_f64(leaf, &["phi", "lon", "lon_0"]) {
            self.crval[0] = lon;
            self.recognized = true;
        }
        if let Some(lat) = WcsInfo::first_f64(leaf, &["theta", "lat", "lat_0"]) {
            self.crval[1] = lat;
            self.recognized = true;
        }
    }

    fn apply_projection(&mut self, leaf: &Value) {
        if let Some(lon) = WcsInfo::first_f64(leaf, &["lon_0"]) {
            self.crval[0] = lon;
        }
        if let Some(lat) = WcsInfo::first_f64(leaf, &["lat_0"]) {
            self.crval[1] = lat;
        }
        self.recognized = true;
    }
}

impl WcsInfo {
    pub fn from_yaml(tree: &Value) -> Option<Self> {
        let wcs = tree
            .get("wcs")
            .or_else(|| tree.get("meta").and_then(|m| m.get("wcs")))?;

        let crpix = Self::extract_pair(wcs, "crpix")?;
        let crval = Self::extract_pair(wcs, "crval")?;
        let cdelt = Self::extract_pair(wcs, "cdelt").unwrap_or([1.0, 1.0]);
        let pc = Self::extract_matrix(wcs, "pc").unwrap_or([[1.0, 0.0], [0.0, 1.0]]);
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
        })
    }

    pub fn from_gwcs(tree: &Value) -> Option<Self> {
        let gwcs = tree.get("gwcs").or_else(|| {
            tree.get("meta")
                .and_then(|m| m.get("wcs"))
                .filter(|w| w.get("steps").is_some())
        })?;

        let steps = gwcs.get("steps")?.as_sequence()?;

        let mut leaves = Vec::new();
        for step in steps {
            if let Some(transform) = step.get("transform").filter(|t| !t.is_null()) {
                Self::flatten_transform(transform, &mut leaves);
            }
        }

        let mut params = GwcsParams::new();
        for leaf in leaves {
            params.apply(leaf);
        }

        if !params.recognized {
            return None;
        }

        Some(Self {
            crpix: params.crpix,
            crval: params.crval,
            cdelt: params.cdelt,
            pc: params.pc,
            ctype: ["RA---TAN".into(), "DEC--TAN".into()],
            cunit: ["deg".into(), "deg".into()],
        })
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

    fn first_f64(node: &Value, keys: &[&str]) -> Option<f64> {
        keys.iter()
            .find_map(|k| node.get(*k).and_then(|v| v.as_f64()))
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

    #[test]
    fn test_complex64_byte_size() {
        let meta = NdArrayMeta {
            source: 0,
            shape: vec![100, 100],
            dtype: DType::Complex64,
            byteorder: ByteOrder::Little,
            offset: 0,
            strides: None,
        };
        assert_eq!(meta.byte_size_per_element(), 8);
        assert_eq!(meta.expected_byte_size(), 100 * 100 * 8);
    }

    #[test]
    fn test_float32_byte_size() {
        let meta = NdArrayMeta {
            source: 0,
            shape: vec![50, 50],
            dtype: DType::Float32,
            byteorder: ByteOrder::Big,
            offset: 0,
            strides: None,
        };
        assert_eq!(meta.byte_size_per_element(), 4);
        assert_eq!(meta.expected_byte_size(), 50 * 50 * 4);
    }

    #[test]
    fn test_float64_byte_size() {
        let meta = NdArrayMeta {
            source: 0,
            shape: vec![10, 20],
            dtype: DType::Float64,
            byteorder: ByteOrder::Little,
            offset: 0,
            strides: None,
        };
        assert_eq!(meta.byte_size_per_element(), 8);
        assert_eq!(meta.expected_byte_size(), 10 * 20 * 8);
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
            let meta = NdArrayMeta {
                source: 0,
                shape: vec![1],
                dtype,
                byteorder: ByteOrder::Little,
                offset: 0,
                strides: None,
            };
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
        assert_eq!(NdArrayMeta::contiguous_strides(&[], 8), Vec::<usize>::new());
    }

    #[test]
    fn test_default_offset_and_strides() {
        let node: Value =
            serde_yaml::from_str("source: 0\nshape: [100]\ndatatype: float64\nbyteorder: little\n")
                .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.offset, 0);
        assert_eq!(meta.strides, None);
        assert!(meta.is_contiguous());
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
        assert!(!meta.is_contiguous());
        assert_eq!(meta.effective_strides(), vec![16]);
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
        let tree: Value = serde_yaml::from_str(yaml).unwrap();
        let wcs = WcsInfo::from_gwcs(&tree).expect("gwcs chain parsed");
        assert_eq!(wcs.crpix, [1024.5, 1020.5]);
        assert_eq!(wcs.cdelt, [0.0001, 0.0002]);
        assert_eq!(wcs.pc, [[1.1, 0.2], [0.3, 1.2]]);
        assert_eq!(wcs.crval, [202.4695, 47.1953]);
        assert_eq!(wcs.ctype, ["RA---TAN".to_string(), "DEC--TAN".to_string()]);
    }

    #[test]
    fn test_gwcs_field_type_fallback() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: {transform_type: Shift, offset: -5.0}
      - transform: {transform_type: Shift, offset: -7.0}
      - transform: {transform_type: AffineTransformation, matrix: [[2.0, 0.0], [0.0, 3.0]]}
      - frame: {name: world}
"#;
        let tree: Value = serde_yaml::from_str(yaml).unwrap();
        let wcs = WcsInfo::from_gwcs(&tree).expect("gwcs field fallback parsed");
        assert_eq!(wcs.crpix, [5.0, 7.0]);
        assert_eq!(wcs.pc, [[2.0, 0.0], [0.0, 3.0]]);
        assert_eq!(wcs.crval, [0.0, 0.0]);
        assert_eq!(wcs.cdelt, [1.0, 1.0]);
    }

    #[test]
    fn test_gwcs_unrecognized_returns_none() {
        let yaml = r#"
meta:
  wcs:
    steps:
      - transform: !transform/identity-1.2.0 {}
      - frame: {name: world}
"#;
        let tree: Value = serde_yaml::from_str(yaml).unwrap();
        assert!(WcsInfo::from_gwcs(&tree).is_none());
    }
}
