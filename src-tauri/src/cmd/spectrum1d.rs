use std::fs::File;

use anyhow::Context;

use crate::cmd::common::blocking_cmd;
use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::spectrum1d::{classify, load_spectrum1d};

pub const EMPTY_PATH_ERROR: &str = "path must name a spectrum file, got an empty string";

fn checked_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(EMPTY_PATH_ERROR.to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn read_spectrum1d_cmd(
    path: String,
    hdu: Option<usize>,
) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!({
        let spectrum = load_spectrum1d(&path, hdu)?;
        Ok(serde_json::to_value(&spectrum)?)
    })
}

#[tauri::command]
pub async fn probe_spectrum1d_cmd(path: String) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!({
        let (readable, _decompressed) = resolve_gzip(std::path::Path::new(&path))?;
        let file = File::open(&readable).with_context(|| format!("Failed to open {path}"))?;
        Ok(serde_json::to_value(classify(&file)?)?)
    })
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use std::path::{Path, PathBuf};

    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};

    const X1D_ROW_BYTES: usize = 28;

    pub fn write_x1d_columns(
        dir: &Path,
        name: &str,
        wavelength_um: &[f64],
        flux: &[f64],
        err: &[f64],
        dq: &[i32],
    ) -> PathBuf {
        let rows = wavelength_um.len();
        assert!(
            flux.len() == rows && err.len() == rows && dq.len() == rows,
            "x1d columns of unequal length"
        );
        let mut primary = empty_primary_cards();
        primary.push(("TELESCOP", "'JWST    '".to_string()));
        let table: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", X1D_ROW_BYTES.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "4".into()),
            ("TTYPE1", "'WAVELENGTH'".into()),
            ("TFORM1", "'1D      '".into()),
            ("TUNIT1", "'um      '".into()),
            ("TTYPE2", "'FLUX    '".into()),
            ("TFORM2", "'1D      '".into()),
            ("TUNIT2", "'Jy      '".into()),
            ("TTYPE3", "'FLUX_ERROR'".into()),
            ("TFORM3", "'1D      '".into()),
            ("TUNIT3", "'Jy      '".into()),
            ("TTYPE4", "'DQ      '".into()),
            ("TFORM4", "'1J      '".into()),
            ("EXTNAME", "'EXTRACT1D'".into()),
            ("EXTVER", "1".into()),
        ];
        let mut data = Vec::with_capacity(rows * X1D_ROW_BYTES);
        for row in 0..rows {
            data.extend_from_slice(&wavelength_um[row].to_be_bytes());
            data.extend_from_slice(&flux[row].to_be_bytes());
            data.extend_from_slice(&err[row].to_be_bytes());
            data.extend_from_slice(&dq[row].to_be_bytes());
        }
        let path = dir.join(name);
        write_raw_hdus(&path, &[(primary, Vec::new()), (table, data)]);
        path
    }

    pub fn iraf_linear_cards(
        start_angstrom: f64,
        step_angstrom: f64,
        bunit: &str,
    ) -> Vec<(&'static str, String)> {
        vec![
            ("CTYPE1", "'LINEAR  '".into()),
            ("WAT0_001", "'system=equispec'".into()),
            (
                "WAT1_001",
                "'wtype=linear label=Wavelength units=angstroms'".into(),
            ),
            ("DC-FLAG", "0".into()),
            ("CRVAL1", format!("{start_angstrom:.4}")),
            ("CDELT1", format!("{step_angstrom:.4}")),
            ("CRPIX1", "1.0".into()),
            ("BUNIT", format!("'{bunit}'")),
        ]
    }

    pub fn write_wcs1d(
        dir: &Path,
        name: &str,
        cards: &[(&'static str, String)],
        flux: &[f32],
    ) -> PathBuf {
        let mut primary: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "1".into()),
            ("NAXIS1", flux.len().to_string()),
            ("EXTEND", "T".into()),
        ];
        primary.extend(cards.iter().cloned());
        let data = flux.iter().flat_map(|v| v.to_be_bytes()).collect();
        let path = dir.join(name);
        write_raw_hdus(&path, &[(primary, data)]);
        path
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use crate::core::cube::lazy::test_support::write_line_cube;

    const ROWS: usize = 40;
    const FLAGGED_ROW: usize = 3;
    const IRAF_START_ANGSTROM: f64 = 4000.0;
    const IRAF_STEP_ANGSTROM: f64 = 2.5;
    const IRAF_BUNIT: &str = "erg/cm2/s/A";
    const DOCUMENT_KEYS: [&str; 12] = [
        "axis",
        "cards",
        "dq",
        "dq_table",
        "err",
        "flux",
        "flux_unit",
        "n_rows",
        "notes",
        "precision",
        "source",
        "wavelength_frame",
    ];

    fn ramp(start: f64, step: f64) -> Vec<f64> {
        (0..ROWS).map(|i| start + step * i as f64).collect()
    }

    fn x1d(dir: &tempfile::TempDir) -> String {
        let mut dq = vec![0; ROWS];
        dq[FLAGGED_ROW] = 1;
        let path = write_x1d_columns(
            dir.path(),
            "doc_x1d.fits",
            &ramp(2.0, 0.001),
            &ramp(10.0, 1.0),
            &[0.5; ROWS],
            &dq,
        );
        path.to_str().unwrap().to_string()
    }

    fn iraf(dir: &tempfile::TempDir) -> String {
        let flux: Vec<f32> = (0..ROWS).map(|i| 1.0 + 0.01 * i as f32).collect();
        let cards = iraf_linear_cards(IRAF_START_ANGSTROM, IRAF_STEP_ANGSTROM, IRAF_BUNIT);
        write_wcs1d(dir.path(), "iraf_linear.fits", &cards, &flux)
            .to_str()
            .unwrap()
            .to_string()
    }

    fn cube(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("line_s3d.fits");
        write_line_cube(&path, 0.0);
        path.to_str().unwrap().to_string()
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[tokio::test]
    async fn read_spectrum1d_cmd_returns_the_x1d_document_with_its_errors_and_dq() {
        let dir = tempfile::tempdir().unwrap();
        let value = read_spectrum1d_cmd(x1d(&dir), None).await.unwrap();
        assert_eq!(sorted_keys(&value), DOCUMENT_KEYS);
        assert_eq!(value["source"]["kind"], "x1d");
        assert_eq!(value["source"]["hdu"], 1);
        assert_eq!(value["wavelength_frame"], "vacuum");
        assert_eq!(value["precision"], "f64");
        assert_eq!(value["flux_unit"], "Jy");
        assert_eq!(value["dq_table"], "jwst");
        assert_eq!(value["n_rows"], ROWS);
        assert_eq!(value["axis"]["unit"], "um");
        let expected = ramp(2.0, 0.001);
        let axis: Vec<f64> = value["axis"]["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(axis, expected);
        assert_eq!(value["flux"][7], 17.0);
        assert_eq!(value["err"][0], 0.5);
        assert_eq!(value["dq"][FLAGGED_ROW], 1);
        assert_eq!(value["dq"][FLAGGED_ROW + 1], 0);
    }

    #[tokio::test]
    async fn read_spectrum1d_cmd_opens_an_iraf_linear_array_as_angstrom_air() {
        let dir = tempfile::tempdir().unwrap();
        let path = iraf(&dir);
        let value = read_spectrum1d_cmd(path.clone(), Some(0)).await.unwrap();
        assert_eq!(sorted_keys(&value), DOCUMENT_KEYS);
        assert_eq!(value["source"]["kind"], "wcs1d");
        assert_eq!(value["source"]["hdu"], 0);
        assert_eq!(value["wavelength_frame"], "air");
        assert_eq!(value["flux_unit"], IRAF_BUNIT);
        assert_eq!(value["axis"]["unit"], "um");
        let first = value["axis"]["values"][0].as_f64().unwrap();
        assert!(
            (first - IRAF_START_ANGSTROM * 1e-4).abs() < 1e-12,
            "first wavelength {first}"
        );
        assert!(value["err"].is_null() && value["dq"].is_null());
        assert_eq!(
            read_spectrum1d_cmd(path, None).await.unwrap()["source"],
            value["source"]
        );
    }

    #[tokio::test]
    async fn probe_spectrum1d_cmd_names_the_kind_or_returns_null_for_a_cube() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            probe_spectrum1d_cmd(x1d(&dir)).await.unwrap(),
            serde_json::json!({ "kind": "x1d", "hdu": 1 })
        );
        assert_eq!(
            probe_spectrum1d_cmd(iraf(&dir)).await.unwrap(),
            serde_json::json!({ "kind": "wcs1d", "hdu": 0 })
        );
        assert!(probe_spectrum1d_cmd(cube(&dir)).await.unwrap().is_null());
        let empty = probe_spectrum1d_cmd("  ".to_string()).await.unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
        let missing = dir
            .path()
            .join("missing.fits")
            .to_str()
            .unwrap()
            .to_string();
        let err = probe_spectrum1d_cmd(missing).await.unwrap_err();
        assert!(err.contains("Failed to open"), "{err}");
    }

    #[tokio::test]
    async fn a_gzip_compressed_x1d_is_probed_and_read_under_its_own_path() {
        use crate::infra::fits::gzip::test_support::gzip_copy;
        let dir = tempfile::tempdir().unwrap();
        let plain = x1d(&dir);
        let gz = gzip_copy(std::path::Path::new(&plain), "doc_x1d.fits.gz")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            probe_spectrum1d_cmd(gz.clone()).await.unwrap(),
            serde_json::json!({ "kind": "x1d", "hdu": 1 })
        );
        let value = read_spectrum1d_cmd(gz.clone(), None).await.unwrap();
        assert_eq!(value["source"]["path"], gz.as_str());
        assert_eq!(
            value["flux"],
            read_spectrum1d_cmd(plain, None).await.unwrap()["flux"]
        );
    }

    #[tokio::test]
    async fn read_spectrum1d_cmd_refuses_a_cube_and_an_empty_path_with_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_spectrum1d_cmd(cube(&dir), None).await.unwrap_err();
        assert!(err.contains("holds no 1D spectrum"), "{err}");
        let empty = read_spectrum1d_cmd(String::new(), None).await.unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
    }
}
