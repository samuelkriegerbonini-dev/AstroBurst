use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::SystemTime;

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::series::read_set;
use super::TsoSet;
use crate::types::image_ref::path_key;

pub const TSO_CACHE_SETS: usize = 2;

const SET_ID_LENGTH: usize = 16;

type FileStamp = (String, u64, Option<SystemTime>);

struct CachedSet {
    stamps: Vec<FileStamp>,
    set: Arc<TsoSet>,
}

pub struct TsoCache {
    capacity: usize,
    sets: Mutex<Vec<CachedSet>>,
}

static PROCESS_CACHE: LazyLock<TsoCache> = LazyLock::new(|| TsoCache::new(TSO_CACHE_SETS));

fn file_stamps(paths: &[String]) -> Option<Vec<FileStamp>> {
    let mut stamps = paths
        .iter()
        .map(|path| {
            let metadata = std::fs::metadata(path).ok()?;
            Some((path_key(path), metadata.len(), metadata.modified().ok()))
        })
        .collect::<Option<Vec<FileStamp>>>()?;
    stamps.sort_by(|(key, ..), (other, ..)| key.cmp(other));
    stamps.dedup_by(|(key, ..), (kept, ..)| key == kept);
    Some(stamps)
}

fn same_files(stamps: &[FileStamp], others: &[FileStamp]) -> bool {
    stamps.len() == others.len() && stamps.iter().zip(others).all(|((key, ..), (other, ..))| key == other)
}

