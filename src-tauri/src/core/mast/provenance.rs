use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROVENANCE: &str = "PROVENANCE.md";
const S_REGION: &str = "s_region.txt";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast")
}

fn provenance() -> String {
    std::fs::read_to_string(fixture_dir().join(PROVENANCE))
        .unwrap_or_else(|error| panic!("tests/fixtures/mast/{PROVENANCE}: {error}"))
}

fn table_rows(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (name, tail) = line.strip_prefix("| `")?.split_once("` |")?;
            Some((name.to_owned(), tail.to_owned()))
        })
        .collect()
}

fn fixture_files() -> BTreeSet<String> {
    std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != PROVENANCE)
        .collect()
}

fn has_date(text: &str) -> bool {
    text.as_bytes().windows(10).any(|window| {
        window.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
    })
}

#[test]
fn every_mast_fixture_has_one_provenance_row_with_its_source_and_capture_date() {
    let text = provenance();
    let rows = table_rows(&text);
    let listed: BTreeSet<String> = rows.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(listed.len(), rows.len(), "a fixture has more than one row");
    assert_eq!(listed, fixture_files());
    for (name, tail) in &rows {
        assert!(has_date(tail), "{name}: the row names no capture date");
        assert!(
            tail.contains("https://") || tail.contains("derived from"),
            "{name}: the row names no MAST URL and no source file"
        );
    }
    assert!(text.contains("public NASA archive metadata"));
}

#[test]
fn the_s_region_lines_come_from_the_captured_searches_or_are_declared_synthetic() {
    let declared = provenance();
    let text = std::fs::read_to_string(fixture_dir().join(S_REGION)).unwrap();
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .collect();
    assert!(!lines.is_empty());
    let mut synthetic = 0;
    for line in lines {
        let mut parts = line.splitn(3, '\t');
        let name = parts.next().unwrap();
        let s_region = parts.nth(1).unwrap();
        let (source, dataset) = name.split_once('/').unwrap();
        let captured = fixture_dir().join(format!("{source}.json"));
        if captured.is_file() {
            let search: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&captured).unwrap()).unwrap();
            let row = search["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["fileSetName"] == dataset)
                .unwrap_or_else(|| panic!("{name}: {dataset} is not a row of {source}.json"));
            assert_eq!(row["s_region"].as_str(), Some(s_region), "{name}");
        } else {
            synthetic += 1;
            assert!(
                declared
                    .lines()
                    .any(|row| row.contains(&format!("`{name}`")) && row.contains("synthetic")),
                "{name} is not a captured row and {PROVENANCE} does not declare it synthetic"
            );
        }
    }
    assert_eq!(synthetic, 1, "only the Roman-form line is synthetic");
}
