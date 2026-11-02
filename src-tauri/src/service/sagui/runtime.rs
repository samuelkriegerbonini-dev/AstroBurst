use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use crate::infra::progress::ProgressHandle;

pub const SAGUI_COMMIT: &str = "56fd3ea776d7c00e056368498da9412bd58b84bb";
pub const R_VERSION: &str = "4.6.2";
pub const BRIDGE: &str = include_str!("bridge.R");

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub available: bool,
    pub ready: bool,
    pub r_version: Option<String>,
    pub sagui_version: Option<String>,
    pub sagui_commit: Option<String>,
    pub issues: Vec<String>,
}

pub fn command(executable: &str, library: &str, work: &Path) -> Result<Command> {
    let exe = Path::new(executable);
    ensure!(exe.is_absolute() && exe.is_file(), "Choose an existing absolute Rscript executable path");
    let library = Path::new(library).canonicalize().context("The private R library does not exist")?;
    ensure!(library.is_dir(), "The private R library is not a directory");
    fs::write(work.join("bridge.R"), BRIDGE)?;
    let mut command = Command::new(exe);
    command.arg("--vanilla").arg(work.join("bridge.R"));
    command.env("R_LIBS_USER", &library).env("R_LIBS_SITE", &library).env("R_LIBS", &library);
    command.env("OMP_NUM_THREADS", "1").env("OPENBLAS_NUM_THREADS", "1").env("MKL_NUM_THREADS", "1");
    command.current_dir(work).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(command)
}

fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe").args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(unix)]
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
    let _ = child.kill();
    let _ = child.wait();
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) { kill_tree(&mut self.0); }
    }
}

pub fn execute(mut command: Command, work: &Path, progress: &ProgressHandle, timeout: Option<Duration>) -> Result<()> {
    command.stdout(Stdio::from(File::create(work.join("stdout.log"))?));
    command.stderr(Stdio::from(File::create(work.join("stderr.log"))?));
    let mut child = Running(command.spawn().context("Could not start Rscript")?);
    let start = Instant::now();
    loop {
        if progress.is_cancelled() { bail!("SAGUI cancelled; results were not published"); }
        if timeout.is_some_and(|limit| start.elapsed() > limit) { bail!("Rscript runtime verification timed out"); }
        if let Some(status) = child.0.try_wait()? {
            ensure!(status.success(), "Rscript failed ({status}); see {}", work.join("stderr.log").display());
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn parse_runtime(text: &str) -> Result<RuntimeInfo> {
    let line = text.lines().find_map(|line| line.strip_prefix("ASTROBURST_R\t")).context("Rscript did not return a runtime identity")?;
    let fields: Vec<_> = line.split('\t').collect();
    ensure!(fields.len() == 3, "Malformed R runtime identity");
    let mut issues: Vec<String> = text.lines().filter_map(|line| line.strip_prefix("ASTROBURST_ISSUE\t").map(str::to_string)).collect();
    if fields[0] != R_VERSION { issues.push(format!("Expected R {R_VERSION}")); }
    if fields[2] != SAGUI_COMMIT { issues.push("Expected the pinned SAGUI commit".into()); }
    Ok(RuntimeInfo { available: true, ready: issues.is_empty(), r_version: Some(fields[0].into()), sagui_version: Some(fields[1].into()), sagui_commit: Some(fields[2].into()), issues })
}

pub fn inspect(executable: &str, library: &str) -> RuntimeInfo {
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    inspect_with_progress(executable, library, &progress)
}

pub fn inspect_with_progress(executable: &str, library: &str, progress: &ProgressHandle) -> RuntimeInfo {
    let result = (|| -> Result<RuntimeInfo> {
        let temp = tempfile::tempdir()?;
        let work = temp.path().canonicalize()?;
        let mut cmd = command(executable, library, &work)?;
        cmd.arg("inspect").arg(Path::new(library).canonicalize()?).arg(&work);
        execute(cmd, &work, progress, Some(Duration::from_secs(30)))?;
        parse_runtime(&fs::read_to_string(work.join("stdout.log"))?)
    })();
    result.unwrap_or_else(|error| RuntimeInfo { available: false, ready: false, r_version: None, sagui_version: None, sagui_commit: None, issues: vec![format!("{error:#}")] })
}
