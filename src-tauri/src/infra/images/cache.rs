use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, LazyLock, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use ndarray::Array2;

use crate::core::history::stamp::{file_stamp, FileStamp};
use crate::infra::fits::band_planes::band_plane_of_path;
use crate::infra::image_source::{Companions, PlaneInfo};
#[cfg(test)]
use crate::types::constants::WIZARD_CACHE_PREFIX;
use crate::types::ImageStats;
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;
use crate::types::image_ref::ImageRef;

const MAX_TRACKED_STAMPS: usize = 1024;

pub struct PlaneLoad {
    pub arr: Array2<f32>,
    pub stats: ImageStats,
    pub header: HduHeader,
    pub int_plane: Option<IntPlane>,
    pub info: Option<PlaneInfo>,
    pub companions: Option<Companions>,
}

impl PlaneLoad {
    pub fn synthetic(arr: Array2<f32>, stats: ImageStats, header: HduHeader) -> Self {
        Self { arr, stats, header, int_plane: None, info: None, companions: None }
    }
}

struct CachedImage {
    arr: Arc<Array2<f32>>,
    stats: ImageStats,
    header: Option<HduHeader>,
    int_plane: Option<Arc<IntPlane>>,
    info: Option<PlaneInfo>,
    companions: Option<Companions>,
}

pub struct ImageEntry {
    inner: Arc<CachedImage>,
}

impl ImageEntry {
    pub fn arr(&self) -> &Array2<f32> {
        &self.inner.arr
    }

    pub fn stats(&self) -> &ImageStats {
        &self.inner.stats
    }

    pub fn data_arc(&self) -> Arc<Array2<f32>> {
        Arc::clone(&self.inner.arr)
    }

    pub fn header(&self) -> Option<&HduHeader> {
        self.inner.header.as_ref()
    }

    pub fn int_plane(&self) -> Option<&IntPlane> {
        self.inner.int_plane.as_deref()
    }

    pub fn plane_info(&self) -> Option<&PlaneInfo> {
        self.inner.info.as_ref()
    }

    pub fn companions(&self) -> Option<&Companions> {
        self.inner.companions.as_ref()
    }
}

impl Clone for ImageEntry {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct LruEntry {
    value: Arc<CachedImage>,
    gen: AtomicU64,
    byte_size: usize,
}

const APP_PINNED_PREFIXES: &[&str] = &["__composite", "__wizard_ch_"];

struct LruInner {
    map: HashMap<String, LruEntry>,
    aliases: HashMap<String, String>,
    max_entries: usize,
    max_bytes: usize,
    current_bytes: usize,
    pinned_bytes: usize,
    generation: AtomicU64,
    pinned_prefixes: &'static [&'static str],
}

