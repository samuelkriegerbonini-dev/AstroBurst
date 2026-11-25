use std::fs::File;

use anyhow::Context;

use crate::cmd::common::blocking_cmd;
use crate::core::analysis::spectrum1d::WavelengthUnitChoice;
use crate::core::analysis::x1d::describe_extract1d_tables;
use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::spectrum1d::{classify, load_spectrum1d_with_unit, read_spectrum_csv};

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
    wavelength_unit: Option<String>,
) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    let unit = wavelength_unit
        .as_deref()
        .map(WavelengthUnitChoice::parse)
        .transpose()?;
    blocking_cmd!({
        let spectrum = load_spectrum1d_with_unit(&path, hdu, unit)?;
        Ok(serde_json::to_value(&spectrum)?)
    })
}

#[tauri::command]
pub async fn read_spectrum_csv_cmd(path: String) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!({
        let spectrum = read_spectrum_csv(&path)?;
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

#[tauri::command]
pub async fn spectrum1d_tables_cmd(path: String) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!({
        let tables = describe_extract1d_tables(&path)?;
        Ok(serde_json::json!({ "tables": tables }))
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
    use crate::core::analysis::x1d::test_fixtures::write_x1d_fixture;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::cube::lazy::test_support::write_line_cube;

    const ROWS: usize = 40;
    const FLAGGED_ROW: usize = 3;
    const IRAF_START_ANGSTROM: f64 = 4000.0;
    const IRAF_STEP_ANGSTROM: f64 = 2.5;
    const IRAF_BUNIT: &str = "erg/cm2/s/A";
    const DOCUMENT_KEYS: [&str; 13] = [
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
        "wavelength_unit_source",
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
        let value = read_spectrum1d_cmd(x1d(&dir), None, None).await.unwrap();
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
        let value = read_spectrum1d_cmd(path.clone(), Some(0), None).await.unwrap();
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
            read_spectrum1d_cmd(path, None, None).await.unwrap()["source"],
            value["source"]
        );
    }

    #[tokio::test]
    async fn probe_spectrum1d_cmd_names_the_kind_or_returns_null_for_a_cube() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            probe_spectrum1d_cmd(x1d(&dir)).await.unwrap(),
            serde_json::json!({ "kind": "x1d", "hdu": 1, "n_tables": 1 })
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
            serde_json::json!({ "kind": "x1d", "hdu": 1, "n_tables": 1 })
        );
        let value = read_spectrum1d_cmd(gz.clone(), None, None).await.unwrap();
        assert_eq!(value["source"]["path"], gz.as_str());
        assert_eq!(
            value["flux"],
            read_spectrum1d_cmd(plain, None, None).await.unwrap()["flux"]
        );
    }

    fn exported_csv(dir: &tempfile::TempDir) -> String {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("spectrum1d")
            .join("document_export.csv");
        let path = dir.path().join("jw_test_x1d_spectrum.csv");
        std::fs::copy(fixture, &path).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn read_spectrum_csv_cmd_returns_the_csv_document_with_its_errors_and_dq() {
        let dir = tempfile::tempdir().unwrap();
        let path = exported_csv(&dir);
        let value = read_spectrum_csv_cmd(path.clone()).await.unwrap();
        assert_eq!(sorted_keys(&value), DOCUMENT_KEYS);
        assert_eq!(
            value["source"],
            serde_json::json!({ "kind": "csv", "path": path })
        );
        assert_eq!(value["wavelength_frame"], "vacuum");
        assert_eq!(value["flux_unit"], "Jy");
        assert_eq!(value["dq_table"], "jwst");
        assert_eq!(value["n_rows"], 12);
        assert_eq!(value["axis"]["values"][0], 1.8);
        assert!(value["flux"][5].is_null() && value["err"][7].is_null());
        assert_eq!(value["dq"][9], 2_147_483_648_u64);
        assert_eq!(
            read_spectrum1d_cmd(path, None, None).await.unwrap()["source"],
            value["source"]
        );
    }

    #[tokio::test]
    async fn read_spectrum_csv_cmd_refuses_an_empty_path_and_names_an_unreadable_csv() {
        let empty = read_spectrum_csv_cmd(" ".to_string()).await.unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_axis.csv");
        std::fs::write(&path, "channel,flux\n0,1\n").unwrap();
        let err = read_spectrum_csv_cmd(path.to_str().unwrap().to_string())
            .await
            .unwrap_err();
        assert!(err.contains("wavelength_vacuum_um"), "{err}");
    }

    #[tokio::test]
    async fn read_spectrum1d_cmd_refuses_a_cube_and_an_empty_path_with_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_spectrum1d_cmd(cube(&dir), None, None).await.unwrap_err();
        assert!(err.contains("holds no 1D spectrum"), "{err}");
        let empty = read_spectrum1d_cmd(String::new(), None, None).await.unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
    }

    #[tokio::test]
    async fn read_spectrum1d_cmd_applies_a_stated_unit_and_refuses_an_unknown_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let cards: Vec<(&'static str, String)> = vec![
            ("CRVAL1", format!("{IRAF_START_ANGSTROM:.4}")),
            ("CDELT1", format!("{IRAF_STEP_ANGSTROM:.4}")),
            ("CRPIX1", "1.0".into()),
        ];
        let flux: Vec<f32> = (0..ROWS).map(|i| 1.0 + 0.01 * i as f32).collect();
        let path = write_wcs1d(dir.path(), "bare_linear.fits", &cards, &flux)
            .to_str()
            .unwrap()
            .to_string();

        let value = read_spectrum1d_cmd(path.clone(), Some(0), Some("Angstrom".into()))
            .await
            .unwrap();
        assert_eq!(sorted_keys(&value), DOCUMENT_KEYS);
        assert_eq!(value["wavelength_unit_source"], "user");
        let first = value["axis"]["values"][0].as_f64().unwrap();
        assert!((first - 0.4).abs() < 1e-12, "first wavelength {first} um");

        let unknown = read_spectrum1d_cmd(path.clone(), Some(0), Some("microns".into()))
            .await
            .unwrap_err();
        assert_eq!(
            unknown,
            "wavelengthUnit must be one of um, nm, Angstrom; got 'microns'"
        );

        let unitless = read_spectrum1d_cmd(path, Some(0), None).await.unwrap_err();
        assert!(
            unitless.contains("the array has no wavelength solution"),
            "{unitless}"
        );
    }

    const TABLE_KEYS: [&str; 12] = [
        "detector",
        "extver",
        "hdu",
        "int_num",
        "label",
        "n_channels",
        "n_rows",
        "slit_name",
        "source_id",
        "source_name",
        "sporder",
        "srctype",
    ];

    #[tokio::test]
    async fn spectrum1d_tables_cmd_lists_every_extract1d_table_with_its_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "mos_x1d.fits", 5, &[2, 1], "um");
        let value = spectrum1d_tables_cmd(path.to_str().unwrap().to_string())
            .await
            .unwrap();
        assert_eq!(sorted_keys(&value), ["tables"]);
        let tables = value["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(sorted_keys(&tables[0]), TABLE_KEYS);
        assert_eq!(
            tables[0],
            serde_json::json!({
                "hdu": 1,
                "extver": 2,
                "n_rows": 5,
                "n_channels": 5,
                "sporder": null,
                "detector": "NRS1",
                "slit_name": null,
                "source_id": null,
                "source_name": null,
                "int_num": null,
                "srctype": "POINT",
                "label": "table 1 of 2 (EXTVER 2)"
            })
        );
        assert_eq!(tables[1]["hdu"], 2);
        assert_eq!(tables[1]["label"], "table 2 of 2 (EXTVER 1)");

        let empty = spectrum1d_tables_cmd(" ".to_string()).await.unwrap_err();
        assert_eq!(empty, EMPTY_PATH_ERROR);
        let err = spectrum1d_tables_cmd(cube(&dir)).await.unwrap_err();
        assert!(err.contains("no EXTRACT1D table"), "{err}");
    }

    fn tso_file(relative: &str) -> String {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR")
            .join("tso")
            .join(relative);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    fn column(listing: &serde_json::Value, key: &str) -> Vec<serde_json::Value> {
        listing["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| table[key].clone())
            .collect()
    }

    fn series_json(
        layout: &str,
        n_series: usize,
        n_integrations: usize,
        n_channels: usize,
    ) -> serde_json::Value {
        serde_json::json!({
            "kind": "x1dints",
            "layout": layout,
            "n_series": n_series,
            "n_integrations": n_integrations,
            "n_channels": n_channels
        })
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/B1, tso/A3, tso/A5"]
    async fn real_x1dints_sets_probe_as_time_series_and_list_their_tables() {
        let prism =
            tso_file("E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits");
        let legacy = tso_file("B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits");
        let soss = tso_file("A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits");
        let gratings =
            tso_file("A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits");

        assert_eq!(
            probe_spectrum1d_cmd(prism.clone()).await.unwrap(),
            series_json("rows", 1, 240, 431),
            "plan/goldens/tso/x1dints_jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048.json"
        );
        assert_eq!(
            probe_spectrum1d_cmd(legacy.clone()).await.unwrap(),
            series_json("extensions", 1, 10, 388),
            "plan/goldens/tso/x1dints_jw01274-o013_t008_miri_p750l-slitlessprism.json"
        );
        assert_eq!(
            probe_spectrum1d_cmd(soss.clone()).await.unwrap(),
            series_json("rows", 3, 5, 2048),
            "plan/goldens/tso/x1dints_jw07073-o004_t001_niriss_clear-gr700xd-substrip256.json"
        );

        let refusal = read_spectrum1d_cmd(prism, None, None).await.unwrap_err();
        assert!(
            refusal
                .contains("holds time-series spectra (240 integrations x 431 channels, 1 series)"),
            "{refusal}"
        );

        let third = read_spectrum1d_cmd(legacy.clone(), Some(4), None).await.unwrap();
        assert_eq!(third["source"]["kind"], "x1d");
        assert_eq!(third["source"]["extver"], 3);
        assert_eq!(third["n_rows"], 388);
        assert_eq!(third["axis"]["values"][0], 13.993632316589355);
        assert_eq!(third["flux_unit"], "Jy");
        let notes = third["notes"].as_array().unwrap();
        assert!(
            notes.iter().any(|note| note
                == "this file holds 10 EXTRACT1D tables; shown: integration 3 of 10"),
            "{notes:?}"
        );

        let integrations = spectrum1d_tables_cmd(legacy).await.unwrap();
        assert_eq!(
            column(&integrations, "hdu"),
            (2..=11).collect::<Vec<i64>>()
        );
        assert_eq!(
            column(&integrations, "label"),
            (1..=10)
                .map(|n| format!("integration {n} of 10"))
                .collect::<Vec<_>>()
        );
        assert_eq!(column(&integrations, "detector"), ["MIRIMAGE"; 10]);
        assert_eq!(column(&integrations, "sporder"), [1; 10]);
        assert_eq!(column(&integrations, "n_rows"), [388; 10]);
        assert_eq!(column(&integrations, "n_channels"), [388; 10]);

        let orders = spectrum1d_tables_cmd(soss).await.unwrap();
        assert_eq!(column(&orders, "label"), ["order 1", "order 2", "order 3"]);
        assert_eq!(column(&orders, "n_rows"), [5; 3]);
        assert_eq!(column(&orders, "n_channels"), [2048, 1783, 1134]);
        assert_eq!(column(&orders, "detector"), ["NIS"; 3]);

        let detectors = spectrum1d_tables_cmd(gratings).await.unwrap();
        assert_eq!(column(&detectors, "label"), ["NRS1", "NRS2"]);
        assert_eq!(column(&detectors, "sporder"), [-1; 2]);
        assert_eq!(column(&detectors, "n_channels"), [1271, 2048]);
    }
}
