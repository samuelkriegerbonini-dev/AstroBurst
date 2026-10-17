use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::settings::MastSettings;
use crate::core::mast::errors::bad_input;
use crate::core::mast::ids::{validate_dataset_id, validate_filename};
use crate::core::mast::jpeg::ImageKind;
use crate::core::mast::types::{MastCacheInfo, MastCmdError, MastMissionId};

pub const SEARCH_CACHE_ENTRIES: usize = 32;
pub const SEARCH_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
pub const IMAGE_SET_ENTRIES: usize = 500;
pub const PRODUCT_LIST_ENTRIES: usize = 64;
pub const LIST_TTL: Duration = Duration::from_secs(24 * 3600);
pub const IMAGE_CAP_BYTES: u64 = 1024 * 1024 * 1024;
pub const LIST_CAP_BYTES: u64 = 50 * 1024 * 1024;
pub const IMG_DIR: &str = "img";
pub const LISTS_DIR: &str = "lists";
pub const REPRESENTATIVE_STEM: &str = "_representative";
pub const REPRESENTATIVE_NAME: &str = "_representative.name";
pub const PART_EXTENSION: &str = "part";
pub const TEMP_EXTENSION: &str = "tmp";

struct Slot<K, V> {
    key: K,
    value: V,
    stored: Instant,
    used: u64,
}

pub struct MemoryLru<K, V> {
    capacity: usize,
    ttl: Duration,
    slots: Vec<Slot<K, V>>,
    tick: u64,
}

impl<K: PartialEq, V: Clone> MemoryLru<K, V> {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self { capacity, ttl, slots: Vec::new(), tick: 0 }
    }

    fn expired(&self, stored: Instant, now: Instant) -> bool {
        now.saturating_duration_since(stored) >= self.ttl
    }

    pub fn get(&mut self, key: &K, now: Instant) -> Option<V> {
        let index = self.slots.iter().position(|slot| slot.key == *key)?;
        if self.expired(self.slots[index].stored, now) {
            self.slots.swap_remove(index);
            return None;
        }
        self.tick += 1;
        self.slots[index].used = self.tick;
        Some(self.slots[index].value.clone())
    }

    pub fn insert(&mut self, key: K, value: V, now: Instant) {
        if self.capacity == 0 {
            return;
        }
        self.tick += 1;
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.key == key) {
            slot.value = value;
            slot.stored = now;
            slot.used = self.tick;
            return;
        }
        let ttl = self.ttl;
        self.slots.retain(|slot| now.saturating_duration_since(slot.stored) < ttl);
        if self.slots.len() >= self.capacity {
            if let Some(oldest) = self.slots.iter().enumerate().min_by_key(|(_, slot)| slot.used).map(|(i, _)| i) {
                self.slots.swap_remove(oldest);
            }
        }
        self.slots.push(Slot { key, value, stored: now, used: self.tick });
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Images,
    Products,
}

impl ListKind {
    const fn suffix(self) -> &'static str {
        match self {
            ListKind::Images => "images",
            ListKind::Products => "products",
        }
    }
}

const WINDOWS_DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn windows_device(value: &str) -> bool {
    let base = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    WINDOWS_DEVICES.contains(&base.as_str())
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

fn safe_component<'a>(field: &str, value: &'a str, validate: fn(&str) -> Result<(), MastCmdError>) -> Result<&'a str, MastCmdError> {
    validate(value)?;
    if value.ends_with('.') || windows_device(value) {
        return Err(bad_input(field, "is not a usable file name"));
    }
    Ok(value)
}

pub fn safe_filename(value: &str) -> Result<&str, MastCmdError> {
    safe_component("filename", value, validate_filename)
}

pub fn cache_root(local_data: &Path, settings: &MastSettings) -> PathBuf {
    local_data.join(settings.cache_dir_name())
}

pub fn dataset_dir(root: &Path, mission: MastMissionId, dataset: &str) -> Result<PathBuf, MastCmdError> {
    let dataset = safe_component("datasetId", dataset, validate_dataset_id)?;
    Ok(root.join(IMG_DIR).join(mission.slug()).join(dataset))
}

pub fn list_file(root: &Path, mission: MastMissionId, dataset: &str, kind: ListKind) -> Result<PathBuf, MastCmdError> {
    let dataset = safe_component("datasetId", dataset, validate_dataset_id)?;
    Ok(root.join(LISTS_DIR).join(mission.slug()).join(format!("{dataset}.{}.json", kind.suffix())))
}

