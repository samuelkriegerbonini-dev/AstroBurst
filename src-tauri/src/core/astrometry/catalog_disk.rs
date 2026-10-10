use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::astrometry::catalog::{CatalogCacheKey, CatalogRow, ConeQuery};

pub const DISK_CACHE_MAX_BYTES: u64 = 64 << 20;
pub const ENV_CACHE_DIR: &str = "ASTROBURST_CATALOG_CACHE_DIR";

const CACHE_FILE_VERSION: u32 = 1;
const FILE_PREFIX: &str = "gaia-dr3_";
const FILE_EXTENSION: &str = "json";
const TEMP_EXTENSION: &str = "json.tmp";
const MAG_NONE: &str = "none";

static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

#[derive(Debug, Serialize, Deserialize)]
struct DiskFile {
    version: u32,
    fetched_unix_s: u64,
    query: ConeQuery,
    rows: Vec<CatalogRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskEntry {
    pub fetched_unix_s: u64,
    pub rows: Vec<CatalogRow>,
}

#[cfg(not(test))]
fn resolve_default() -> Option<PathBuf> {
    dirs::cache_dir().map(|base| base.join("astroburst").join("catalog-cache"))
}

#[cfg(test)]
fn resolve_default() -> Option<PathBuf> {
    None
}

fn resolve_from_env() -> Option<PathBuf> {
    match std::env::var_os(ENV_CACHE_DIR) {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => resolve_default(),
    }
}

pub fn init_from_env() {
    CACHE_DIR.get_or_init(|| {
        let dir = resolve_from_env()?;
        match fs::create_dir_all(&dir) {
            Ok(()) => Some(dir),
            Err(e) => {
                log::warn!("catalog cache: cannot create {} ({}); the catalog cache stays in memory", dir.display(), e);
                None
            }
        }
    });
}

pub fn cache_dir() -> Option<&'static Path> {
    CACHE_DIR.get().and_then(|dir| dir.as_deref())
}

fn key_part(value: i64) -> String {
    if value < 0 {
        format!("m{}", value.unsigned_abs())
    } else {
        value.to_string()
    }
}

pub fn file_name(key: &CatalogCacheKey) -> String {
    let (ra, dec, radius, mag, rows) = key;
    let mag = mag.map_or_else(|| MAG_NONE.to_string(), key_part);
    format!(
        "{FILE_PREFIX}{}_{}_{}_{}_{}.{FILE_EXTENSION}",
        key_part(*ra),
        key_part(*dec),
        key_part(*radius),
        mag,
        rows
    )
}

fn remove_unreadable(path: &Path, reason: &str) {
    match fs::remove_file(path) {
        Ok(()) => log::warn!("catalog cache: removed unreadable {} ({})", path.display(), reason),
        Err(e) => log::warn!("catalog cache: {} is unreadable ({}) and could not be removed ({})", path.display(), reason, e),
    }
}

pub fn load(dir: &Path, key: &CatalogCacheKey) -> Option<DiskEntry> {
    let path = dir.join(file_name(key));
    let bytes = fs::read(&path).ok()?;
    match serde_json::from_slice::<DiskFile>(&bytes) {
        Ok(file) if file.version == CACHE_FILE_VERSION => Some(DiskEntry {
            fetched_unix_s: file.fetched_unix_s,
            rows: file.rows,
        }),
        Ok(file) => {
            remove_unreadable(&path, &format!("version {}", file.version));
            None
        }
        Err(e) => {
            remove_unreadable(&path, &e.to_string());
            None
        }
    }
}

pub fn store(dir: &Path, key: &CatalogCacheKey, query: &ConeQuery, rows: &[CatalogRow], now_unix_s: u64) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name(key));
    let temp = path.with_extension(TEMP_EXTENSION);
    let file = DiskFile {
        version: CACHE_FILE_VERSION,
        fetched_unix_s: now_unix_s,
        query: query.clone(),
        rows: rows.to_vec(),
    };
    let content = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    if let Err(e) = fs::write(&temp, content).and_then(|()| fs::rename(&temp, &path)) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    log::info!("catalog cache: stored {}", path.display());
    if let Err(e) = evict_to_cap(dir, DISK_CACHE_MAX_BYTES) {
        log::warn!("catalog cache: eviction in {} stopped ({})", dir.display(), e);
    }
    Ok(())
}

