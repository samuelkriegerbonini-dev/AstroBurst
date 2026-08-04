use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use anyhow::Result;

use crate::core::cube::lazy::LazyCube;

const MAX_OPEN_CUBES: usize = 4;

type FileStamp = (u64, Option<SystemTime>);

struct OpenCube {
    cube: Arc<LazyCube>,
    stamp: FileStamp,
    last_access: u64,
}

struct CubeCacheInner {
    map: HashMap<String, OpenCube>,
    counter: u64,
}

pub struct CubeCache {
    inner: Mutex<CubeCacheInner>,
}

pub static GLOBAL_CUBE_CACHE: LazyLock<CubeCache> = LazyLock::new(|| CubeCache {
    inner: Mutex::new(CubeCacheInner {
        map: HashMap::new(),
        counter: 0,
    }),
});

fn stamp_of(path: &str) -> Option<FileStamp> {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

impl CubeCache {
    pub fn get_or_open(&self, path: &str) -> Result<Arc<LazyCube>> {
        let stamp = stamp_of(path);

        if let Some(st) = &stamp {
            let mut g = self.inner.lock().unwrap();
            if let Some(entry) = g.map.get(path) {
                if &entry.stamp == st {
                    let cube = Arc::clone(&entry.cube);
                    g.counter += 1;
                    let counter = g.counter;
                    g.map.get_mut(path).unwrap().last_access = counter;
                    return Ok(cube);
                }
            }
            g.map.remove(path);
        }

        let cube = Arc::new(LazyCube::open(path)?);

        if let Some(st) = stamp {
            let mut g = self.inner.lock().unwrap();
            if let Some(entry) = g.map.get(path) {
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
                path.to_string(),
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
        let mut g = self.inner.lock().unwrap();
        g.map.remove(path);
    }
}