pub fn stored_name(filename: &str, kind: ImageKind) -> String {
    let extension = format!(".{}", kind.extension());
    if filename.ends_with(&extension) {
        filename.to_owned()
    } else {
        format!("{filename}{extension}")
    }
}

pub fn cached_image(dir: &Path, filename: &str) -> Option<PathBuf> {
    [ImageKind::Jpeg, ImageKind::Png]
        .into_iter()
        .map(|kind| dir.join(stored_name(filename, kind)))
        .find(|path| path.is_file())
}

pub fn representative_file(dir: &Path) -> Option<PathBuf> {
    cached_image(dir, REPRESENTATIVE_STEM)
}

pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

#[derive(Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored<T> {
    stored_at: u64,
    value: T,
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn read_stored_at<T: DeserializeOwned>(path: &Path, ttl: Duration, now: SystemTime) -> Option<(T, SystemTime)> {
    let bytes = std::fs::read(path).ok()?;
    let stored: Stored<T> = serde_json::from_slice(&bytes).ok()?;
    let age = unix_secs(now).checked_sub(stored.stored_at)?;
    (age <= ttl.as_secs()).then(|| (stored.value, UNIX_EPOCH + Duration::from_secs(stored.stored_at)))
}

pub fn read_stored<T: DeserializeOwned>(path: &Path, ttl: Duration, now: SystemTime) -> Option<T> {
    read_stored_at(path, ttl, now).map(|(value, _)| value)
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<u64> {
    let parent = path.parent().ok_or_else(|| io::Error::other("cache path has no folder"))?;
    std::fs::create_dir_all(parent)?;
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = parent.join(format!("{name}.{}-{n}.{PART_EXTENSION}", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })?;
    Ok(bytes.len() as u64)
}

pub fn write_stored<T: Serialize>(path: &Path, value: &T, now: SystemTime) -> io::Result<u64> {
    let bytes = serde_json::to_vec(&Stored { stored_at: unix_secs(now), value }).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

struct CachedFile {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

fn is_part(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == PART_EXTENSION)
}

fn walk_files(dir: &Path, out: &mut Vec<CachedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            walk_files(&path, out);
        } else if kind.is_file() {
            let Ok(meta) = entry.metadata() else { continue };
            out.push(CachedFile { path, len: meta.len(), modified: meta.modified().ok() });
        }
    }
}

fn files_in(dir: &Path) -> Vec<CachedFile> {
    let mut out = Vec::new();
    walk_files(dir, &mut out);
    out
}

pub fn dir_info(dir: &Path) -> MastCacheInfo {
    files_in(dir).iter().fold(MastCacheInfo::default(), |info, file| MastCacheInfo {
        bytes: info.bytes + file.len,
        files: info.files + 1,
    })
}

pub fn evict_to_cap(dir: &Path, cap: u64, target: u64) -> io::Result<u64> {
    let (parts, mut files): (Vec<CachedFile>, Vec<CachedFile>) = files_in(dir).into_iter().partition(|file| is_part(&file.path));
    let mut total: u64 = files.iter().chain(&parts).map(|file| file.len).sum();
    if total <= cap {
        return Ok(total);
    }
    files.sort_by_key(|file| file.modified);
    let mut first_error: Option<io::Error> = None;
    for file in files {
        if total <= target {
            break;
        }
        match std::fs::remove_file(&file.path) {
            Ok(()) => total -= file.len,
            Err(error) if error.kind() == io::ErrorKind::NotFound => total -= file.len,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(total), Err)
}

fn temp_owner(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".part").or_else(|| name.strip_suffix(".tmp"))?;
    let tail = match stem.strip_prefix("dl-") {
        Some(rest) => rest,
        None => stem.rsplit_once('.').map_or(stem, |(_, tail)| tail),
    };
    tail.split('-').next()?.parse().ok()
}

fn is_temp(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == PART_EXTENSION || ext == TEMP_EXTENSION)
}

pub fn sweep_stale_temps(root: &Path, pid: u32, now: SystemTime, max_age: Duration) -> u64 {
    let mut freed = 0;
    for file in files_in(root).into_iter().filter(|file| is_temp(&file.path)) {
        let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let recent = file.modified.is_some_and(|modified| now.duration_since(modified).map_or(true, |age| age <= max_age));
        if temp_owner(&name) == Some(pid) && recent {
            continue;
        }
        if std::fs::remove_file(&file.path).is_ok() {
            freed += file.len;
        }
    }
    freed
}