impl TsoCache {
    pub fn new(capacity: usize) -> TsoCache {
        TsoCache { capacity, sets: Mutex::new(Vec::new()) }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<CachedSet>> {
        self.sets.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn open(&self, paths: &[String]) -> Result<Arc<TsoSet>> {
        let stamps = file_stamps(paths);
        if let Some(stamps) = &stamps {
            let mut sets = self.lock();
            if let Some(position) = sets.iter().position(|cached| cached.stamps == *stamps) {
                let hit = sets.remove(position);
                let set = Arc::clone(&hit.set);
                sets.push(hit);
                return Ok(set);
            }
        }
        let set = Arc::new(read_set(paths)?);
        if let Some(stamps) = stamps {
            let mut sets = self.lock();
            sets.retain(|cached| !same_files(&cached.stamps, &stamps));
            sets.push(CachedSet { stamps, set: Arc::clone(&set) });
            let beyond_capacity = sets.len().saturating_sub(self.capacity);
            sets.drain(..beyond_capacity);
        }
        Ok(set)
    }
}

pub fn open_tso(paths: &[String]) -> Result<Arc<TsoSet>> {
    PROCESS_CACHE.open(paths)
}

pub fn set_id(set: &TsoSet) -> String {
    let mut hashes: Vec<&str> = set.files.iter().map(|file| file.sha256.as_str()).collect();
    hashes.sort_unstable();
    let mut hasher = Sha256::new();
    for hash in hashes {
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    let digest: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    digest[..SET_ID_LENGTH].to_string()
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;
    use std::path::Path;
    use std::time::Duration;

    use super::*;
    use crate::core::analysis::tso::series::build_set;
    use crate::core::analysis::tso::series::test_support::{file_of, table_of};
    use crate::core::analysis::tso::DATAMODEL_ROWS;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, rows_hdu, series_primary, write_series_file, SynthTable};
    use crate::infra::fits::x1dints::X1dIntsFile;

    fn series_file(dir: &Path, name: &str, integrations: RangeInclusive<i32>) -> String {
        let primary = series_primary(DATAMODEL_ROWS, &[("EXPSTART", "60000.0".to_string()), ("NINTS", "4".to_string())]);
        let rows = integrations.map(|number| ramp_row(number, 60000.0 + 0.1 * f64::from(number), 4)).collect();
        let table = SynthTable { extver: 1, detector: Some("NRS1"), sporder: Some(-1), flux_unit: Some("Jy"), rows };
        write_series_file(dir, name, primary, vec![rows_hdu(&table)]).to_str().unwrap().to_string()
    }

    #[test]
    fn a_set_is_read_once_and_given_back_whatever_the_order_of_the_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = series_file(dir.path(), "a-seg001_x1dints.fits", 1..=2);
        let b = series_file(dir.path(), "a-seg002_x1dints.fits", 3..=4);
        let cache = TsoCache::new(2);
        assert_eq!(cache.len(), 0);

        let first = cache.open(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(first.series.len(), 1);
        assert_eq!(first.series[0].int_num, [1, 2, 3, 4]);
        let reordered = cache.open(&[b.clone(), a.clone(), b.clone()]).unwrap();
        assert!(Arc::ptr_eq(&first, &reordered), "the same files in another order were read again");
        let other_slashes = cache.open(&[a.replace('\\', "/"), b.clone()]).unwrap();
        assert!(Arc::ptr_eq(&first, &other_slashes), "the same files spelled with other slashes were read again");
        assert_eq!(cache.len(), 1);

        let only_a = cache.open(&[a]).unwrap();
        assert!(!Arc::ptr_eq(&first, &only_a));
        assert_eq!(only_a.series[0].int_num, [1, 2]);
        assert_eq!(cache.len(), 2);
    }

    fn size_and_time(path: &str) -> (u64, SystemTime) {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.len(), metadata.modified().unwrap())
    }

    fn set_time(path: &str, time: SystemTime) {
        std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn a_rewritten_file_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let a = series_file(dir.path(), "a_x1dints.fits", 1..=2);
        let cache = TsoCache::new(2);
        let before = cache.open(&[a.clone()]).unwrap();
        assert_eq!(before.series[0].n_integrations, 2);
        let (size, written) = size_and_time(&a);

        assert_eq!(series_file(dir.path(), "a_x1dints.fits", 1..=3), a);
        let later = written + Duration::from_secs(2);
        set_time(&a, later);
        assert_eq!(size_and_time(&a), (size, later), "three rows fill the same FITS block as two");
        let after = cache.open(&[a.clone()]).unwrap();
        assert!(!Arc::ptr_eq(&before, &after), "a file whose time changed was served from the cache");
        assert_eq!(after.series[0].n_integrations, 3);
        assert_eq!(cache.len(), 1, "the set of the old file stayed in the cache");
        assert!(Arc::ptr_eq(&after, &cache.open(&[a.clone()]).unwrap()));

        series_file(dir.path(), "a_x1dints.fits", 1..=17);
        set_time(&a, later);
        let (grown, time) = size_and_time(&a);
        assert!(grown > size && time == later, "{grown} bytes after {size} bytes");
        let longer = cache.open(&[a]).unwrap();
        assert!(!Arc::ptr_eq(&after, &longer), "a file whose size changed was served from the cache");
        assert_eq!(longer.series[0].n_integrations, 17);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_oldest_set_leaves_when_a_third_arrives_and_failures_are_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let x = [series_file(dir.path(), "x_x1dints.fits", 1..=2)];
        let y = [series_file(dir.path(), "y_x1dints.fits", 1..=2)];
        let z = [series_file(dir.path(), "z_x1dints.fits", 1..=2)];
        let cache = TsoCache::new(2);
        let first_x = cache.open(&x).unwrap();
        let first_y = cache.open(&y).unwrap();
        let first_z = cache.open(&z).unwrap();
        assert_eq!(cache.len(), 2);

        assert!(Arc::ptr_eq(&first_y, &cache.open(&y).unwrap()), "y was dropped while only x was older");
        assert!(!Arc::ptr_eq(&first_x, &cache.open(&x).unwrap()), "x stayed beyond the capacity");
        assert_eq!(cache.len(), 2);
        assert!(!Arc::ptr_eq(&first_z, &cache.open(&z).unwrap()), "a hit on y did not make it newer than z");
        assert_eq!(cache.len(), 2);

        let absent = dir.path().join("absent_x1dints.fits").to_str().unwrap().to_string();
        let failure = format!("{:#}", cache.open(&[absent]).unwrap_err());
        assert!(failure.contains("absent_x1dints.fits"), "{failure}");
        assert_eq!(cache.len(), 2);
        let not_a_series = dir.path().join("text_x1dints.fits");
        std::fs::write(&not_a_series, b"not a FITS file").unwrap();
        assert!(cache.open(&[not_a_series.to_str().unwrap().to_string()]).is_err());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.open(&[]).unwrap_err().to_string(), "no file given: pass at least one x1dints file");
        assert_eq!(cache.len(), 2);
    }

    fn hashed_file(name: &str, detector: &str, sha256: String) -> X1dIntsFile {
        let mut file = file_of(name, 2, Some(1), Some(60000.0), vec![table_of(detector, Some(-1), 1, &[ramp_row(1, 60000.0, 4)])]);
        file.file.sha256 = sha256;
        file
    }

    #[test]
    fn the_set_id_depends_on_the_content_of_the_files_only() {
        let a = || hashed_file("a_x1dints.fits", "NRS1", "a".repeat(64));
        let b = || hashed_file("b_x1dints.fits", "NRS2", "b".repeat(64));
        assert_eq!(set_id(&build_set(vec![a(), b()]).unwrap()), "913f9338fb6c1725");
        assert_eq!(set_id(&build_set(vec![b(), a()]).unwrap()), "913f9338fb6c1725");
        let mut renamed = build_set(vec![a(), b()]).unwrap();
        renamed.files[0].path = "D:/elsewhere/renamed_x1dints.fits".into();
        renamed.files[0].file_name = "renamed_x1dints.fits".into();
        assert_eq!(set_id(&renamed), "913f9338fb6c1725");

        let zeros = hashed_file("zeros_x1dints.fits", "NRS1", "0".repeat(64));
        assert_eq!(set_id(&build_set(vec![zeros]).unwrap()), "827d096d92f3deea");
    }
}
