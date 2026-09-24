use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::SystemTime;

use anyhow::Result;

use crate::core::cube::lazy::LazyCube;
use crate::types::image_ref::path_key;
use crate::types::{ImageRef, PlaneSelector};

const MAX_OPEN_CUBES: usize = 4;

type FileStamp = (u64, Option<SystemTime>);
type CubeKey = (String, PlaneSelector);

struct OpenCube {
    cube: Arc<LazyCube>,
    stamp: FileStamp,
    last_access: u64,
}

struct CubeCacheInner {
    map: HashMap<CubeKey, OpenCube>,
    counter: u64,
}

pub struct CubeCache {
    inner: Mutex<CubeCacheInner>,
}

pub static GLOBAL_CUBE_CACHE: LazyLock<CubeCache> = LazyLock::new(CubeCache::new);

fn stamp_of(path: &str) -> Option<FileStamp> {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

fn cube_key(reference: &ImageRef) -> CubeKey {
    (path_key(&reference.path), reference.plane.clone())
}

impl CubeCache {
    fn new() -> CubeCache {
        CubeCache {
            inner: Mutex::new(CubeCacheInner {
                map: HashMap::new(),
                counter: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CubeCacheInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get_or_open(&self, path: &str) -> Result<Arc<LazyCube>> {
        let reference = ImageRef::parse(path);
        let key = cube_key(&reference);
        let stamp = stamp_of(&reference.path);

        if let Some(st) = &stamp {
            let mut g = self.lock();
            g.counter += 1;
            let counter = g.counter;
            if let Some(entry) = g.map.get_mut(&key) {
                if &entry.stamp == st {
                    entry.last_access = counter;
                    return Ok(Arc::clone(&entry.cube));
                }
            }
            g.map.remove(&key);
        }

        let cube = Arc::new(LazyCube::open(path)?);

        if let Some(st) = stamp {
            let mut g = self.lock();
            if let Some(entry) = g.map.get(&key) {
                if entry.stamp == st {
                    return Ok(Arc::clone(&entry.cube));
                }
            }
            while g.map.len() >= MAX_OPEN_CUBES {
                let victim = g
                    .map
                    .iter()
                    .min_by_key(|(_, e)| e.last_access)
                    .map(|(k, _)| k.clone());
                match victim {
                    Some(k) => {
                        g.map.remove(&k);
                    }
                    None => break,
                }
            }
            g.counter += 1;
            let counter = g.counter;
            g.map.insert(
                key,
                OpenCube {
                    cube: Arc::clone(&cube),
                    stamp: st,
                    last_access: counter,
                },
            );
        }

        Ok(cube)
    }

    pub fn invalidate(&self, path: &str) {
        let source = path_key(&ImageRef::parse(path).path);
        self.lock().map.retain(|(cached, _), _| *cached != source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cube::lazy::test_support::{write_line_cube, write_mef_cube, LINE_CUBE_DEPTH};

    #[test]
    fn a_plane_ref_opens_through_the_cache_keyed_by_its_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.fits");
        write_line_cube(&path, 0.0);
        let source = path.to_str().unwrap().to_string();
        let cache = CubeCache::new();

        let by_ref = cache.get_or_open(&format!("{}#hdu=0", source)).expect("#hdu=0 must open the cube");
        assert_eq!(by_ref.geometry.naxis3, LINE_CUBE_DEPTH);
        let again = cache.get_or_open(&format!("{}#hdu=0", source)).unwrap();
        assert!(Arc::ptr_eq(&by_ref, &again));
        let auto = cache.get_or_open(&source).unwrap();
        assert_eq!(auto.hdu_index, 0);

        cache.invalidate(&source.replace('\\', "/"));
        assert!(cache.lock().map.is_empty(), "a differently spelled path left cube planes cached");
        let reopened = cache.get_or_open(&format!("{}#hdu=0", source)).unwrap();
        assert!(!Arc::ptr_eq(&by_ref, &reopened));
    }

    #[test]
    fn each_hdu_of_a_source_is_its_own_cube() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mef.fits");
        write_mef_cube(&path, 3, 3, 4, &[("SCI", 1.0), ("ERR", 90.0)]);
        let source = path.to_str().unwrap();
        let cache = CubeCache::new();
        let sci = cache.get_or_open(&format!("{}#hdu=1", source)).unwrap();
        let err = cache.get_or_open(&format!("{}#hdu=2", source)).unwrap();
        assert_eq!(sci.get_frame(0).unwrap()[[0, 0]], 1.0);
        assert_eq!(err.get_frame(0).unwrap()[[0, 0]], 90.0);
        cache.invalidate(&format!("{}#hdu=1", source));
        assert!(cache.lock().map.is_empty());
    }

    #[test]
    fn the_global_cache_accepts_the_ref_the_frontend_sends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("global_ref.fits");
        write_line_cube(&path, 0.0);
        let key = format!("{}#hdu=0", path.to_str().unwrap());
        assert!(GLOBAL_CUBE_CACHE.get_or_open(&key).is_ok());
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }
}