impl LruInner {
    fn new(max_entries: usize, max_bytes: usize, pinned_prefixes: &'static [&'static str]) -> Self {
        Self {
            map: HashMap::with_capacity(max_entries),
            aliases: HashMap::new(),
            max_entries,
            max_bytes,
            current_bytes: 0,
            pinned_bytes: 0,
            generation: AtomicU64::new(0),
            pinned_prefixes,
        }
    }

    fn entry_bytes(entry: &Arc<CachedImage>) -> usize {
        let (rows, cols) = entry.arr.dim();
        let pixels = rows.saturating_mul(cols).saturating_mul(std::mem::size_of::<f32>());
        let ints = entry.int_plane.as_ref().map(|p| p.byte_size()).unwrap_or(0);
        pixels.saturating_add(ints)
    }

    fn next_gen(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn resolve<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        if self.map.contains_key(key) {
            return Some(key);
        }
        self.aliases.get(key).map(String::as_str).filter(|target| self.map.contains_key(*target))
    }

    fn get_readonly(&self, key: &str) -> Option<Arc<CachedImage>> {
        let entry = self.map.get(self.resolve(key)?)?;
        entry.gen.store(self.next_gen(), Ordering::Relaxed);
        Some(Arc::clone(&entry.value))
    }

    fn is_pinned(&self, key: &str) -> bool {
        self.pinned_prefixes.iter().any(|prefix| key.starts_with(prefix))
    }

    fn take(&mut self, key: &str) -> bool {
        let Some(removed) = self.map.remove(key) else {
            return false;
        };
        self.current_bytes = self.current_bytes.saturating_sub(removed.byte_size);
        if self.is_pinned(key) {
            self.pinned_bytes = self.pinned_bytes.saturating_sub(removed.byte_size);
        }
        self.aliases.retain(|_, target| target.as_str() != key);
        true
    }

    fn evict_lru(&mut self) -> bool {
        let victim = self
            .map
            .iter()
            .filter(|(k, _)| !self.is_pinned(k))
            .min_by_key(|(_, e)| e.gen.load(Ordering::Relaxed))
            .map(|(k, _)| k.clone());
        victim.is_some_and(|key| self.take(&key))
    }

    fn unpinned_len(&self) -> usize {
        self.map.keys().filter(|k| !self.is_pinned(k)).count()
    }

    fn unpinned_bytes(&self) -> usize {
        self.current_bytes.saturating_sub(self.pinned_bytes)
    }

    fn put(&mut self, key: String, value: Arc<CachedImage>) {
        let new_bytes = Self::entry_bytes(&value);
        let pinned = self.is_pinned(&key);

        self.aliases.remove(&key);
        self.take(&key);

        if pinned {
            self.pinned_bytes = self.pinned_bytes.saturating_add(new_bytes);
            if self.pinned_bytes > self.max_bytes {
                log::warn!(
                    "ImageCache: pinned entries hold {} bytes, more than the {} byte budget of the evictable entries",
                    self.pinned_bytes,
                    self.max_bytes,
                );
            }
        } else {
            while self.unpinned_bytes().saturating_add(new_bytes) > self.max_bytes || self.unpinned_len() >= self.max_entries {
                if !self.evict_lru() {
                    break;
                }
            }
        }

        let gen = self.next_gen();
        self.current_bytes = self.current_bytes.saturating_add(new_bytes);
        self.map.insert(
            key,
            LruEntry {
                value,
                gen: AtomicU64::new(gen),
                byte_size: new_bytes,
            },
        );
    }

    fn alias(&mut self, alias: &str, canonical: &str) {
        if alias != canonical && self.map.contains_key(canonical) && !self.map.contains_key(alias) {
            self.aliases.insert(alias.to_string(), canonical.to_string());
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(target) = self.aliases.remove(key) {
            self.take(&target);
        }
        self.take(key);
    }

    fn remove_where(&mut self, pred: impl Fn(&str) -> bool) {
        let keys: Vec<String> = self
            .map
            .keys()
            .filter(|k| pred(k))
            .chain(self.aliases.iter().filter(|(alias, _)| pred(alias)).map(|(_, target)| target))
            .cloned()
            .collect();
        for k in keys {
            self.take(&k);
        }
        self.aliases.retain(|alias, _| !pred(alias));
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn memory_estimate_bytes(&self) -> usize {
        self.current_bytes
    }
}

pub struct ImageCache {
    inner: RwLock<LruInner>,
    stamps: Mutex<HashMap<String, FileStamp>>,
    loads: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

fn source_file_of(r: &ImageRef) -> &str {
    match band_plane_of_path(&r.path) {
        Some(band) => &r.path[..band.file.len()],
        None => &r.path,
    }
}

fn is_key_of_source(key: &str, source: &str) -> bool {
    let r = ImageRef::parse(key);
    !r.is_synthetic() && source_file_of(&r) == source
}

impl ImageCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self::with_pinned_prefixes(max_entries, max_bytes, &[])
    }

    pub fn with_pinned_prefixes(max_entries: usize, max_bytes: usize, pinned_prefixes: &'static [&'static str]) -> Self {
        Self {
            inner: RwLock::new(LruInner::new(max_entries, max_bytes, pinned_prefixes)),
            stamps: Mutex::new(HashMap::new()),
            loads: Mutex::new(HashMap::new()),
        }
    }

    fn lock_loads(&self) -> MutexGuard<'_, HashMap<String, Arc<Mutex<()>>>> {
        self.loads.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load_once(
        &self,
        key: &str,
        cached: impl Fn(&LruInner) -> Option<Arc<CachedImage>>,
        build: impl FnOnce() -> Result<CachedImage>,
    ) -> Result<ImageEntry> {
        if let Some(inner) = cached(&self.inner.read().unwrap()) {
            return Ok(ImageEntry { inner });
        }
        let slot = Arc::clone(self.lock_loads().entry(key.to_string()).or_default());
        let result = {
            let _one_loader = if rayon::current_thread_index().is_some() {
                slot.try_lock().ok()
            } else {
                Some(slot.lock().unwrap_or_else(|e| e.into_inner()))
            };
            let waited_for = cached(&self.inner.read().unwrap());
            match waited_for {
                Some(inner) => Ok(ImageEntry { inner }),
                None => build().map(|built| {
                    let mut cache = self.inner.write().unwrap();
                    let inner = cached(&cache).unwrap_or_else(|| {
                        let entry = Arc::new(built);
                        cache.put(key.to_string(), Arc::clone(&entry));
                        entry
                    });
                    ImageEntry { inner }
                }),
            }
        };
        let mut loads = self.lock_loads();
        if loads.get(key).is_some_and(|current| Arc::ptr_eq(current, &slot)) && Arc::strong_count(&slot) <= 2 {
            loads.remove(key);
        }
        result
    }

    fn lock_stamps(&self) -> MutexGuard<'_, HashMap<String, FileStamp>> {
        self.stamps.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn invalidate_source(&self, source: &str) {
        self.remove_where(|key| is_key_of_source(key, source));
    }

    pub fn revalidate_source(&self, key: &str) -> Option<FileStamp> {
        let r = ImageRef::parse(key);
        if r.is_synthetic() {
            return None;
        }
        let source = source_file_of(&r).to_string();
        let meta = std::fs::metadata(&source);
        let mut stamps = self.lock_stamps();
        match meta {
            Ok(meta) => {
                let current = file_stamp(&meta);
                match stamps.get(&source) {
                    Some(known) if *known == current => {}
                    Some(_) => {
                        self.invalidate_source(&source);
                        stamps.insert(source, current);
                    }
                    None => {
                        if self.any_key(|k| is_key_of_source(k, &source)) {
                            self.invalidate_source(&source);
                        }
                        if stamps.len() >= MAX_TRACKED_STAMPS {
                            stamps.clear();
                        }
                        stamps.insert(source, current);
                    }
                }
                Some(current)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.invalidate_source(&source);
                stamps.remove(&source);
                None
            }
            Err(_) => None,
        }
    }

    pub fn forget_if_rewritten(&self, key: &str, before: Option<FileStamp>) {
        let r = ImageRef::parse(key);
        if r.is_synthetic() {
            return;
        }
        let source = source_file_of(&r);
        let now = std::fs::metadata(source).ok().map(|meta| file_stamp(&meta));
        if now != before {
            self.invalidate_source(source);
        }
    }

    pub fn invalidate_written(&self, path: &str) {
        let source = source_file_of(&ImageRef::parse(path)).to_string();
        let meta = std::fs::metadata(&source);
        let mut stamps = self.lock_stamps();
        self.invalidate_source(&source);
        match meta {
            Ok(meta) => {
                stamps.insert(source, file_stamp(&meta));
            }
            Err(_) => {
                stamps.remove(&source);
            }
        }
    }

    pub fn get(&self, path: &str) -> Option<ImageEntry> {
        let cache = self.inner.read().unwrap();
        cache.get_readonly(path).map(|inner| ImageEntry { inner })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner.read().unwrap().resolve(key).is_some()
    }

    pub fn holds(&self, key: &str, data: &Weak<Array2<f32>>) -> Option<bool> {
        let cache = self.inner.read().unwrap();
        let entry = cache.map.get(cache.resolve(key)?)?;
        Some(std::ptr::eq(Arc::as_ptr(&entry.value.arr), data.as_ptr()))
    }

    pub fn get_or_load<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats)>,
    {
        self.load_once(
            path,
            |cache| cache.get_readonly(path),
            || {
                let (arr, stats) = loader()?;
                Ok(CachedImage { arr: Arc::new(arr), stats, header: None, int_plane: None, info: None, companions: None })
            },
        )
    }

    pub fn get_or_load_full<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats, HduHeader)>,
    {
        self.load_once(
            path,
            |cache| cache.get_readonly(path).filter(|entry| entry.header.is_some()),
            || {
                let (arr, stats, header) = loader()?;
                Ok(CachedImage { arr: Arc::new(arr), stats, header: Some(header), int_plane: None, info: None, companions: None })
            },
        )
    }

    pub fn get_or_load_plane<F>(&self, key: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<PlaneLoad>,
    {
        self.load_once(
            key,
            |cache| cache.get_readonly(key).filter(|entry| entry.header.is_some()),
            || {
                let load = loader()?;
                Ok(CachedImage {
                    arr: Arc::new(load.arr),
                    stats: load.stats,
                    header: Some(load.header),
                    int_plane: load.int_plane.map(Arc::new),
                    info: load.info,
                    companions: load.companions,
                })
            },
        )
    }

    pub fn get_or_load_plane_as<F>(&self, key: &str, canonical: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<PlaneLoad>,
    {
        let entry = self.get_or_load_plane(canonical, loader)?;
        self.inner.write().unwrap().alias(key, canonical);
        Ok(entry)
    }

    pub fn upgrade_header<F>(&self, path: &str, header_loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<HduHeader>,
    {
        {
            let cache = self.inner.read().unwrap();
            if let Some(entry) = cache.get_readonly(path) {
                if entry.header.is_some() {
                    return Ok(ImageEntry { inner: entry });
                }
                drop(cache);

                let header = header_loader()?;
                let upgraded = Arc::new(CachedImage {
                    arr: Arc::clone(&entry.arr),
                    stats: entry.stats.clone(),
                    header: Some(header),
                    int_plane: entry.int_plane.clone(),
                    info: entry.info.clone(),
                    companions: entry.companions.clone(),
                });
                let mut w = self.inner.write().unwrap();
                let stored_as = w.resolve(path).unwrap_or(path).to_string();
                w.put(stored_as, Arc::clone(&upgraded));
                return Ok(ImageEntry { inner: upgraded });
            }
        }
        Err(anyhow::anyhow!("No cached entry to upgrade for {}", path))
    }

    pub fn insert_synthetic(&self, key: &str, arr: Arc<Array2<f32>>, stats: ImageStats) {
        self.insert_synthetic_with_header(key, arr, stats, None);
    }

    pub fn insert_synthetic_with_header(
        &self,
        key: &str,
        arr: Arc<Array2<f32>>,
        stats: ImageStats,
        header: Option<HduHeader>,
    ) {
        #[cfg(test)]
        assert_wizard_entries_locked(self, key.starts_with(WIZARD_CACHE_PREFIX));
        let header = header.map(|mut header| {
            let (rows, cols) = arr.dim();
            header.set("NAXIS1", cols.to_string());
            header.set("NAXIS2", rows.to_string());
            header
        });
        let entry = Arc::new(CachedImage {
            arr,
            stats,
            header,
            int_plane: None,
            info: None,
            companions: None,
        });
        let mut cache = self.inner.write().unwrap();
        cache.put(key.to_string(), entry);
    }

    pub fn invalidate(&self, path: &str) {
        #[cfg(test)]
        assert_wizard_entries_locked(self, path.starts_with(WIZARD_CACHE_PREFIX));
        let mut cache = self.inner.write().unwrap();
        cache.remove(path);
    }

    pub fn remove(&self, key: &str) {
        self.invalidate(key);
    }

    pub fn remove_prefix(&self, prefix: &str) {
        #[cfg(test)]
        assert_wizard_entries_locked(self, overlaps_wizard_keys(prefix));
        let mut cache = self.inner.write().unwrap();
        cache.remove_where(|k| k.starts_with(prefix));
    }

    pub fn remove_where(&self, pred: impl Fn(&str) -> bool) {
        let mut cache = self.inner.write().unwrap();
        cache.remove_where(pred);
    }

    pub fn remove_prefix_except(&self, prefix: &str, keep_prefix: &str) {
        #[cfg(test)]
        assert_wizard_entries_locked(self, overlaps_wizard_keys(prefix));
        let mut cache = self.inner.write().unwrap();
        cache.remove_where(|k| k.starts_with(prefix) && !k.starts_with(keep_prefix));
    }

    pub fn any_key(&self, pred: impl Fn(&str) -> bool) -> bool {
        self.inner.read().unwrap().map.keys().any(|k| pred(k))
    }

    pub fn len(&self) -> usize {
        let cache = self.inner.read().unwrap();
        cache.len()
    }

    pub fn memory_estimate_bytes(&self) -> usize {
        let cache = self.inner.read().unwrap();
        cache.memory_estimate_bytes()
    }

    pub fn pinned_bytes(&self) -> usize {
        self.inner.read().unwrap().pinned_bytes
    }
}

const DEFAULT_MAX_ENTRIES: usize = 32;
const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024 * 1024;

pub static GLOBAL_IMAGE_CACHE: LazyLock<ImageCache> = LazyLock::new(|| {
    ImageCache::with_pinned_prefixes(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_BYTES, APP_PINNED_PREFIXES)
});

#[cfg(test)]
static WIZARD_ENTRIES: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_wizard_entries() -> std::sync::MutexGuard<'static, ()> {
    WIZARD_ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
fn overlaps_wizard_keys(prefix: &str) -> bool {
    prefix.starts_with(WIZARD_CACHE_PREFIX) || WIZARD_CACHE_PREFIX.starts_with(prefix)
}

#[cfg(test)]
fn assert_wizard_entries_locked(cache: &ImageCache, touches_wizard_keys: bool) {
    if touches_wizard_keys && std::ptr::eq(cache, &*GLOBAL_IMAGE_CACHE) {
        assert!(
            matches!(WIZARD_ENTRIES.try_lock(), Err(std::sync::TryLockError::WouldBlock)),
            "a test wrote or removed {WIZARD_CACHE_PREFIX}* keys of GLOBAL_IMAGE_CACHE without holding lock_wizard_entries()"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::image_ref::ImageRef;
    use crate::types::ImageStats;

    fn make_test_entry(rows: usize, cols: usize) -> (Array2<f32>, ImageStats) {
        let arr = Array2::from_elem((rows, cols), 1.0f32);
        let stats = ImageStats {
            min: 0.0,
            max: 1.0,
            median: 0.5,
            mad: 0.1,
            sigma: 0.148,
            mean: 0.5,
            valid_count: (rows * cols) as u64,
        };
        (arr, stats)
    }

    #[test]
    fn test_get_or_load_caches() {
        let cache = ImageCache::new(4, usize::MAX);
        let mut load_count = 0u32;

        let entry1 = cache
            .get_or_load("file1.fits", || {
                load_count += 1;
                Ok(make_test_entry(100, 100))
            })
            .unwrap();
        assert_eq!(entry1.arr().dim(), (100, 100));
        assert_eq!(load_count, 1);

        let entry2 = cache
            .get_or_load("file1.fits", || {
                load_count += 1;
                Ok(make_test_entry(200, 200))
            })
            .unwrap();
        assert_eq!(entry2.arr().dim(), (100, 100));
        assert_eq!(load_count, 1);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = ImageCache::new(2, usize::MAX);

        cache
            .get_or_load("a", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("b", || Ok(make_test_entry(20, 20)))
            .unwrap();
        assert_eq!(cache.len(), 2);

        cache
            .get_or_load("c", || Ok(make_test_entry(30, 30)))
            .unwrap();
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_lru_access_refreshes() {
        let cache = ImageCache::new(2, usize::MAX);

        cache
            .get_or_load("a", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("b", || Ok(make_test_entry(20, 20)))
            .unwrap();

        let _ = cache.get("a");

        cache
            .get_or_load("c", || Ok(make_test_entry(30, 30)))
            .unwrap();
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_contains_is_non_touching() {
        let cache = ImageCache::new(2, usize::MAX);

        cache
            .get_or_load("a", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("b", || Ok(make_test_entry(20, 20)))
            .unwrap();
        assert!(cache.contains("a"));
        assert!(cache.contains("b"));
        assert!(!cache.contains("z"));

        for _ in 0..5 {
            assert!(cache.contains("a"));
        }

        cache
            .get_or_load("c", || Ok(make_test_entry(30, 30)))
            .unwrap();
        assert!(!cache.contains("a"));
        assert!(cache.contains("b"));
        assert!(cache.contains("c"));
    }

    #[test]
    fn test_arc_zero_copy() {
        let cache = ImageCache::new(4, usize::MAX);
        cache
            .get_or_load("x", || Ok(make_test_entry(1000, 1000)))
            .unwrap();

        let e1 = cache.get("x").unwrap();
        let e2 = cache.get("x").unwrap();
        let ptr1 = e1.arr().as_ptr();
        let ptr2 = e2.arr().as_ptr();
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_memory_estimate() {
        let cache = ImageCache::new(4, usize::MAX);
        cache
            .get_or_load("a", || Ok(make_test_entry(100, 100)))
            .unwrap();
        assert_eq!(cache.memory_estimate_bytes(), 100 * 100 * 4);
    }

    #[test]
    fn test_invalidate() {
        let cache = ImageCache::new(4, usize::MAX);
        cache
            .get_or_load("a", || Ok(make_test_entry(10, 10)))
            .unwrap();
        assert_eq!(cache.len(), 1);
        cache.invalidate("a");
        assert_eq!(cache.len(), 0);
        assert!(cache.get("a").is_none());
    }

    fn app_cache(max_entries: usize) -> ImageCache {
        ImageCache::with_pinned_prefixes(max_entries, usize::MAX, APP_PINNED_PREFIXES)
    }

    #[test]
    fn a_cache_without_pinned_prefixes_evicts_client_named_composite_keys() {
        let cache = ImageCache::new(2, usize::MAX);
        for i in 0..5 {
            let (arr, stats) = make_test_entry(4, 4);
            cache.insert_synthetic(&format!("__composite_{i}"), Arc::new(arr), stats);
        }
        assert_eq!(cache.len(), 2, "names chosen by a client must not bypass the entry cap");
        assert_eq!(cache.memory_estimate_bytes(), 2 * 4 * 4 * 4);
        assert!(cache.get("__composite_4").is_some());
        assert!(cache.get("__composite_0").is_none());
    }

    #[test]
    fn the_app_cache_does_not_pin_the_unused_star_mask_key() {
        let cache = app_cache(1);
        let (arr, stats) = make_test_entry(2, 2);
        cache.insert_synthetic("__star_mask", Arc::new(arr), stats);
        cache.get_or_load("a.fits", || Ok(make_test_entry(2, 2))).unwrap();
        assert!(cache.get("__star_mask").is_none());
        assert!(cache.get("a.fits").is_some());
    }

    #[test]
    fn remove_where_and_any_key_select_by_predicate() {
        let cache = ImageCache::new(8, usize::MAX);
        for key in ["a.fits", "a.fits#hdu=3", "ab.fits"] {
            cache.get_or_load(key, || Ok(make_test_entry(2, 2))).unwrap();
        }
        assert!(cache.any_key(|k| ImageRef::parse(k).path == "a.fits"));
        cache.remove_where(|k| ImageRef::parse(k).path == "a.fits");
        assert!(!cache.any_key(|k| ImageRef::parse(k).path == "a.fits"));
        assert!(cache.contains("ab.fits"));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.memory_estimate_bytes(), 2 * 2 * 4);
    }

    #[test]
    fn remove_prefix_except_keeps_only_the_named_run() {
        let cache = ImageCache::new(8, usize::MAX);
        for key in ["__wizard_ch_ta_r_aligned", "__wizard_ch_b_aligned", "__wizard_ch_tb_r_aligned", "__wizard_ch_tb_g_cropped", "plain.fits"] {
            cache.get_or_load(key, || Ok(make_test_entry(2, 2))).unwrap();
        }
        cache.remove_prefix_except("__wizard_ch_", "__wizard_ch_tb_");
        assert!(cache.contains("__wizard_ch_tb_r_aligned"));
        assert!(cache.contains("__wizard_ch_tb_g_cropped"));
        assert!(cache.contains("plain.fits"));
        assert!(!cache.contains("__wizard_ch_ta_r_aligned"));
        assert!(!cache.contains("__wizard_ch_b_aligned"), "a legacy bin key must not be mistaken for a run key");
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_pinned_only_does_not_infinite_loop() {
        let cache = app_cache(2);
        cache
            .get_or_load("__composite_r", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("__composite_g", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("__composite_b", || Ok(make_test_entry(10, 10)))
            .unwrap();
        assert_eq!(cache.len(), 3);
        assert!(cache.get("__composite_r").is_some());
        assert!(cache.get("__composite_g").is_some());
        assert!(cache.get("__composite_b").is_some());
    }

    #[test]
    fn test_pinned_entries_do_not_consume_regular_slots() {
        let cache = app_cache(2);
        let (arr, stats) = make_test_entry(10, 10);
        cache.insert_synthetic("__wizard_ch_ha_aligned", Arc::new(arr.clone()), stats.clone());
        cache.insert_synthetic("__wizard_ch_oiii_aligned", Arc::new(arr.clone()), stats.clone());
        cache.insert_synthetic("__wizard_ch_sii_aligned", Arc::new(arr), stats);

        cache
            .get_or_load("a.fits", || Ok(make_test_entry(10, 10)))
            .unwrap();
        cache
            .get_or_load("b.fits", || Ok(make_test_entry(10, 10)))
            .unwrap();
        assert!(cache.get("a.fits").is_some());
        assert!(cache.get("b.fits").is_some());

        cache
            .get_or_load("c.fits", || Ok(make_test_entry(10, 10)))
            .unwrap();
        assert!(cache.get("a.fits").is_none());
        assert!(cache.get("b.fits").is_some());
        assert!(cache.get("c.fits").is_some());
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn test_remove_prefix_releases_pinned_bytes() {
        let cache = app_cache(8);
        let (arr, stats) = make_test_entry(10, 10);
        cache.insert_synthetic("__wizard_ch_ha_aligned", Arc::new(arr.clone()), stats.clone());
        cache.insert_synthetic("__wizard_ch_ha_cropped", Arc::new(arr.clone()), stats.clone());
        cache.insert_synthetic("__composite_r", Arc::new(arr), stats);
        cache
            .get_or_load("a.fits", || Ok(make_test_entry(10, 10)))
            .unwrap();
        assert_eq!(cache.len(), 4);
        assert_eq!(cache.memory_estimate_bytes(), 4 * 10 * 10 * 4);

        cache.remove_prefix("__wizard_ch_");

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.memory_estimate_bytes(), 2 * 10 * 10 * 4);
        assert!(cache.get("__wizard_ch_ha_aligned").is_none());
        assert!(cache.get("__wizard_ch_ha_cropped").is_none());
        assert!(cache.get("__composite_r").is_some());
        assert!(cache.get("a.fits").is_some());
    }

    fn int_plane(rows: usize, cols: usize) -> IntPlane {
        IntPlane { bits: Array2::from_elem((rows, cols), 3u32), signed: false }
    }

    fn companions() -> Companions {
        Companions {
            dq: Some(ImageRef::hdu("a.fits", 3)),
            err: Some(ImageRef::hdu("a.fits", 2)),
            wavelength: None,
        }
    }

    fn plane(rows: usize, cols: usize, ints: bool) -> PlaneLoad {
        let (arr, stats) = make_test_entry(rows, cols);
        PlaneLoad {
            arr,
            stats,
            header: HduHeader::empty(),
            int_plane: ints.then(|| int_plane(rows, cols)),
            info: None,
            companions: Some(companions()),
        }
    }

    #[test]
    fn get_or_load_plane_stores_int_plane_and_counts_its_bytes() {
        let cache = ImageCache::new(4, usize::MAX);
        let entry = cache
            .get_or_load_plane("a.fits#hdu=3", || Ok(plane(10, 10, true)))
            .unwrap();
        assert!(entry.header().is_some());
        assert_eq!(entry.int_plane().unwrap().bits[[0, 0]], 3);
        assert_eq!(cache.memory_estimate_bytes(), 10 * 10 * 4 + 10 * 10 * 4);

        let again = cache
            .get_or_load_plane("a.fits#hdu=3", || panic!("must not reload"))
            .unwrap();
        assert!(again.int_plane().is_some());

        let none = cache
            .get_or_load_plane("b.fits", || Ok(plane(4, 4, false)))
            .unwrap();
        assert!(none.int_plane().is_none());
        assert_eq!(cache.memory_estimate_bytes(), 800 + 64);
    }

    #[test]
    fn pinned_bytes_over_the_budget_do_not_evict_every_unpinned_entry() {
        let cache = ImageCache::with_pinned_prefixes(8, 100, &["__p"]);
        let (arr, stats) = make_test_entry(5, 6);
        cache.insert_synthetic("__p_composite", Arc::new(arr), stats);
        cache.get_or_load("a", || Ok(make_test_entry(1, 3))).unwrap();
        cache.get_or_load("b", || Ok(make_test_entry(1, 3))).unwrap();
        assert!(cache.contains("__p_composite"));
        assert!(cache.contains("a"), "120 pinned bytes over a 100 byte budget must not cost the unpinned entries their place");
        assert!(cache.contains("b"));
        assert_eq!(cache.memory_estimate_bytes(), 120 + 12 + 12);
        assert_eq!(cache.pinned_bytes(), 120);

        for key in ["c", "d", "e", "f", "g", "h", "i"] {
            cache.get_or_load(key, || Ok(make_test_entry(1, 3))).unwrap();
        }
        assert!(!cache.contains("a"), "the byte budget still bounds the unpinned entries");
        assert!(cache.contains("i"));
        assert!(cache.contains("__p_composite"));
        assert!(cache.memory_estimate_bytes() - cache.pinned_bytes() <= 100);
    }

    #[test]
    fn two_loads_of_one_cold_key_run_the_loader_once() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let cache = Arc::new(ImageCache::new(4, usize::MAX));
        let calls = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let first = {
            let (cache, calls) = (Arc::clone(&cache), Arc::clone(&calls));
            std::thread::spawn(move || {
                cache
                    .get_or_load_plane("slow.fits#hdu=1", || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(plane(4, 4, false))
                    })
                    .map(|entry| entry.arr().dim())
            })
        };
        started_rx.recv().unwrap();
        let second = {
            let (cache, calls) = (Arc::clone(&cache), Arc::clone(&calls));
            std::thread::spawn(move || {
                cache
                    .get_or_load_plane("slow.fits#hdu=1", || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(plane(8, 8, false))
                    })
                    .map(|entry| entry.arr().dim())
            })
        };
        let deadline = Instant::now() + Duration::from_millis(300);
        while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        release_tx.send(()).unwrap();
        assert_eq!(first.join().unwrap().unwrap(), (4, 4));
        assert_eq!(second.join().unwrap().unwrap(), (4, 4), "the second caller takes the plane the first one loaded");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one cold key was decoded more than once");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_failed_load_lets_the_next_caller_try_again() {
        let cache = ImageCache::new(4, usize::MAX);
        let failed = cache.get_or_load_plane("flaky.fits", || anyhow::bail!("disk hiccup"));
        assert!(failed.is_err());
        let entry = cache.get_or_load_plane("flaky.fits", || Ok(plane(2, 2, false))).unwrap();
        assert_eq!(entry.arr().dim(), (2, 2));
    }

    #[test]
    fn get_or_load_plane_keeps_companions_and_synthetic_has_none() {
        let cache = ImageCache::new(4, usize::MAX);
        let entry = cache
            .get_or_load_plane("a.fits#hdu=1", || Ok(plane(2, 2, false)))
            .unwrap();
        assert_eq!(entry.companions(), Some(&companions()));
        assert!(entry.plane_info().is_none());

        let (arr, stats) = make_test_entry(2, 2);
        let synthetic = cache
            .get_or_load_plane("cut", || Ok(PlaneLoad::synthetic(arr, stats, HduHeader::empty())))
            .unwrap();
        assert!(synthetic.companions().is_none());
        assert!(synthetic.int_plane().is_none());
        let (arr, stats) = make_test_entry(2, 2);
        cache.insert_synthetic("__composite_r", Arc::new(arr), stats);
        assert!(cache.get("__composite_r").unwrap().companions().is_none());
    }

    #[test]
    fn get_or_load_plane_replaces_headerless_entry() {
        let cache = ImageCache::new(4, usize::MAX);
        cache.get_or_load("k", || Ok(make_test_entry(5, 5))).unwrap();
        let entry = cache
            .get_or_load_plane("k", || Ok(plane(6, 6, true)))
            .unwrap();
        assert_eq!(entry.arr().dim(), (6, 6));
        assert!(entry.int_plane().is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn upgrade_header_keeps_int_plane_and_companions() {
        let cache = ImageCache::new(4, usize::MAX);
        let (arr, stats) = make_test_entry(3, 3);
        let inner = Arc::new(CachedImage {
            arr: Arc::new(arr),
            stats,
            header: None,
            int_plane: Some(Arc::new(int_plane(3, 3))),
            info: None,
            companions: Some(companions()),
        });
        cache.inner.write().unwrap().put("p".into(), inner);
        let upgraded = cache.upgrade_header("p", || Ok(HduHeader::empty())).unwrap();
        assert!(upgraded.header().is_some());
        assert!(upgraded.int_plane().is_some());
        assert_eq!(upgraded.companions(), Some(&companions()));
        assert_eq!(cache.memory_estimate_bytes(), 9 * 4 * 2);
    }

    #[test]
    fn plane_refs_are_not_pinned_and_evict_normally() {
        let cache = ImageCache::new(2, usize::MAX);
        cache
            .get_or_load_plane("a.fits#hdu=2", || Ok(plane(2, 2, false)))
            .unwrap();
        cache.get_or_load("b.fits", || Ok(make_test_entry(2, 2))).unwrap();
        cache.get_or_load("c.fits", || Ok(make_test_entry(2, 2))).unwrap();
        assert!(cache.get("a.fits#hdu=2").is_none());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_synthetic_entry_can_carry_a_header_that_describes_its_own_grid() {
        let cache = ImageCache::new(4, usize::MAX);
        let (arr, stats) = make_test_entry(3, 5);
        let mut header = HduHeader::empty();
        header.set_f64("CRVAL1", 12.5);
        header.set("NAXIS1", "999".to_string());
        cache.insert_synthetic_with_header("__wizard_ch_r_aligned", Arc::new(arr), stats, Some(header));
        let carried = cache.get("__wizard_ch_r_aligned").unwrap();
        let h = carried.header().expect("header stored with the synthetic entry");
        assert_eq!(h.get_f64("CRVAL1"), Some(12.5));
        assert_eq!(h.get_i64("NAXIS1"), Some(5), "NAXIS1 must describe the entry's own columns");
        assert_eq!(h.get_i64("NAXIS2"), Some(3), "NAXIS2 must describe the entry's own rows");
        assert_eq!(cache.memory_estimate_bytes(), 3 * 5 * 4);

        let (arr, stats) = make_test_entry(2, 2);
        cache.insert_synthetic("__wizard_ch_g_aligned", Arc::new(arr), stats);
        assert!(cache.get("__wizard_ch_g_aligned").unwrap().header().is_none());
        let (arr, stats) = make_test_entry(2, 2);
        cache.insert_synthetic_with_header("__wizard_ch_b_aligned", Arc::new(arr), stats, None);
        assert!(cache.get("__wizard_ch_b_aligned").unwrap().header().is_none());
    }

    #[test]
    fn test_get_or_load_full_with_header() {
        let cache = ImageCache::new(4, usize::MAX);
        let entry = cache
            .get_or_load_full("h.fits", || {
                let (arr, stats) = make_test_entry(10, 10);
                let header = crate::types::header::HduHeader {
                    cards: vec![("SIMPLE".to_string(), "T".to_string())],
                    index: std::collections::HashMap::from([(
                        "SIMPLE".to_string(),
                        "T".to_string(),
                    )]),
                    string_keys: None,
                };
                Ok((arr, stats, header))
            })
            .unwrap();
        assert!(entry.header().is_some());
        assert_eq!(entry.header().unwrap().get("SIMPLE"), Some("T"));
    }

    fn load_entries(cache: &ImageCache, keys: &[&str]) {
        for key in keys {
            cache.get_or_load(key, || Ok(make_test_entry(2, 2))).unwrap();
        }
    }

    #[test]
    fn every_key_of_a_rewritten_file_is_dropped_together_band_planes_included() {
        let dir = tempfile::tempdir().unwrap();
        let f_path = dir.path().join("f.fits");
        let g_path = dir.path().join("g.fits");
        std::fs::write(&f_path, b"abc").unwrap();
        std::fs::write(&g_path, b"g").unwrap();
        let (f, g) = (f_path.to_str().unwrap(), g_path.to_str().unwrap());
        let band = format!("{f}#hdu=0&plane=1");
        let band_hdu = format!("{band}#hdu=0");
        let hdu = format!("{f}#hdu=1");
        let cache = ImageCache::new(8, usize::MAX);

        assert!(cache.revalidate_source(&band).is_some());
        load_entries(&cache, &[f, &hdu, &band, &band_hdu, g]);
        cache.revalidate_source(&band);
        assert_eq!(cache.len(), 5);

        std::fs::write(&f_path, b"rewritten more").unwrap();
        assert!(cache.revalidate_source(f).is_some());
        assert_eq!(cache.len(), 1);
        assert!(cache.contains(g));
    }

    #[test]
    fn a_band_plane_entry_follows_its_file_when_it_is_written_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let f_path = dir.path().join("f.fits");
        std::fs::write(&f_path, b"abc").unwrap();
        let f = f_path.to_str().unwrap();
        let band = format!("{f}#hdu=0&plane=1");
        let cache = ImageCache::new(8, usize::MAX);

        load_entries(&cache, &[f, &band]);
        cache.invalidate_written(f);
        assert_eq!(cache.len(), 0);

        load_entries(&cache, &[f, &band]);
        std::fs::remove_file(&f_path).unwrap();
        assert!(cache.revalidate_source(&band).is_none());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_file_named_like_a_plane_key_keeps_its_own_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let odd_path = dir.path().join("odd.fits");
        let named_path = dir.path().join("odd.fits#hdu=0&plane=1");
        std::fs::write(&odd_path, b"abc").unwrap();
        std::fs::write(&named_path, b"def").unwrap();
        let (odd, named) = (odd_path.to_str().unwrap(), named_path.to_str().unwrap());
        let cache = ImageCache::new(8, usize::MAX);
        for key in [odd, named] {
            assert!(cache.revalidate_source(key).is_some(), "{key}");
            load_entries(&cache, &[key]);
        }

        std::fs::write(&odd_path, b"abcd").unwrap();
        cache.revalidate_source(odd);
        assert!(!cache.contains(odd));
        assert!(cache.contains(named));

        std::fs::write(&named_path, b"defg").unwrap();
        cache.revalidate_source(named);
        assert!(!cache.contains(named));
        assert_eq!(cache.len(), 0);
    }
}
