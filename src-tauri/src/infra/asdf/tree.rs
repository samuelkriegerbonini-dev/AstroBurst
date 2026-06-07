use serde_yaml::Value;

use super::parser::AsdfError;

#[derive(Debug, Clone)]
pub struct NdArrayMeta {
    pub source: usize,
    pub shape: Vec<usize>,
    pub dtype: DType,
    pub byteorder: ByteOrder,
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

#[derive(Debug, Clone, PartialEq)]
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

impl NdArrayMeta {
    pub fn from_yaml(node: &Value) -> Result<Self, AsdfError> {
        let source = node
            .get("source")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| AsdfError::MissingField("source".into()))? as usize;

        let shape = node
            .get("shape")
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| AsdfError::MissingField("shape".into()))?
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as usize))
            .collect();

        let dtype_str = node
            .get("datatype")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AsdfError::MissingField("datatype".into()))?;

        let (dtype, prefix_order) = Self::parse_dtype(dtype_str)?;

        let byteorder = node
            .get("byteorder")
            .and_then(|v| v.as_str())
            .map(|s| if s == "big" { ByteOrder::Big } else { ByteOrder::Little })
            .unwrap_or(prefix_order);

        Ok(Self {
            source,
            shape,
            dtype,
            byteorder,
        })
    }

    fn parse_dtype(s: &str) -> Result<(DType, ByteOrder), AsdfError> {
        let (order, type_str) = if s.starts_with('>') {
            (ByteOrder::Big, &s[1..])
        } else if s.starts_with('<') {
            (ByteOrder::Little, &s[1..])
        } else if s.starts_with('=') || s.starts_with('|') {
            (ByteOrder::Little, &s[1..])
        } else {
            (ByteOrder::Little, s)
        };

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

    pub fn byte_size_per_element(&self) -> usize {
        match self.dtype {
            DType::Complex128 => 16,
            DType::Float64 | DType::Complex64 | DType::Int64 | DType::UInt64 => 8,
            DType::Float32 | DType::Int32 | DType::UInt32 => 4,
            DType::Int16 | DType::UInt16 => 2,
            DType::Int8 | DType::UInt8 | DType::Bool8 => 1,
        }
    }

    pub fn expected_byte_size(&self) -> usize {
        self.shape.iter().product::<usize>() * self.byte_size_per_element()
    }
}

impl WcsInfo {
    pub fn from_yaml(tree: &Value) -> Option<Self> {
        let wcs = tree.get("wcs").or_else(|| {
            tree.get("meta")
                .and_then(|m| m.get("wcs"))
        })?;

        let crpix = Self::extract_pair(wcs, "crpix")?;
        let crval = Self::extract_pair(wcs, "crval")?;
        let cdelt = Self::extract_pair(wcs, "cdelt").unwrap_or([1.0, 1.0]);

        let pc = Self::extract_matrix(wcs, "pc").unwrap_or([[1.0, 0.0], [0.0, 1.0]]);

        let ctype = Self::extract_string_pair(wcs, "ctype")
            .unwrap_or_else(|| ["RA---TAN".into(), "DEC--TAN".into()]);

        let cunit = Self::extract_string_pair(wcs, "cunit")
            .unwrap_or_else(|| ["deg".into(), "deg".into()]);

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

        let mut crpix = [0.0, 0.0];
        let mut crval = [0.0, 0.0];
        let mut cdelt = [1.0, 1.0];
        let mut pc = [[1.0, 0.0], [0.0, 1.0]];

        for step in steps {
            let transform = match step.get("transform") {
                Some(t) => t,
                None => continue,
            };

            let ttype = transform
                .get("transform_type")
                .or_else(|| transform.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match ttype {
                s if s.contains("Shift") => {
                    if let Some(offset) = transform.get("offset").and_then(|v| v.as_f64()) {
                        if crpix[0] == 0.0 {
                            crpix[0] = -offset;
                        } else {
                            crpix[1] = -offset;
                        }
                    }
                }
                s if s.contains("Scale") => {
                    if let Some(factor) = transform.get("factor").and_then(|v| v.as_f64()) {
                        if cdelt[0] == 1.0 {
                            cdelt[0] = factor;
                        } else {
                            cdelt[1] = factor;
                        }
                    }
                }
                s if s.contains("AffineTransformation") || s.contains("Rotation") => {
                    if let Some(matrix) = Self::extract_matrix(transform, "matrix") {
                        pc = matrix;
                    }
                }
                s if s.contains("Pix2Sky") || s.contains("TAN") => {
                    if let Some(lon) = transform.get("lon_0").and_then(|v| v.as_f64()) {
                        crval[0] = lon;
                    }
                    if let Some(lat) = transform.get("lat_0").and_then(|v| v.as_f64()) {
                        crval[1] = lat;
                    }
                }
                _ => {}
            }
        }

        Some(Self {
            crpix,
            crval,
            cdelt,
            pc,
            ctype: ["RA---TAN".into(), "DEC--TAN".into()],
            cunit: ["deg".into(), "deg".into()],
        })
    }

    fn extract_pair(node: &Value, key: &str) -> Option<[f64; 2]> {
        let arr = node.get(key)?.as_sequence()?;
        if arr.len() >= 2 {
            Some([arr[0].as_f64()?, arr[1].as_f64()?])
        } else {
            None
        }
    }

    fn extract_matrix(node: &Value, key: &str) -> Option<[[f64; 2]; 2]> {
        let mat = node.get(key)?.as_sequence()?;
        if mat.len() >= 2 {
            let row0 = mat[0].as_sequence()?;
            let row1 = mat[1].as_sequence()?;
            Some([
                [row0[0].as_f64()?, row0[1].as_f64()?],
                [row1[0].as_f64()?, row1[1].as_f64()?],
            ])
        } else {
            None
        }
    }

    fn extract_string_pair(node: &Value, key: &str) -> Option<[String; 2]> {
        let arr = node.get(key)?.as_sequence()?;
        if arr.len() >= 2 {
            Some([
                arr[0].as_str()?.to_string(),
                arr[1].as_str()?.to_string(),
            ])
        } else {
            None
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
            let (dtype, _) = NdArrayMeta::parse_dtype(name).unwrap();
            assert_eq!(dtype, expected, "dtype mismatch for {}", name);
            let meta = NdArrayMeta {
                source: 0,
                shape: vec![1],
                dtype,
                byteorder: ByteOrder::Little,
            };
            assert_eq!(meta.byte_size_per_element(), size, "size mismatch for {}", name);
        }
    }

    #[test]
    fn test_byteorder_field_overrides_prefix() {
        let node: Value = serde_yaml::from_str(
            "source: 0\nshape: [4]\ndatatype: int32\nbyteorder: big\n",
        )
        .unwrap();
        let meta = NdArrayMeta::from_yaml(&node).unwrap();
        assert_eq!(meta.byteorder, ByteOrder::Big);
        assert_eq!(meta.dtype, DType::Int32);
    }

    #[test]
    fn test_unsupported_dtype_errors() {
        assert!(NdArrayMeta::parse_dtype("float16").is_err());
    }
}
