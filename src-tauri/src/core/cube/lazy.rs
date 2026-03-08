use std::collections::HashMap;

use ndarray::Array2;

#[derive(Debug, Clone)]
pub struct CubeGeometry {
    pub naxis1: usize,
    pub naxis2: usize,
    pub naxis3: usize,
    pub bitpix: i64,
    pub bytes_per_pixel: usize,
    pub bzero: f64,
    pub bscale: f64,
    pub data_offset: usize,
    pub frame_bytes: usize,
}

pub struct LruFrameCache {
    entries: HashMap<usize, CacheEntry>,
    max_entries: usize,
    access_counter: u64,
}

struct CacheEntry {
    frame: Array2<f32>,
    last_access: u64,
}

impl LruFrameCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            access_counter: 0,
        }
    }

    pub fn get(&mut self, frame_idx: usize) -> Option<Array2<f32>> {
        if let Some(entry) = self.entries.get_mut(&frame_idx) {
            self.access_counter += 1;
            entry.last_access = self.access_counter;
            Some(entry.frame.clone())
        } else {
            None
        }
    }

    pub fn insert(&mut self, frame_idx: usize, frame: Array2<f32>) {
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&frame_idx) {
            if let Some((&evict_key, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
            {
                self.entries.remove(&evict_key);
            }
        }
        self.access_counter += 1;
        self.entries.insert(
            frame_idx,
            CacheEntry {
                frame,
                last_access: self.access_counter,
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.access_counter = 0;
    }
}

pub use crate::core::cube::eager::GlobalCubeStats;

pub fn normalize_frame_with_stats(data: &Array2<f32>, stats: &GlobalCubeStats) -> Array2<f32> {
    let alpha: f32 = 10.0;
    let inv_sigma_alpha = alpha / stats.sigma;

    data.mapv(|v| {
        if !v.is_finite() {
            return 0.0;
        }
        let clamped = v.clamp(stats.low, stats.high);
        let scaled = inv_sigma_alpha * (clamped - stats.median);
        scaled.asinh()
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LazyCubeResult {
    pub dimensions: [usize; 3],
    pub collapsed_path: String,
    pub collapsed_median_path: String,
    pub frames_dir: String,
    pub frame_count: usize,
    pub total_frames: usize,
    pub center_spectrum: Vec<f32>,
    pub wavelengths: Option<Vec<f64>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_cache() {
        let mut cache = LruFrameCache::new(2);
        let frame1 = Array2::<f32>::zeros((3, 3));
        let frame2 = Array2::<f32>::ones((3, 3));
        let frame3 = Array2::<f32>::from_elem((3, 3), 2.0);

        cache.insert(0, frame1);
        cache.insert(1, frame2);
        cache.insert(2, frame3);

        assert!(cache.get(0).is_none());
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn test_normalize_frame_with_stats() {
        let frame = Array2::from_shape_vec((2, 2), vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        let stats = GlobalCubeStats {
            median: 2.5,
            sigma: 1.0,
            low: 1.0,
            high: 4.0,
        };
        let normalized = normalize_frame_with_stats(&frame, &stats);
        assert_eq!(normalized.dim(), (2, 2));
        for &v in normalized.iter() {
            assert!(v.is_finite());
        }
    }
}
