use std::collections::HashMap;
use std::sync::{Arc, RwLock, LazyLock};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use ndarray::Array2;

use crate::types::ImageStats;
use crate::types::header::HduHeader;

struct CachedImage {
    arr: Arc<Array2<f32>>,
    stats: ImageStats,
    header: Option<HduHeader>,
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

struct LruInner {
    map: HashMap<String, LruEntry>,
    max_entries: usize,
    max_bytes: usize,
    current_bytes: usize,
    generation: AtomicU64,
}

impl LruInner {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            map: HashMap::with_capacity(max_entries),
            max_entries,
            max_bytes,
            current_bytes: 0,
            generation: AtomicU64::new(0),
        }
    }

    fn entry_bytes(entry: &Arc<CachedImage>) -> usize {
        let (rows, cols) = entry.arr.dim();
        rows * cols * std::mem::size_of::<f32>()
    }

    fn next_gen(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn get_readonly(&self, key: &str) -> Option<Arc<CachedImage>> {
        if let Some(entry) = self.map.get(key) {
            entry.gen.store(self.next_gen(), Ordering::Relaxed);
            Some(Arc::clone(&entry.value))
        } else {
            None
        }
    }

    fn is_pinned(key: &str) -> bool {
        key.starts_with("__composite") || key.starts_with("__wizard_ch_") || key == "__star_mask"
    }

    fn evict_lru(&mut self) {
        if self.map.is_empty() {
            return;
        }
        let victim = self
            .map
            .iter()
            .filter(|(k, _)| !Self::is_pinned(k))
            .min_by_key(|(_, e)| e.gen.load(Ordering::Relaxed))
            .map(|(k, _)| k.clone());
        if let Some(key) = victim {
            if let Some(removed) = self.map.remove(&key) {
                self.current_bytes -= removed.byte_size;
            }
        }
    }

    fn put(&mut self, key: String, value: Arc<CachedImage>) {
        let new_bytes = Self::entry_bytes(&value);

        if let Some(old) = self.map.remove(&key) {
            self.current_bytes -= old.byte_size;
        }

        while (self.current_bytes + new_bytes > self.max_bytes
            || self.map.len() >= self.max_entries)
            && !self.map.is_empty()
        {
            self.evict_lru();
        }

        let gen = self.next_gen();
        self.current_bytes += new_bytes;
        self.map.insert(
            key,
            LruEntry {
                value,
                gen: AtomicU64::new(gen),
                byte_size: new_bytes,
            },
        );
    }

    fn remove(&mut self, key: &str) {
        if let Some(removed) = self.map.remove(key) {
            self.current_bytes -= removed.byte_size;
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.current_bytes = 0;
        self.generation.store(0, Ordering::Relaxed);
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
}

impl ImageCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(LruInner::new(max_entries, max_bytes)),
        }
    }

    pub fn get(&self, path: &str) -> Option<ImageEntry> {
        let cache = self.inner.read().unwrap();
        cache.get_readonly(path).map(|inner| ImageEntry { inner })
    }

    pub fn get_or_load<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats)>,
    {
        {
            let cache = self.inner.read().unwrap();
            if let Some(entry) = cache.get_readonly(path) {
                return Ok(ImageEntry { inner: entry });
            }
        }

        let (arr, stats) = loader()?;
        let entry = Arc::new(CachedImage {
            arr: Arc::new(arr),
            stats,
            header: None,
        });

        {
            let mut cache = self.inner.write().unwrap();
            if let Some(existing) = cache.get_readonly(path) {
                return Ok(ImageEntry { inner: existing });
            }
            cache.put(path.to_string(), Arc::clone(&entry));
        }

        Ok(ImageEntry { inner: entry })
    }

    pub fn get_or_load_full<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats, HduHeader)>,
    {
        {
            let cache = self.inner.read().unwrap();
            if let Some(entry) = cache.get_readonly(path) {
                if entry.header.is_some() {
                    return Ok(ImageEntry { inner: entry });
                }
            }
        }

        let (arr, stats, header) = loader()?;
        let entry = Arc::new(CachedImage {
            arr: Arc::new(arr),
            stats,
            header: Some(header),
        });

        {
            let mut cache = self.inner.write().unwrap();
            if let Some(existing) = cache.get_readonly(path) {
                if existing.header.is_some() {
                    return Ok(ImageEntry { inner: existing });
                }
            }
            cache.put(path.to_string(), Arc::clone(&entry));
        }

        Ok(ImageEntry { inner: entry })
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
                });
                let mut w = self.inner.write().unwrap();
                w.put(path.to_string(), Arc::clone(&upgraded));
                return Ok(ImageEntry { inner: upgraded });
            }
        }
        Err(anyhow::anyhow!("No cached entry to upgrade for {}", path))
    }

    pub fn insert_synthetic(&self, key: &str, arr: Arc<Array2<f32>>, stats: ImageStats) {
        let entry = Arc::new(CachedImage {
            arr,
            stats,
            header: None,
        });
        let mut cache = self.inner.write().unwrap();
        cache.put(key.to_string(), entry);
    }

    pub fn invalidate(&self, path: &str) {
        let mut cache = self.inner.write().unwrap();
        cache.remove(path);
    }

    pub fn remove(&self, key: &str) {
        self.invalidate(key);
    }

    pub fn clear(&self) {
        let mut cache = self.inner.write().unwrap();
        cache.clear();
    }

    pub fn len(&self) -> usize {
        let cache = self.inner.read().unwrap();
        cache.len()
    }

    pub fn memory_estimate_bytes(&self) -> usize {
        let cache = self.inner.read().unwrap();
        cache.memory_estimate_bytes()
    }
}

const DEFAULT_MAX_ENTRIES: usize = 32;
const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024 * 1024;

pub static GLOBAL_IMAGE_CACHE: LazyLock<ImageCache> =
    LazyLock::new(|| ImageCache::new(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_BYTES));

#[cfg(test)]
mod tests {
    use super::*;
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
                };
                Ok((arr, stats, header))
            })
            .unwrap();
        assert!(entry.header().is_some());
        assert_eq!(entry.header().unwrap().get("SIMPLE"), Some("T"));
    }
}