type EvictionCandidate = (PathBuf, u64, Option<std::time::SystemTime>);

pub fn evict_to_cap(dir: &Path, cap_bytes: u64) -> std::io::Result<()> {
    let mut files: Vec<EvictionCandidate> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(FILE_EXTENSION) {
            continue;
        }
        if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(FILE_PREFIX)) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        files.push((path, meta.len(), meta.modified().ok()));
    }
    evict_oldest(files, cap_bytes)
}

fn evict_oldest(mut files: Vec<EvictionCandidate>, cap_bytes: u64) -> std::io::Result<()> {
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort_by(|a, b| a.2.cmp(&b.2));
    let mut first_error: Option<std::io::Error> = None;
    for (path, len, _) in files {
        if total <= cap_bytes {
            break;
        }
        match fs::remove_file(&path) {
            Ok(()) => total -= len,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => total -= len,
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::catalog::catalog_cache_key;
    use std::time::{Duration, SystemTime};

    fn row(id: &str, ra: f64, dec: f64) -> CatalogRow {
        CatalogRow {
            id: id.to_string(),
            ra,
            dec,
            ra_epoch: ra,
            dec_epoch: dec,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            g: Some(12.5),
            bp: None,
            rp: None,
            bp_rp: Some(0.8),
            parallax_mas: None,
        }
    }

    fn query(ra: f64, dec: f64) -> ConeQuery {
        ConeQuery {
            ra,
            dec,
            radius_deg: 0.05,
            mag_limit: Some(17.0),
            max_rows: 500,
        }
    }

    fn set_mtime(path: &Path, time: SystemTime) {
        fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn store_then_load_round_trips_rows_and_fetch_time() {
        let dir = tempfile::tempdir().unwrap();
        let q = query(210.0, -20.5);
        let key = catalog_cache_key(&q);
        assert_eq!(file_name(&key), "gaia-dr3_2100000_m205000_500_1700_500.json");
        assert_eq!(
            file_name(&catalog_cache_key(&ConeQuery { mag_limit: None, max_rows: 5000, ..q.clone() })),
            "gaia-dr3_2100000_m205000_500_none_5000.json"
        );
        let rows = vec![row("a", 210.001, -20.499), row("b", 209.999, -20.501)];

        store(dir.path(), &key, &q, &rows, 1_700_000_000).unwrap();

        let path = dir.path().join(file_name(&key));
        assert!(path.is_file(), "{}", path.display());
        assert!(!path.with_extension(TEMP_EXTENSION).exists(), "the temp file was renamed away");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\":1"), "{text}");
        assert!(text.contains("\"fetched_unix_s\":1700000000"), "{text}");
        let entry = load(dir.path(), &key).expect("a stored entry loads");
        assert_eq!(entry.fetched_unix_s, 1_700_000_000);
        assert_eq!(entry.rows, rows);
    }

    #[test]
    fn oldest_files_are_evicted_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut paths = Vec::new();
        for i in 0..3u64 {
            let q = query(211.0 + i as f64 * 0.5, 10.0);
            let key = catalog_cache_key(&q);
            let rows = vec![row("x", q.ra, q.dec), row("y", q.ra, q.dec)];
            store(dir.path(), &key, &q, &rows, 0).unwrap();
            let path = dir.path().join(file_name(&key));
            set_mtime(&path, base + Duration::from_secs(100 * i));
            paths.push(path);
        }
        let n = fs::metadata(&paths[0]).unwrap().len();
        for p in &paths {
            let len = fs::metadata(p).unwrap().len();
            assert!(len.abs_diff(n) <= 2, "entries are about the same size: {len} vs {n}");
        }
        let junk = dir.path().join("gaia-dr3_junk.json");
        fs::write(&junk, vec![b'{'; n as usize]).unwrap();
        set_mtime(&junk, base - Duration::from_secs(100));

        evict_to_cap(dir.path(), (n as f64 * 2.5) as u64).unwrap();

        assert!(!junk.exists(), "the unparseable file with the oldest mtime goes first, without parsing");
        assert!(!paths[0].exists(), "the oldest entry is evicted");
        assert!(paths[1].exists(), "{}", paths[1].display());
        assert!(paths[2].exists(), "{}", paths[2].display());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn foreign_json_files_are_neither_counted_nor_evicted() {
        let dir = tempfile::tempdir().unwrap();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut paths = Vec::new();
        for i in 0..3u64 {
            let q = query(215.0 + i as f64 * 0.5, -5.0);
            let key = catalog_cache_key(&q);
            store(dir.path(), &key, &q, &[row("x", q.ra, q.dec), row("y", q.ra, q.dec)], 0).unwrap();
            let path = dir.path().join(file_name(&key));
            set_mtime(&path, base + Duration::from_secs(100 * i));
            paths.push(path);
        }
        let n = fs::metadata(&paths[0]).unwrap().len();
        let foreign = dir.path().join("my-notes.json");
        fs::write(&foreign, vec![b'x'; (n * 10) as usize]).unwrap();
        set_mtime(&foreign, base - Duration::from_secs(100));

        evict_to_cap(dir.path(), (n as f64 * 2.5) as u64).unwrap();

        assert!(foreign.exists(), "a file without the gaia-dr3_ prefix is left alone");
        assert!(!paths[0].exists(), "the oldest entry is evicted");
        assert!(paths[1].exists() && paths[2].exists(), "the foreign file does not count toward the cap");
    }

    #[test]
    fn a_missing_candidate_does_not_stop_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("gaia-dr3_real.json");
        fs::write(&real, b"{}").unwrap();
        let gone = dir.path().join("gaia-dr3_gone.json");
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let candidates = vec![(gone.clone(), 2, Some(t0)), (real.clone(), 2, Some(t0 + Duration::from_secs(1)))];

        evict_oldest(candidates, 1).unwrap_or_else(|e| panic!("an entry removed by another process is not an error: {e}"));

        assert!(!real.exists(), "eviction continues past the missing file");
        assert!(!gone.exists());
    }

    #[test]
    fn a_failed_rename_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let q = query(217.0, 60.0);
        let key = catalog_cache_key(&q);
        let path = dir.path().join(file_name(&key));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"").unwrap();

        store(dir.path(), &key, &q, &[row("a", q.ra, q.dec)], 1).expect_err("a file cannot replace a non-empty directory");

        assert!(!path.with_extension(TEMP_EXTENSION).exists(), "the temp file is removed after a failed rename");
        assert!(path.is_dir());
    }

    #[test]
    fn a_corrupt_entry_is_a_miss_and_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let q = query(213.0, 44.0);
        let key = catalog_cache_key(&q);
        let path = dir.path().join(file_name(&key));
        fs::write(&path, b"{not json").unwrap();
        assert!(load(dir.path(), &key).is_none());
        assert!(!path.exists(), "the corrupt file is deleted");

        let other = query(213.5, 44.0);
        let other_key = catalog_cache_key(&other);
        let other_path = dir.path().join(file_name(&other_key));
        fs::write(
            &other_path,
            r#"{"version":2,"fetched_unix_s":1,"query":{"ra":213.5,"dec":44.0,"radius_deg":0.05,"mag_limit":17.0,"max_rows":500},"rows":[]}"#,
        )
        .unwrap();
        assert!(load(dir.path(), &other_key).is_none());
        assert!(!other_path.exists(), "a foreign version is a miss and is deleted");

        assert!(load(dir.path(), &catalog_cache_key(&query(214.0, 44.0))).is_none(), "an absent file is a plain miss");
    }

    #[test]
    fn the_default_dir_is_none_under_test() {
        assert!(cache_dir().is_none());
        assert!(resolve_default().is_none());
    }
}
