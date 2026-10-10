pub mod inverse;
pub mod parse;
pub mod pipeline;
pub mod sip_fit;
pub mod source;
pub mod transform;

pub use inverse::{NewtonOptions, GWCS_INVERSE_ACCEPT_PX};
pub use parse::{
    classify_wcs_node, find_wcs_node, is_fitswcs_imaging, missing_wcs_node, parse_gwcs, parse_node,
    tag_family, wcs_node_at, wcsinfo_sip_residuals, ArrayResolver, WcsNodeClass, WcsNodeKey,
    WCS_NODE_SEARCH_ORDER,
};
pub use pipeline::{BoundingBox, FrameKind, GwcsFrame, GwcsPipeline, GwcsStep, InverseResult};
pub use sip_fit::{fit_pipeline_tan_sip, fit_tan_sip, SipFit, SipFitOptions, SIP_FIT_MAX_DEGREE};
pub use source::{GwcsOrigin, GwcsSource};
pub use transform::{Axis, Kind, Node, Vals, MAX_LANES};

#[derive(Debug, Clone, thiserror::Error)]
pub enum GwcsError {
    #[error("gWCS transform '{tag}' at {path} is not supported by the evaluator")]
    UnsupportedTag { tag: String, path: String },
    #[error("gWCS transform '{tag}' at {path} has no inverse (none stored, none derivable)")]
    NoInverse { tag: String, path: String },
    #[error("gWCS transform at {path}: {reason}")]
    Shape { path: String, reason: String },
    #[error("gWCS frame: {0}")]
    Frame(String),
    #[error("gWCS array at {path}: {reason}")]
    Arrays { path: String, reason: String },
    #[error("gWCS node at {path} lacks '{field}'")]
    Missing { path: String, field: String },
    #[error("gWCS bounding box: {0}")]
    Bbox(String),
    #[error("gWCS: {0}")]
    Parse(String),
}

#[cfg(test)]
mod tests {
    #[test]
    fn pipeline_re_exports_newton_options_and_vals() {
        let opts = super::pipeline::NewtonOptions::default();
        assert_eq!((opts.tol_px, opts.max_iter, opts.h_px), (1e-7, 10, 0.01));
        let v = super::pipeline::Vals::pair(1.0, 2.0);
        assert_eq!(v.as_slice(), &[1.0, 2.0]);
        let g: super::GwcsPipeline = match super::test_support::try_fixture_pipeline("wcs_gwcs_examples_tan.asdf") {
            Ok(p) => p,
            Err(e) => panic!("{e:#}"),
        };
        let r = g.backward_exact(29.980362905902044, 44.986109428818445, &opts);
        assert!(r.converged, "{r:?}");
        super::test_support::assert_close_slice(&r.pixel, &[0.0, 0.0], 1e-6, "TAN example origin");
    }

    #[test]
    fn real_data_directories_come_only_from_the_environment() {
        use super::test_support::{env_dir, skip_if_absent};
        let unset = env_dir("ASTROBURST_GWCS_TEST_UNSET_VARIABLE");
        assert_eq!(unset.to_str(), Some("<ASTROBURST_GWCS_TEST_UNSET_VARIABLE unset>"));
        assert!(skip_if_absent(&unset.join("any.fits")), "an unset variable skips the real-data test");
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use serde::Deserialize;
    use serde_yaml::Value;

    use super::parse::{find_wcs_node, parse_gwcs, parse_node};
    use super::pipeline::GwcsPipeline;
    use super::transform::Node;
    use super::GwcsError;
    use crate::infra::asdf::parser::{parse_tree, AsdfFile};
    use crate::infra::asdf::AsdfImage;

    pub(crate) const TRUTH_DIR_VAR: &str = "ASTROBURST_GWCS_TRUTH_DIR";
    pub(crate) const REAL_DATA_DIR_VAR: &str = "ASTROBURST_GWCS_DATA_DIR";
    pub(crate) const MAST_DIR_VAR: &str = "ASTROBURST_MAST_DIR";
    pub(crate) const DOCUMENTS_FITS_DIR_VAR: &str = "ASTROBURST_DOCUMENTS_FITS_DIR";

    pub(crate) fn fixtures_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("gwcs");
        assert!(dir.is_dir(), "gwcs fixture directory {} is missing", dir.display());
        dir
    }

