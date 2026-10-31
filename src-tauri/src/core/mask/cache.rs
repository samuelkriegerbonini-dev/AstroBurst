use std::sync::{Arc, LazyLock};

use ndarray::Array2;

use crate::core::history::id::canonical_ref;
use crate::core::history::stamp::{file_stamp, utc_timestamp};
use crate::core::mask::{rasterize, recipe_hash, MaskError, MaskMods, MaskRecipe};
use crate::infra::cache::ImageCache;
use crate::types::image::ImageStats;
use crate::types::image_ref::ImageRef;

pub const MASK_CACHE_MAX_ENTRIES: usize = 16;
pub const MASK_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;
const MASK_KEY_PREFIX: &str = "__mask__";

pub static MASK_CACHE: LazyLock<MaskCache> =
    LazyLock::new(|| MaskCache::new(MASK_CACHE_MAX_ENTRIES, MASK_CACHE_MAX_BYTES));

pub struct MaskCache {
    rasters: ImageCache,
}

impl MaskCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self { rasters: ImageCache::new(max_entries, max_bytes) }
    }

    pub fn raster(
        &self,
        source: &str,
        recipe: &MaskRecipe,
        mods: &MaskMods,
        planes: &[&Array2<f32>],
    ) -> Result<Arc<Array2<f32>>, MaskError> {
        self.get_or_insert(&entry_key(source, recipe, mods), || rasterize(recipe, mods, planes))
    }

    pub fn keep(&self, key: &str, raster: Arc<Array2<f32>>) {
        self.rasters.insert_synthetic(key, raster, ImageStats::default());
    }

    pub fn len(&self) -> usize {
        self.rasters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn memory_bytes(&self) -> usize {
        self.rasters.memory_estimate_bytes()
    }

    fn get_or_insert(
        &self,
        key: &str,
        rasterize: impl FnOnce() -> Result<Array2<f32>, MaskError>,
    ) -> Result<Arc<Array2<f32>>, MaskError> {
        if let Some(entry) = self.rasters.get(key) {
            return Ok(entry.data_arc());
        }
        let raster = Arc::new(rasterize()?);
        self.rasters.insert_synthetic(key, Arc::clone(&raster), ImageStats::default());
        Ok(raster)
    }
}

pub fn entry_key(source: &str, recipe: &MaskRecipe, mods: &MaskMods) -> String {
    let stamp = match recipe {
        MaskRecipe::File { path, .. } => file_token(path),
        _ => String::new(),
    };
    format!("{MASK_KEY_PREFIX}{:016x}{stamp}#{source}", recipe_hash(recipe, mods))
}

pub fn file_source(path: &str) -> Option<String> {
    let token = file_token(path);
    (!token.is_empty()).then(|| format!("{}{token}", canonical_ref(path)))
}

fn file_token(path: &str) -> String {
    std::fs::metadata(ImageRef::parse(path).path)
        .map(|meta| {
            let (len, modified) = file_stamp(&meta);
            format!("@{len}@{}", modified.map(utc_timestamp).unwrap_or_default())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::core::mask::MarkMode;
    use crate::infra::fits::writer::write_fits_mono;

    fn ramp(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32)
    }

    fn raw_key(name: &str) -> String {
        format!("{MASK_KEY_PREFIX}{name}")
    }

    #[test]
    fn raster_cache_evicts_above_512_mib_and_rehits_by_hash() {
        assert_eq!(MASK_CACHE_MAX_BYTES, 512 * 1024 * 1024);
        assert_eq!(MASK_CACHE_MAX_ENTRIES, 16);
        assert!(MASK_CACHE.is_empty() || MASK_CACHE.memory_bytes() <= MASK_CACHE_MAX_BYTES);

        let cache = MaskCache::new(MASK_CACHE_MAX_ENTRIES, MASK_CACHE_MAX_BYTES);
        let plane = ramp(64, 64);
        let recipe = MaskRecipe::Range { lo: 100.0, hi: 2000.0, fuzz: 50.0 };
        let plain = MaskMods::default();
        let first = cache.raster("n0001.fits", &recipe, &plain, &[&plane]).unwrap();
        let again = cache.raster("n0001.fits", &recipe, &plain, &[&plane]).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "one recipe, mods and source was rasterised twice");
        assert_eq!(cache.len(), 1);

        let inverted = cache.raster("n0001.fits", &recipe, &MaskMods { invert: true, ..plain }, &[&plane]).unwrap();
        assert!(!Arc::ptr_eq(&first, &inverted), "a change of mods re-hit the old raster");
        assert!(first.iter().zip(inverted.iter()).all(|(m, i)| m + i == 1.0));
        let elsewhere = cache.raster("n0002.fits", &recipe, &plain, &[&plane]).unwrap();
        assert!(!Arc::ptr_eq(&first, &elsewhere), "the raster of another input was served");
        assert_eq!(cache.len(), 3);

        let large = MaskCache::new(MASK_CACHE_MAX_ENTRIES, MASK_CACHE_MAX_BYTES);
        let (rows, cols) = (8192, 8193);
        let raster_bytes = rows * cols * std::mem::size_of::<f32>();
        assert!(raster_bytes < MASK_CACHE_MAX_BYTES && 2 * raster_bytes > MASK_CACHE_MAX_BYTES);
        let calls = Cell::new(0);
        let zeros = || {
            calls.set(calls.get() + 1);
            Ok(Array2::zeros((rows, cols)))
        };
        large.get_or_insert(&raw_key("a"), &zeros).unwrap();
        large.get_or_insert(&raw_key("a"), &zeros).unwrap();
        assert_eq!(calls.get(), 1, "a cached raster was rasterised again");
        assert_eq!(large.memory_bytes(), raster_bytes);

        large.get_or_insert(&raw_key("b"), &zeros).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(large.len(), 1, "two rasters above 512 MiB together were both kept");
        assert_eq!(large.memory_bytes(), raster_bytes);
        large.get_or_insert(&raw_key("b"), &zeros).unwrap();
        assert_eq!(calls.get(), 2, "the newest raster was not served from the cache");
        large.get_or_insert(&raw_key("a"), &zeros).unwrap();
        assert_eq!(calls.get(), 3, "the evicted raster was served instead of rebuilt");
        assert!(large.memory_bytes() <= MASK_CACHE_MAX_BYTES);
    }

    #[test]
    fn raster_cache_misses_when_a_file_mask_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mask.fits");
        let path_text = path.to_str().unwrap();
        write_fits_mono(path_text, &Array2::from_elem((10, 12), 1.0), None).unwrap();
        let recipe = MaskRecipe::File { path: path_text.to_string(), mark: MarkMode::NonZero, lo: None, hi: None };
        let cache = MaskCache::new(4, 1 << 20);
        let plane = ramp(10, 12);
        let plain = MaskMods::default();

        let first = cache.raster("m42.fits", &recipe, &plain, &[&plane]).unwrap();
        assert_eq!(first.dim(), (10, 12));
        assert!(first.iter().all(|&m| m == 1.0));
        assert!(Arc::ptr_eq(&first, &cache.raster("m42.fits", &recipe, &plain, &[&plane]).unwrap()));

        write_fits_mono(path_text, &Array2::from_elem((40, 40), 0.0), None).unwrap();
        let rewritten = cache.raster("m42.fits", &recipe, &plain, &[&plane]).unwrap();
        assert_eq!(rewritten.dim(), (40, 40), "a rewritten mask file kept serving its old raster");
        assert!(rewritten.iter().all(|&m| m == 0.0));
    }
}
