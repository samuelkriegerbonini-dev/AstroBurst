use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const LOG_STEM: &str = "astroburst";
const LOG_EXTENSION: &str = "log";
const KEPT_LOGS: usize = 3;
const DEFAULT_FILTER: &str = "warn,astroburst_lib=info";

fn log_path(dir: &Path, generation: usize) -> PathBuf {
    match generation {
        0 => dir.join(format!("{LOG_STEM}.{LOG_EXTENSION}")),
        n => dir.join(format!("{LOG_STEM}.{n}.{LOG_EXTENSION}")),
    }
}

fn rotate(dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let oldest = log_path(dir, KEPT_LOGS - 1);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for generation in (0..KEPT_LOGS - 1).rev() {
        let path = log_path(dir, generation);
        if path.exists() {
            fs::rename(&path, log_path(dir, generation + 1))?;
        }
    }
    Ok(log_path(dir, 0))
}

pub fn install(dir: &Path) -> anyhow::Result<PathBuf> {
    let path = rotate(dir)?;
    let file = fs::File::create(&path)?;
    env_logger::Builder::new()
        .parse_filters(DEFAULT_FILTER)
        .parse_env(env_logger::Env::default())
        .format_timestamp_millis()
        .write_style(env_logger::WriteStyle::Never)
        .target(env_logger::Target::Pipe(Box::new(file)))
        .try_init()?;
    log::info!("AstroBurst {} started", env!("CARGO_PKG_VERSION"));
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::rotate;

    #[test]
    fn each_start_keeps_the_two_previous_logs_and_drops_older_ones() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        for run in ["first", "second", "third", "fourth"] {
            let current = rotate(&logs).unwrap();
            assert_eq!(current, logs.join("astroburst.log"));
            assert!(!current.exists(), "the log before the {run} run was not moved aside");
            std::fs::write(&current, run).unwrap();
        }
        let read = |name: &str| std::fs::read_to_string(logs.join(name)).ok();
        assert_eq!(read("astroburst.log").as_deref(), Some("fourth"));
        assert_eq!(read("astroburst.1.log").as_deref(), Some("third"));
        assert_eq!(read("astroburst.2.log").as_deref(), Some("second"));
        assert_eq!(std::fs::read_dir(&logs).unwrap().count(), 3);
    }
}
