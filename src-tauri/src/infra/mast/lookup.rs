use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::cache::MemoryLru;
use super::hub::MastHub;
use super::models::null_as_default;
use crate::core::mast::errors::{classify, OpKind, Operation};
use crate::core::mast::query::validate_name;
use crate::core::mast::types::{MastCmdError, MastResolved};

pub const LOOKUP_PATH: &str = "api/v0/invoke";
pub const LOOKUP_SERVICE: &str = "Mast.Name.Lookup";
pub const NAMES_FILE: &str = "names.json";
pub const POSITIVE_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
pub const NEGATIVE_TTL: Duration = Duration::from_secs(3600);
pub const NEGATIVE_ENTRIES: usize = 256;

#[derive(Debug, Default, Deserialize)]
pub struct LookupResponse {
    #[serde(rename = "resolvedCoordinate", default, deserialize_with = "null_as_default")]
    pub resolved: Vec<LookupEntry>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LookupEntry {
    #[serde(rename = "canonicalName", default)]
    pub canonical_name: Option<String>,
    #[serde(default)]
    pub resolver: Option<String>,
    #[serde(default)]
    pub ra: Option<f64>,
    #[serde(default)]
    pub decl: Option<f64>,
    #[serde(rename = "objectType", default)]
    pub object_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredName {
    pub canonical_name: String,
    pub resolver: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub object_type: Option<String>,
    pub stored_at: u64,
}

pub struct NameCache {
    negative: Mutex<MemoryLru<String, ()>>,
    file: Mutex<()>,
}

impl Default for NameCache {
    fn default() -> Self {
        Self { negative: Mutex::new(MemoryLru::new(NEGATIVE_ENTRIES, NEGATIVE_TTL)), file: Mutex::new(()) }
    }
}

pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn normalize_key(name: &str) -> String {
    collapse_whitespace(name).to_lowercase()
}

pub fn lookup_request(name: &str) -> String {
    serde_json::json!({
        "service": LOOKUP_SERVICE,
        "params": {"input": name, "format": "json"},
        "format": "json"
    })
    .to_string()
}

fn non_empty(text: &Option<String>) -> Option<String> {
    text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned)
}

pub fn first_resolved(response: &LookupResponse, now: SystemTime) -> Option<StoredName> {
    response.resolved.iter().find_map(|entry| {
        let ra_deg = entry.ra.filter(|v| v.is_finite())?;
        let dec_deg = entry.decl.filter(|v| v.is_finite())?;
        Some(StoredName {
            canonical_name: entry.canonical_name.as_deref().map(collapse_whitespace).unwrap_or_default(),
            resolver: non_empty(&entry.resolver).unwrap_or_default(),
            ra_deg,
            dec_deg,
            object_type: non_empty(&entry.object_type),
            stored_at: unix_secs(now),
        })
    })
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn fresh(entry: &StoredName, now: SystemTime) -> bool {
    unix_secs(now).saturating_sub(entry.stored_at) <= POSITIVE_TTL.as_secs()
}

fn read_names(root: &Path) -> Option<BTreeMap<String, StoredName>> {
    let bytes = std::fs::read(root.join(NAMES_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl NameCache {
    fn file_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn negative_guard(&self) -> std::sync::MutexGuard<'_, MemoryLru<String, ()>> {
        self.negative.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn read_positive(&self, root: &Path, key: &str, now: SystemTime) -> Option<StoredName> {
        let _guard = self.file_guard();
        let entry = read_names(root)?.remove(key)?;
        fresh(&entry, now).then_some(entry)
    }

    pub fn write_positive(&self, root: &Path, key: &str, entry: &StoredName, now: SystemTime) -> std::io::Result<()> {
        let _guard = self.file_guard();
        std::fs::create_dir_all(root)?;
        let mut entries = read_names(root).unwrap_or_default();
        entries.retain(|_, stored| fresh(stored, now));
        entries.insert(key.to_owned(), entry.clone());
        let bytes = serde_json::to_vec(&entries).map_err(std::io::Error::other)?;
        let temp = root.join(format!("{NAMES_FILE}.{}.tmp", std::process::id()));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, root.join(NAMES_FILE)).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }

    pub fn is_negative(&self, key: &str, now: Instant) -> bool {
        self.negative_guard().get(&key.to_owned(), now).is_some()
    }

    pub fn remember_negative(&self, key: &str, now: Instant) {
        self.negative_guard().insert(key.to_owned(), (), now);
    }

    pub fn clear_negative(&self) {
        self.negative_guard().clear();
    }
}

fn resolved(query: String, stored: StoredName, started: Instant, from_cache: bool) -> MastResolved {
    let canonical_name = if stored.canonical_name.is_empty() { query.clone() } else { stored.canonical_name };
    MastResolved {
        query,
        canonical_name,
        resolver: stored.resolver,
        ra_deg: stored.ra_deg,
        dec_deg: stored.dec_deg,
        object_type: stored.object_type,
        elapsed_ms: started.elapsed().as_millis() as u64,
        from_cache,
    }
}

pub async fn resolve(hub: &MastHub, name: &str) -> Result<Option<MastResolved>, MastCmdError> {
    let started = Instant::now();
    let query = validate_name(name)?;
    let key = normalize_key(&query);
    if let Some(stored) = hub.names().read_positive(hub.cache_root(), &key, SystemTime::now()) {
        return Ok(Some(resolved(query, stored, started, true)));
    }
    if hub.names().is_negative(&key, Instant::now()) {
        return Ok(None);
    }
    let op = Operation::new(None, OpKind::Lookup, LOOKUP_PATH);
    let body = lookup_request(&query);
    let sent = Instant::now();
    let answer: Result<LookupResponse, _> = hub.anon_client(OpKind::Lookup).post_form(LOOKUP_PATH, &[("request", body.as_str())]).await;
    let response = match answer {
        Ok(response) => response,
        Err(error) => {
            let consecutive = hub.limiter().record(&error, &op, sent, Instant::now());
            return Err(classify(&error, &op, consecutive));
        }
    };
    let now = SystemTime::now();
    match first_resolved(&response, now) {
        Some(stored) => {
            if let Err(error) = hub.names().write_positive(hub.cache_root(), &key, &stored, now) {
                log::warn!("MAST name cache was not written: {error}");
            }
            Ok(Some(resolved(query, stored, started, false)))
        }
        None => {
            hub.names().remember_negative(&key, Instant::now());
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn lookup_request_and_parsing_follow_the_portal_contract() {
        let request: Value = serde_json::from_str(&lookup_request("M \"16\"")).unwrap();
        assert_eq!(
            request,
            json!({"service": "Mast.Name.Lookup", "params": {"input": "M \"16\"", "format": "json"}, "format": "json"})
        );
        let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast/lookup_m16.json")).unwrap();
        let response: LookupResponse = serde_json::from_slice(&bytes).unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let stored = first_resolved(&response, now).unwrap();
        assert_eq!(stored.canonical_name, "M 16");
        assert_eq!(stored.resolver, "SIMBAD");
        assert_eq!((stored.ra_deg, stored.dec_deg), (274.688, -13.792));
        assert_eq!(stored.object_type.as_deref(), Some("OpenCluster"));
        assert_eq!(stored.stored_at, 1_800_000_000);
        let miss: LookupResponse = serde_json::from_str(r#"{"resolvedCoordinate":[],"status":""}"#).unwrap();
        assert!(first_resolved(&miss, now).is_none());
        let partial: LookupResponse =
            serde_json::from_str(r#"{"resolvedCoordinate":[{"canonicalName":"X","ra":null},{"canonicalName":"Y","ra":1.5,"decl":2.5,"objectType":""}]}"#).unwrap();
        let second = first_resolved(&partial, now).unwrap();
        assert_eq!(second.canonical_name, "Y");
        assert_eq!(second.object_type, None);
        assert_eq!(second.resolver, "");
        let null: LookupResponse = serde_json::from_str(r#"{"resolvedCoordinate":null}"#).unwrap();
        assert!(null.resolved.is_empty());
    }

    #[test]
    fn lookup_collapses_canonical_whitespace() {
        assert_eq!(collapse_whitespace("M  16"), "M 16");
        assert_eq!(collapse_whitespace("  NGC \t 6611\n"), "NGC 6611");
        assert_eq!(normalize_key("  m   16 "), "m 16");
        assert_eq!(normalize_key("M\t16"), normalize_key("m 16"));
        assert_eq!(validate_name("  Eagle  Nebula ").unwrap(), "Eagle  Nebula");
        for bad in ["", "   ", "M\u{0}16", "a\u{7}"] {
            let error = validate_name(bad).unwrap_err();
            assert!(error.message.starts_with("name: "), "{bad:?}");
        }
        assert!(validate_name(&"n".repeat(crate::core::mast::query::NAME_MAX + 1)).is_err());
    }

    #[test]
    fn names_file_keeps_positive_entries_for_thirty_days() {
        let dir = tempfile::tempdir().unwrap();
        let cache = NameCache::default();
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let entry = StoredName {
            canonical_name: "M 16".into(),
            resolver: "SIMBAD".into(),
            ra_deg: 274.688,
            dec_deg: -13.792,
            object_type: None,
            stored_at: 1_800_000_000,
        };
        assert!(cache.read_positive(dir.path(), "m16", now).is_none());
        cache.write_positive(dir.path(), "m16", &entry, now).unwrap();
        assert_eq!(cache.read_positive(dir.path(), "m16", now + Duration::from_secs(3600)), Some(entry.clone()));
        assert!(cache.read_positive(dir.path(), "m16", now + POSITIVE_TTL + Duration::from_secs(1)).is_none());
        let later = now + POSITIVE_TTL + Duration::from_secs(10);
        let fresh = StoredName { stored_at: 1_800_000_000 + POSITIVE_TTL.as_secs() + 10, canonical_name: "M 42".into(), ..entry };
        cache.write_positive(dir.path(), "m42", &fresh, later).unwrap();
        let file: BTreeMap<String, StoredName> = serde_json::from_slice(&std::fs::read(dir.path().join(NAMES_FILE)).unwrap()).unwrap();
        assert_eq!(file.keys().collect::<Vec<_>>(), vec!["m42"]);
        std::fs::write(dir.path().join(NAMES_FILE), b"{not json").unwrap();
        assert!(cache.read_positive(dir.path(), "m42", later).is_none());
        cache.write_positive(dir.path(), "m42", &fresh, later).unwrap();
        assert!(cache.read_positive(dir.path(), "m42", later).is_some());
        let start = Instant::now();
        assert!(!cache.is_negative("x", start));
        cache.remember_negative("x", start);
        assert!(cache.is_negative("x", start + Duration::from_secs(3599)));
        assert!(!cache.is_negative("x", start + NEGATIVE_TTL + Duration::from_secs(1)));
        cache.remember_negative("y", start);
        cache.clear_negative();
        assert!(!cache.is_negative("y", start));
    }
}