    pub(crate) fn env_dir(var: &str) -> PathBuf {
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("<{var} unset>")))
    }

    pub(crate) fn truth_dir() -> PathBuf {
        env_dir(TRUTH_DIR_VAR)
    }

    pub(crate) fn real_data_dir() -> PathBuf {
        env_dir(REAL_DATA_DIR_VAR)
    }

    pub(crate) fn mast_dir() -> PathBuf {
        env_dir(MAST_DIR_VAR)
    }

    pub(crate) fn documents_fits_dir() -> PathBuf {
        env_dir(DOCUMENTS_FITS_DIR_VAR)
    }

    pub(crate) fn heavy_test_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("exampleFits").join("sample-data").join("heavyTest")
    }

    pub(crate) fn resolve_arrays(asdf: &AsdfFile, node: &Value) -> Option<(Vec<usize>, Vec<f64>)> {
        AsdfImage::float_values(asdf, node)
    }

    pub(crate) fn open_fixture_file(name: &str) -> AsdfFile {
        let path = fixtures_dir().join(name);
        assert!(path.is_file(), "fixture {} is missing", path.display());
        AsdfFile::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    pub(crate) fn try_fixture_node(name: &str) -> Result<Node, GwcsError> {
        let asdf = open_fixture_file(name);
        let transform = asdf.tree.get("transform").expect("fixture has a 'transform' key");
        parse_node(transform, &|n| resolve_arrays(&asdf, n), "transform")
    }

    pub(crate) fn fixture_node(name: &str) -> Node {
        try_fixture_node(name).unwrap_or_else(|e| panic!("{name}: {e:#}"))
    }

    pub(crate) fn try_fixture_pipeline(name: &str) -> Result<GwcsPipeline, GwcsError> {
        let asdf = open_fixture_file(name);
        let (key, node) = find_wcs_node(&asdf.tree).expect("fixture has a wcs node");
        parse_gwcs(node, &|n| resolve_arrays(&asdf, n), &key.0)
    }

    pub(crate) fn fixture_pipeline(name: &str) -> GwcsPipeline {
        try_fixture_pipeline(name).unwrap_or_else(|e| panic!("{name}: {e:#}"))
    }

    pub(crate) fn expected() -> &'static serde_json::Value {
        static EXPECTED: OnceLock<serde_json::Value> = OnceLock::new();
        EXPECTED.get_or_init(|| {
            let path = fixtures_dir().join("expected.json");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            serde_json::from_str(&text).expect("expected.json parses")
        })
    }

    pub(crate) fn expected_cases(name: &str) -> &'static [serde_json::Value] {
        expected()[name]["cases"].as_array().unwrap_or_else(|| panic!("expected.json has no cases for {name}")).as_slice()
    }

    pub(crate) fn json_f64s(v: &serde_json::Value) -> Vec<f64> {
        v.as_array()
            .unwrap_or_else(|| panic!("{v} is not an array"))
            .iter()
            .map(|x| x.as_f64().unwrap_or(f64::NAN))
            .collect()
    }

    pub(crate) fn json_opt_pair(v: &serde_json::Value) -> [Option<f64>; 2] {
        let a = v.as_array().unwrap_or_else(|| panic!("{v} is not an array"));
        [a[0].as_f64(), a[1].as_f64()]
    }

    #[derive(Debug, Deserialize)]
    pub(crate) struct TruthFile {
        pub bounding_box: Option<[[f64; 2]; 2]>,
        pub frames: Vec<String>,
        pub points: Vec<TruthPoint>,
        pub source_id: String,
    }

    #[derive(Debug, Deserialize)]
    pub(crate) struct TruthPoint {
        pub label: String,
        pub pixel: [f64; 2],
        pub inside_bbox: bool,
        pub world: Vec<Option<f64>>,
        pub world_no_bbox: Vec<Option<f64>>,
        pub frames_forward: BTreeMap<String, Vec<Option<f64>>>,
        pub frames_backward_from_world_no_bbox: Option<BTreeMap<String, Vec<Option<f64>>>>,
        pub invert: Option<Vec<Option<f64>>>,
        pub invert_no_bbox: Option<Vec<Option<f64>>>,
        pub exact_inverse_newton: Option<Vec<Option<f64>>>,
    }

    pub(crate) fn truth_from(path: &Path) -> TruthFile {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    pub(crate) fn truth(name: &str) -> TruthFile {
        truth_from(&fixtures_dir().join("truth").join(format!("{name}.json")))
    }

    pub(crate) fn pair(v: &[Option<f64>]) -> [Option<f64>; 2] {
        [v[0], v[1]]
    }

    pub(crate) fn unwrap_pair(v: &[Option<f64>]) -> [f64; 2] {
        [v[0].unwrap_or(f64::NAN), v[1].unwrap_or(f64::NAN)]
    }

    #[track_caller]
    pub(crate) fn assert_close(got: f64, exp: f64, tol: f64, what: &str) {
        assert!(
            (got - exp).abs() <= tol,
            "{what}: got {got:?}, expected {exp:?} (|diff| {:e} > {tol:e})",
            (got - exp).abs()
        );
    }

    #[track_caller]
    pub(crate) fn assert_close_slice(got: &[f64], exp: &[f64], tol: f64, what: &str) {
        assert_eq!(got.len(), exp.len(), "{what}: lane count {} vs {}", got.len(), exp.len());
        for (i, (g, e)) in got.iter().zip(exp).enumerate() {
            if e.is_nan() {
                assert!(g.is_nan(), "{what}[{i}]: got {g:?}, expected NaN");
            } else {
                assert_close(*g, *e, tol, &format!("{what}[{i}]"));
            }
        }
    }

    #[track_caller]
    pub(crate) fn assert_nan_pattern(got: [f64; 2], exp: [Option<f64>; 2], tol: f64, what: &str) {
        for i in 0..2 {
            match exp[i] {
                None => assert!(got[i].is_nan(), "{what}[{i}]: got {:?}, expected NaN", got[i]),
                Some(e) => {
                    assert!(got[i].is_finite(), "{what}[{i}]: got {:?}, expected {e:?}", got[i]);
                    assert_close(got[i], e, tol, &format!("{what}[{i}]"));
                }
            }
        }
    }

    pub(crate) fn real_syntax(body: &str) -> String {
        format!("#ASDF 1.0.0\n#ASDF_STANDARD 1.6.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n{body}...\n")
    }

    pub(crate) fn parse_real_syntax(body: &str) -> Value {
        parse_tree(&real_syntax(body)).unwrap_or_else(|e| panic!("synthetic YAML does not parse: {e}\n{body}"))
    }

    pub(crate) fn no_arrays(_: &Value) -> Option<(Vec<usize>, Vec<f64>)> {
        None
    }

    pub(crate) fn asdf_from_fits(path: &Path) -> Result<AsdfFile, String> {
        use crate::infra::fits::asdf_hdu::{find_asdf_hdu, read_asdf_hdu_bytes};
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let hdu = find_asdf_hdu(&file)
            .map_err(|e| format!("{}: {e:#}", path.display()))?
            .ok_or_else(|| format!("{}: no ASDF HDU", path.display()))?;
        let bytes = read_asdf_hdu_bytes(&file, &hdu).map_err(|e| format!("{}: {e:#}", path.display()))?;
        AsdfFile::from_bytes(bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn asdf_tree_only(path: &Path) -> Result<AsdfFile, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let end = bytes
            .windows(4)
            .position(|w| w == b"\n...")
            .ok_or_else(|| format!("{}: no YAML document end", path.display()))?;
        let mut text = bytes[..end + 4].to_vec();
        text.push(b'\n');
        AsdfFile::from_bytes(text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn skip_if_absent(path: &Path) -> bool {
        if path.exists() {
            false
        } else {
            eprintln!("skipped: {} absent", path.display());
            true
        }
    }
}