pub fn remove_tree(dir: &Path) -> (MastCacheInfo, Option<io::Error>) {
    let mut freed = MastCacheInfo::default();
    let mut first_error: Option<io::Error> = None;
    for file in files_in(dir) {
        match std::fs::remove_file(&file.path) {
            Ok(()) => {
                freed.bytes += file.len;
                freed.files += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if first_error.is_none() {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => first_error = Some(error),
        }
    }
    (freed, first_error)
}

pub fn remove_file(path: &Path) -> io::Result<MastCacheInfo> {
    let len = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MastCacheInfo::default()),
        Err(error) => return Err(error),
    };
    match std::fs::remove_file(path) {
        Ok(()) => Ok(MastCacheInfo { bytes: len, files: 1 }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(MastCacheInfo::default()),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Default)]
struct BudgetState {
    used: Option<u64>,
    evicting: bool,
    pending: u64,
}

#[derive(Debug)]
pub struct DiskBudget {
    cap: u64,
    state: Mutex<BudgetState>,
}

impl DiskBudget {
    pub fn new(cap: u64) -> Self {
        Self { cap, state: Mutex::new(BudgetState::default()) }
    }

    fn state(&self) -> MutexGuard<'_, BudgetState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn target(&self) -> u64 {
        self.cap / 10 * 9 + self.cap % 10 * 9 / 10
    }

    pub fn note_write(&self, bytes: u64) -> bool {
        let mut state = self.state();
        if state.evicting {
            state.pending += bytes;
            return false;
        }
        let over = match state.used {
            None => true,
            Some(used) => {
                let used = used + bytes;
                state.used = Some(used);
                used > self.cap
            }
        };
        if over {
            state.evicting = true;
            state.pending = 0;
        }
        over
    }

    pub fn begin(&self) -> bool {
        let mut state = self.state();
        if state.evicting {
            return false;
        }
        state.evicting = true;
        state.pending = 0;
        true
    }

    pub fn finish(&self, remaining: u64) {
        let mut state = self.state();
        state.used = Some(remaining + state.pending);
        state.pending = 0;
        state.evicting = false;
    }

    pub fn reset(&self, remaining: u64) {
        let mut state = self.state();
        state.used = Some(remaining);
        state.pending = 0;
    }

    pub fn used(&self) -> Option<u64> {
        self.state().used
    }

    pub fn busy(&self) -> bool {
        self.state().evicting
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::MastErrorCode;
    use crate::infra::mast::settings::{ENV_MAST_URL, CACHE_DIR, TEST_CACHE_DIR};

    fn write(path: &Path, bytes: usize, age_secs: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; bytes]).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age_secs)).unwrap();
    }

    #[test]
    fn memory_lru_evicts_least_recent_and_expires() {
        let start = Instant::now();
        let mut cache = MemoryLru::new(2, Duration::from_secs(60));
        assert!(cache.is_empty());
        cache.insert("a", 1, start);
        cache.insert("b", 2, start);
        assert_eq!(cache.get(&"a", start), Some(1));
        cache.insert("c", 3, start);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"b", start), None);
        assert_eq!(cache.get(&"a", start), Some(1));
        assert_eq!(cache.get(&"c", start), Some(3));
        cache.insert("a", 10, start + Duration::from_secs(30));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"a", start + Duration::from_secs(80)), Some(10));
        assert_eq!(cache.get(&"c", start + Duration::from_secs(61)), None);
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
        let mut zero = MemoryLru::new(0, Duration::from_secs(60));
        zero.insert("x", 1, start);
        assert_eq!(zero.get(&"x", start), None);
    }

    #[test]
    fn test_cache_root_when_url_overridden() {
        let base = Path::new("C:/Users/u/AppData/Local/com.astroburst.desktop");
        assert_eq!(cache_root(base, &MastSettings::default()), base.join(CACHE_DIR));
        let canned = MastSettings::from_lookup(|name| (name == ENV_MAST_URL).then(|| "http://127.0.0.1:43123".to_owned()));
        assert_eq!(cache_root(base, &canned), base.join(TEST_CACHE_DIR));
        let ignored = MastSettings::from_lookup(|name| (name == ENV_MAST_URL).then(|| "http://evil.example".to_owned()));
        assert_eq!(cache_root(base, &ignored), base.join(CACHE_DIR));
        let root = Path::new("R");
        assert_eq!(dataset_dir(root, MastMissionId::Jwst, "jw02739-o001_t001_nircam").unwrap(), root.join("img").join("jwst").join("jw02739-o001_t001_nircam"));
        assert_eq!(
            list_file(root, MastMissionId::Hst, "ICK909030", ListKind::Images).unwrap(),
            root.join("lists").join("hst").join("ICK909030.images.json")
        );
        assert_eq!(
            list_file(root, MastMissionId::RomanCgi, "r1", ListKind::Products).unwrap(),
            root.join("lists").join("roman_cgi").join("r1.products.json")
        );
        for bad in ["../x", "a/b", "CON", "nul", "Com1.txt", "lpt9", "a.", ""] {
            let error = dataset_dir(root, MastMissionId::Jwst, bad).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad}");
            assert!(error.message.starts_with("datasetId: "), "{bad}: {}", error.message);
            assert!(list_file(root, MastMissionId::Jwst, bad, ListKind::Images).is_err(), "{bad}");
        }
        assert!(dataset_dir(root, MastMissionId::Jwst, "CONSOLE").is_ok());
    }

    #[test]
    fn stored_extension_follows_sniffed_type() {
        assert_eq!(stored_name("x_thumb.jpg", ImageKind::Jpeg), "x_thumb.jpg");
        assert_eq!(stored_name("x_thumb.jpg", ImageKind::Png), "x_thumb.jpg.png");
        assert_eq!(stored_name("x.png", ImageKind::Png), "x.png");
        assert_eq!(stored_name("x.fits", ImageKind::Jpeg), "x.fits.jpg");
        assert_eq!(stored_name("X.JPG", ImageKind::Jpeg), "X.JPG.jpg");
        assert_eq!(stored_name(REPRESENTATIVE_STEM, ImageKind::Png), "_representative.png");
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cached_image(dir.path(), "x_thumb.jpg"), None);
        write(&dir.path().join("x_thumb.jpg.png"), 10, 0);
        assert_eq!(cached_image(dir.path(), "x_thumb.jpg"), Some(dir.path().join("x_thumb.jpg.png")));
        write(&dir.path().join("x_thumb.jpg"), 10, 0);
        assert_eq!(cached_image(dir.path(), "x_thumb.jpg"), Some(dir.path().join("x_thumb.jpg")));
        write(&dir.path().join("y.fits.jpg"), 10, 0);
        assert_eq!(cached_image(dir.path(), "y.fits"), Some(dir.path().join("y.fits.jpg")));
        assert_eq!(representative_file(dir.path()), None);
        write(&dir.path().join("_representative.png"), 10, 0);
        assert_eq!(representative_file(dir.path()), Some(dir.path().join("_representative.png")));
        write(&dir.path().join("z.jpg.1.part"), 10, 0);
        assert_eq!(cached_image(dir.path(), "z.jpg"), None);
    }

    #[test]
    fn cache_eviction_respects_cap_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let oldest = root.join("jwst").join("a").join("x1.jpg");
        let middle = root.join("jwst").join("a").join("x2.jpg");
        let newest = root.join("hst").join("b").join("x3.jpg");
        let part = root.join("hst").join("b").join("dl-1-0.part");
        write(&oldest, 100, 300);
        write(&middle, 100, 200);
        write(&newest, 100, 100);
        write(&part, 50, 900);
        assert_eq!(evict_to_cap(root, 350, 320).unwrap(), 350);
        assert!(oldest.exists() && middle.exists() && newest.exists());
        touch(&oldest);
        assert_eq!(evict_to_cap(root, 300, 250).unwrap(), 250, "part files count toward the cap");
        assert!(oldest.exists(), "touched file was evicted");
        assert!(!middle.exists(), "least recently used file survived");
        assert!(newest.exists());
        assert!(part.exists(), "an in-flight part file was evicted");
        assert_eq!(evict_to_cap(root, 50, 0).unwrap(), 50);
        assert!(!oldest.exists() && !newest.exists());
        assert!(part.exists());
        assert_eq!(evict_to_cap(&root.join("missing"), 10, 0).unwrap(), 0);
    }

    #[test]
    fn stale_temp_files_of_other_processes_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pid = std::process::id();
        let foreign = u32::MAX - 1;
        let images = root.join("img").join("jwst").join("jw1");
        let stale = [
            images.join(format!("dl-{foreign}-0.part")),
            images.join(format!("x_thumb.jpg.{foreign}-3.part")),
            root.join("lists").join("jwst").join(format!("jw1.images.json.{foreign}-1.part")),
            root.join(format!("names.json.{foreign}.tmp")),
            images.join(format!("dl-{pid}-7.part")),
            images.join("odd.part"),
        ];
        let kept = [images.join(format!("dl-{pid}-8.part")), root.join(format!("names.json.{pid}.tmp")), images.join("x_thumb.jpg")];
        for path in &stale[..4] {
            write(path, 40, 0);
        }
        write(&stale[4], 40, 900);
        write(&stale[5], 40, 0);
        for path in &kept {
            write(path, 40, 0);
        }
        assert_eq!(sweep_stale_temps(root, pid, SystemTime::now(), Duration::from_secs(120)), 6 * 40);
        for path in &stale {
            assert!(!path.exists(), "{} survived", path.display());
        }
        for path in &kept {
            assert!(path.exists(), "{} was swept", path.display());
        }
        assert_eq!(sweep_stale_temps(&root.join("missing"), pid, SystemTime::now(), Duration::from_secs(120)), 0);
    }

    #[test]
    fn stored_lists_expire_and_infos_count_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lists").join("jwst").join("jw1.images.json");
        let now = SystemTime::now();
        let written = write_stored(&path, &vec![1, 2, 3], now).unwrap();
        assert_eq!(written, std::fs::metadata(&path).unwrap().len());
        assert_eq!(read_stored::<Vec<i32>>(&path, LIST_TTL, now + Duration::from_secs(60)), Some(vec![1, 2, 3]));
        assert_eq!(read_stored::<Vec<i32>>(&path, LIST_TTL, now + LIST_TTL + Duration::from_secs(1)), None);
        assert_eq!(read_stored::<String>(&path, LIST_TTL, now), None);
        std::fs::write(&path, b"{broken").unwrap();
        assert_eq!(read_stored::<Vec<i32>>(&path, LIST_TTL, now), None);
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        write(&dir.path().join("img").join("jwst").join("a").join("t.jpg"), 40, 0);
        write(&dir.path().join("names.json"), 2, 0);
        let info = dir_info(dir.path());
        assert_eq!(info.files, 3);
        assert_eq!(info.bytes, 40 + 2 + std::fs::metadata(&path).unwrap().len());
        assert_eq!(dir_info(&dir.path().join("missing")), MastCacheInfo::default());
        let (freed, error) = remove_tree(&dir.path().join("img"));
        assert_eq!(freed, MastCacheInfo { bytes: 40, files: 1 });
        assert!(error.is_none());
        assert!(!dir.path().join("img").exists());
        let (again, error) = remove_tree(&dir.path().join("img"));
        assert_eq!(again, MastCacheInfo::default());
        assert!(error.is_none());
        assert_eq!(remove_file(&dir.path().join("names.json")).unwrap(), MastCacheInfo { bytes: 2, files: 1 });
        assert_eq!(remove_file(&dir.path().join("names.json")).unwrap(), MastCacheInfo::default());
    }

    #[test]
    fn disk_budget_asks_for_one_eviction_at_a_time() {
        let budget = DiskBudget::new(100);
        assert_eq!(budget.cap(), 100);
        assert_eq!(budget.target(), 90);
        assert_eq!(budget.used(), None);
        assert!(!budget.busy());
        assert!(budget.note_write(10), "unknown usage needs a scan");
        assert!(budget.busy());
        assert!(!budget.note_write(10), "a scan is already running");
        budget.finish(50);
        assert!(!budget.busy());
        assert_eq!(budget.used(), Some(60));
        assert!(!budget.note_write(30));
        assert!(budget.note_write(30));
        assert!(!budget.begin());
        budget.finish(85);
        assert_eq!(budget.used(), Some(85));
        assert!(budget.note_write(20));
        budget.finish(10);
        budget.reset(0);
        assert_eq!(budget.used(), Some(0));
        budget.reset(7);
        assert_eq!(budget.used(), Some(7));
        let fresh = DiskBudget::new(100);
        assert!(fresh.begin());
        assert!(!fresh.begin());
        fresh.finish(5);
        assert_eq!(fresh.used(), Some(5));
    }
}
