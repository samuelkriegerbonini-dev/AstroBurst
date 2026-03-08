use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock, LazyLock};

use anyhow::Result;
use ndarray::Array2;

use crate::types::ImageStats;
use crate::types::header::HduHeader;

struct CachedImage {
    arr: Array2<f32>,
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

struct LruInner {
    map: HashMap<String, Arc<CachedImage>>,
    order: VecDeque<String>,
    capacity: usize,
}

impl LruInner {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<CachedImage>> {
        if self.map.contains_key(key) {
            self.order.retain(|k| k != key);
            self.order.push_back(key.to_string());
            self.map.get(key).map(Arc::clone)
        } else {
            None
        }
    }

    fn put(&mut self, key: String, value: Arc<CachedImage>) {
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        } else if self.map.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn memory_estimate_bytes(&self) -> usize {
        self.map.values().map(|v| {
            let (rows, cols) = v.arr.dim();
            rows * cols * std::mem::size_of::<f32>()
        }).sum()
    }
}

pub struct ImageCache {
    inner: RwLock<LruInner>,
}

impl ImageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(LruInner::new(capacity)),
        }
    }

    pub fn get(&self, path: &str) -> Option<ImageEntry> {
        let mut cache = self.inner.write().unwrap();
        cache.get(path).map(|inner| ImageEntry { inner })
    }

    pub fn get_or_load<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats)>,
    {
        {
            let mut cache = self.inner.write().unwrap();
            if let Some(entry) = cache.get(path) {
                return Ok(ImageEntry { inner: entry });
            }
        }

        let (arr, stats) = loader()?;
        let entry = Arc::new(CachedImage { arr, stats, header: None });

        {
            let mut cache = self.inner.write().unwrap();
            cache.put(path.to_string(), Arc::clone(&entry));
        }

        Ok(ImageEntry { inner: entry })
    }

    pub fn get_or_load_full<F>(&self, path: &str, loader: F) -> Result<ImageEntry>
    where
        F: FnOnce() -> Result<(Array2<f32>, ImageStats, HduHeader)>,
    {
        {
            let mut cache = self.inner.write().unwrap();
            if let Some(entry) = cache.get(path) {
                return Ok(ImageEntry { inner: entry });
            }
        }

        let (arr, stats, header) = loader()?;
        let entry = Arc::new(CachedImage { arr, stats, header: Some(header) });

        {
            let mut cache = self.inner.write().unwrap();
            cache.put(path.to_string(), Arc::clone(&entry));
        }

        Ok(ImageEntry { inner: entry })
    }

    pub fn invalidate(&self, path: &str) {
        let mut cache = self.inner.write().unwrap();
        cache.map.remove(path);
        cache.order.retain(|k| k != path);
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

const DEFAULT_CACHE_SIZE: usize = 8;

pub static GLOBAL_IMAGE_CACHE: LazyLock<ImageCache> =
    LazyLock::new(|| ImageCache::new(DEFAULT_CACHE_SIZE));

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
        let cache = ImageCache::new(4);
        let mut load_count = 0u32;

        let entry1 = cache.get_or_load("file1.fits", || {
            load_count += 1;
            Ok(make_test_entry(100, 100))
        }).unwrap();
        assert_eq!(entry1.arr().dim(), (100, 100));
        assert_eq!(load_count, 1);

        let entry2 = cache.get_or_load("file1.fits", || {
            load_count += 1;
            Ok(make_test_entry(200, 200))
        }).unwrap();
        assert_eq!(entry2.arr().dim(), (100, 100));
        assert_eq!(load_count, 1);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = ImageCache::new(2);

        cache.get_or_load("a", || Ok(make_test_entry(10, 10))).unwrap();
        cache.get_or_load("b", || Ok(make_test_entry(20, 20))).unwrap();
        assert_eq!(cache.len(), 2);

        cache.get_or_load("c", || Ok(make_test_entry(30, 30))).unwrap();
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_lru_access_refreshes() {
        let cache = ImageCache::new(2);

        cache.get_or_load("a", || Ok(make_test_entry(10, 10))).unwrap();
        cache.get_or_load("b", || Ok(make_test_entry(20, 20))).unwrap();

        let _ = cache.get("a");

        cache.get_or_load("c", || Ok(make_test_entry(30, 30))).unwrap();
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_arc_zero_copy() {
        let cache = ImageCache::new(4);
        cache.get_or_load("x", || Ok(make_test_entry(1000, 1000))).unwrap();

        let e1 = cache.get("x").unwrap();
        let e2 = cache.get("x").unwrap();
        let ptr1 = e1.arr().as_ptr();
        let ptr2 = e2.arr().as_ptr();
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_memory_estimate() {
        let cache = ImageCache::new(4);
        cache.get_or_load("a", || Ok(make_test_entry(100, 100))).unwrap();
        assert_eq!(cache.memory_estimate_bytes(), 100 * 100 * 4);
    }

    #[test]
    fn test_invalidate() {
        let cache = ImageCache::new(4);
        cache.get_or_load("a", || Ok(make_test_entry(10, 10))).unwrap();
        assert_eq!(cache.len(), 1);
        cache.invalidate("a");
        assert_eq!(cache.len(), 0);
        assert!(cache.get("a").is_none());
    }

    #[test]
    fn test_get_or_load_full_with_header() {
        let cache = ImageCache::new(4);
        let entry = cache.get_or_load_full("h.fits", || {
            let (arr, stats) = make_test_entry(10, 10);
            let header = crate::types::header::HduHeader {
                cards: vec![("SIMPLE".to_string(), "T".to_string())],
                index: std::collections::HashMap::from([("SIMPLE".to_string(), "T".to_string())]),
            };
            Ok((arr, stats, header))
        }).unwrap();
        assert!(entry.header().is_some());
        assert_eq!(entry.header().unwrap().get("SIMPLE"), Some("T"));
    }
}
