use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

pub const INSTANCE_LOCK_FILE: &str = "instance.lock";

#[derive(Debug)]
pub struct HeldLock {
    _file: File,
    path: PathBuf,
    remove_on_drop: bool,
}

impl HeldLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn try_lock_at(path: &Path, remove_on_drop: bool) -> io::Result<Option<HeldLock>> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(HeldLock { _file: file, path: path.to_path_buf(), remove_on_drop })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

pub fn try_lock_file(path: &Path) -> io::Result<Option<HeldLock>> {
    try_lock_at(path, false)
}

pub fn try_lock_temporary_file(path: &Path) -> io::Result<Option<HeldLock>> {
    try_lock_at(path, true)
}

pub fn try_instance_lock(app_data_dir: &Path) -> io::Result<Option<HeldLock>> {
    try_lock_file(&app_data_dir.join(INSTANCE_LOCK_FILE))
}

#[cfg(test)]
mod tests {
    use super::{try_instance_lock, try_lock_temporary_file};

    #[test]
    fn a_second_holder_of_the_instance_lock_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let first = try_instance_lock(dir.path()).unwrap().expect("the first instance gets the lock");
        assert!(try_instance_lock(dir.path()).unwrap().is_none(), "two instances held the lock at once");
        drop(first);
        assert!(try_instance_lock(dir.path()).unwrap().is_some());
    }

    #[test]
    fn a_temporary_lock_file_is_removed_when_its_holder_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin.part.lock");
        let held = try_lock_temporary_file(&path).unwrap().unwrap();
        assert!(try_lock_temporary_file(&path).unwrap().is_none());
        drop(held);
        assert!(!path.exists(), "the lock file was left behind");
    }
}
