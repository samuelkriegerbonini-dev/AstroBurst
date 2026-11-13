use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use crate::infra::paths::plain;
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

#[cfg(windows)]
struct KillOnClose(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for KillOnClose {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn kill_on_close(child: &Child) -> Option<KillOnClose> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() { return None; }
    let job = KillOnClose(handle);
    let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION { BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION { LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, ..Default::default() }, ..Default::default() };
    let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    let limited = unsafe { SetInformationJobObject(handle, JobObjectExtendedLimitInformation, std::ptr::from_ref(&limits).cast(), size) } != 0;
    let assigned = limited && unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } != 0;
    assigned.then_some(job)
}

#[cfg(windows)]
fn spawn_suspended(command: &mut Command) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED).spawn()
}

#[cfg(windows)]
fn resume(child: &Child) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error()); }
    let mut entry = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut resumed = false;
    let mut listed = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while listed {
        if entry.th32OwnerProcessID == child.id() {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                let previous = unsafe { ResumeThread(thread) };
                resumed |= previous != u32::MAX && previous > 0;
                unsafe { CloseHandle(thread) };
            }
        }
        listed = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    if resumed { Ok(()) } else { Err(std::io::Error::other(format!("no suspended thread of process {} could be resumed", child.id()))) }
}

#[cfg(windows)]
fn taskkill_path() -> std::path::PathBuf {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() { return std::path::PathBuf::from("taskkill.exe"); }
    std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length])).join("taskkill.exe")
}

fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    if matches!(child.try_wait(), Ok(None)) {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(taskkill_path()).args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(unix)]
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
    let _ = child.kill();
    let _ = child.wait();
}

struct Running {
    child: Child,
    #[cfg(windows)]
    job: Option<KillOnClose>,
}

impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) { kill_tree(&mut self.child); }
        #[cfg(windows)]
        drop(self.job.take());
    }
}

const LIVENESS_INTERVAL: Duration = Duration::from_secs(2);
const STDERR_TAIL_LINES: usize = 8;

fn marked<'a>(stdout: &'a str, prefix: &str) -> Vec<&'a str> {
    stdout.lines().filter_map(|line| line.strip_prefix(prefix)).map(str::trim_end).filter(|line| !line.is_empty()).collect()
}

pub fn failure_message(stdout: &str, stderr: &str, status: &str, log: &Path) -> String {
    let refused = marked(stdout, "ASTROBURST_REFUSED\t");
    if !refused.is_empty() { return refused.join("; "); }
    let errors = marked(stdout, "ASTROBURST_ERROR\t");
    let tail = if errors.is_empty() {
        let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        lines[lines.len().saturating_sub(STDERR_TAIL_LINES)..].join(" | ")
    } else {
        errors.join("; ")
    };
    let log = plain(log).display().to_string();
    if tail.is_empty() { format!("Rscript failed ({status}); full log: {log}") } else { format!("Rscript failed ({status}): {tail}; full log: {log}") }
}

fn last_console_line(log: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = File::open(log).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(16 * 1024))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with("ASTROBURST_")).last().map(|line| line.chars().take(160).collect())
}

pub fn execute(command: Command, work: &Path, progress: &ProgressHandle, timeout: Option<Duration>) -> Result<()> {
    execute_labelled("Rscript", command, work, progress, timeout)
}

pub fn execute_labelled(label: &str, mut command: Command, work: &Path, progress: &ProgressHandle, timeout: Option<Duration>) -> Result<()> {
    command.stdout(Stdio::from(File::create(work.join("stdout.log"))?));
    command.stderr(Stdio::from(File::create(work.join("stderr.log"))?));
    #[cfg(windows)]
    let child = spawn_suspended(&mut command).context("Could not start Rscript")?;
    #[cfg(not(windows))]
    let child = command.spawn().context("Could not start Rscript")?;
    #[cfg(windows)]
    let job = kill_on_close(&child);
    let mut child = Running { child, #[cfg(windows)] job };
    #[cfg(windows)]
    resume(&child.child).context("Could not start Rscript")?;
    let start = Instant::now();
    let mut reported = start;
    loop {
        if progress.is_cancelled() { bail!("SAGUI cancelled; results were not published"); }
        if timeout.is_some_and(|limit| start.elapsed() > limit) { bail!("Rscript runtime verification timed out"); }
        if let Some(status) = child.child.try_wait()? {
            if status.success() { return Ok(()); }
            let read = |name: &str| String::from_utf8_lossy(&fs::read(work.join(name)).unwrap_or_default()).into_owned();
            bail!(failure_message(&read("stdout.log"), &read("stderr.log"), &status.to_string(), &work.join("stderr.log")));
        }
        if reported.elapsed() >= LIVENESS_INTERVAL {
            reported = Instant::now();
            let secs = start.elapsed().as_secs();
            let stage = match last_console_line(&work.join("stderr.log")) {
                Some(line) => format!("{label} - R running {secs} s - {line}"),
                None => format!("{label} - R running {secs} s"),
            };
            progress.emit_stage(&stage);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn parse_runtime(text: &str) -> Result<RuntimeInfo> {
    let line = text.lines().find_map(|line| line.strip_prefix("ASTROBURST_R\t")).context("Rscript did not return a runtime identity")?;
    let fields: Vec<_> = line.split('\t').collect();
    ensure!(fields.len() == 3, "Malformed R runtime identity");
    let mut issues: Vec<String> = text.lines().filter_map(|line| line.strip_prefix("ASTROBURST_ISSUE\t").map(str::to_string)).collect();
    if fields[0] != R_VERSION && !issues.iter().any(|issue| issue.starts_with("Expected R ")) { issues.push(format!("Expected R {R_VERSION}; found {}", fields[0])); }
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
        execute_labelled("Checking the R runtime", cmd, &work, progress, Some(Duration::from_secs(30)))?;
        parse_runtime(&fs::read_to_string(work.join("stdout.log"))?)
    })();
    result.unwrap_or_else(|error| RuntimeInfo { available: false, ready: false, r_version: None, sagui_version: None, sagui_commit: None, issues: vec![format!("{error:#}")] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use crate::infra::progress::{ProgressPayload, ProgressSink};

    #[test]
    fn failure_message_prefers_astroburst_error_lines_then_the_stderr_tail() {
        let log = Path::new("sagui-run").join("stderr.log");
        let shown = log.display();
        let identity = "ASTROBURST_R\t4.6.2\t0.0.0.9000\tabc\n";
        let stderr = "Loading required package: sagui\r\n\r\nFehler in segment_regions(cube) : \r\n  Keine gültigen Spektren übrig.\r\nRuft auf: do.call -> <Anonymous>\r\n\r\nAusführung angehalten\r\n";
        let message = failure_message(&format!("{identity}ASTROBURST_ERROR\tNo valid spectra remain after masking.\n"), stderr, "exit code: 1", &log);
        assert_eq!(message, format!("Rscript failed (exit code: 1): No valid spectra remain after masking.; full log: {shown}"));
        let message = failure_message(identity, stderr, "exit code: 1", &log);
        assert_eq!(message, format!("Rscript failed (exit code: 1): Loading required package: sagui | Fehler in segment_regions(cube) : | Keine gültigen Spektren übrig. | Ruft auf: do.call -> <Anonymous> | Ausführung angehalten; full log: {shown}"));
        let long = (1..=12).map(|i| format!("line {i:02}\n\n")).collect::<String>();
        let message = failure_message(identity, &long, "exit code: 1", &log);
        assert!(message.contains("line 05 | line 06") && message.contains("line 12;") && !message.contains("line 04"), "{message}");
        let refused = format!("{identity}ASTROBURST_REFUSED\tExact Ward in R is limited to 65536 support pixels; crop or choose sparse\n");
        assert_eq!(failure_message(&refused, stderr, "exit code: 3", &log), "Exact Ward in R is limited to 65536 support pixels; crop or choose sparse");
        assert_eq!(failure_message("", "", "exit code: 1", &log), format!("Rscript failed (exit code: 1); full log: {shown}"));
    }

    #[cfg(windows)]
    #[test]
    fn the_log_path_drops_the_verbatim_prefix_only_where_win32_can_open_the_result() {
        assert_eq!(failure_message("", "", "exit code: 1", Path::new(r"\\?\C:\jobs\sagui-run\stderr.log")), r"Rscript failed (exit code: 1); full log: C:\jobs\sagui-run\stderr.log");
        assert_eq!(failure_message("", "", "exit code: 1", Path::new(r"\\?\C:\jobs\con\stderr.log")), r"Rscript failed (exit code: 1); full log: \\?\C:\jobs\con\stderr.log");
    }

    #[test]
    fn a_mismatched_r_version_yields_exactly_one_expected_r_issue() {
        let reported = format!("ASTROBURST_R\t4.6.3\t0.0.0.9000\t{SAGUI_COMMIT}\nASTROBURST_ISSUE\tExpected R {R_VERSION}; found 4.6.3\n");
        let info = parse_runtime(&reported).unwrap();
        assert!(!info.ready);
        assert_eq!(info.issues, vec![format!("Expected R {R_VERSION}; found 4.6.3")]);
        let silent = parse_runtime(&format!("ASTROBURST_R\t4.6.3\t0.0.0.9000\t{SAGUI_COMMIT}\n")).unwrap();
        assert!(!silent.ready);
        assert_eq!(silent.issues, vec![format!("Expected R {R_VERSION}; found 4.6.3")]);
        assert!(parse_runtime(&format!("ASTROBURST_R\t{R_VERSION}\t0.0.0.9000\t{SAGUI_COMMIT}\n")).unwrap().ready);
    }

    #[test]
    fn a_slow_failing_script_reports_liveness_and_its_stderr_tail() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let script = {
            use std::os::windows::process::CommandExt;
            let mut script = Command::new("cmd.exe");
            script.raw_arg("/c echo Fehler in foo : kaputt 1>&2 & ping -n 5 127.0.0.1 >nul & exit 1").creation_flags(0x08000000);
            script
        };
        #[cfg(unix)]
        let script = {
            let mut script = Command::new("sh");
            script.args(["-c", "echo 'Fehler in foo : kaputt' >&2; sleep 4; exit 1"]);
            script
        };
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: ProgressSink = { let seen = Arc::clone(&seen); Arc::new(move |payload: &ProgressPayload| seen.lock().unwrap().push(payload.stage.clone())) };
        let progress = ProgressHandle::with_sink(sink, 5);
        let error = execute_labelled("Running official SAGUI in isolated R", script, dir.path(), &progress, None).unwrap_err().to_string();
        let stages = seen.lock().unwrap().clone();
        assert!(stages.iter().any(|stage| stage.starts_with("Running official SAGUI in isolated R - R running ") && stage.ends_with(" s - Fehler in foo : kaputt")), "{stages:?}");
        let status = if cfg!(windows) { "exit code: 1" } else { "exit status: 1" };
        assert!(error.starts_with(&format!("Rscript failed ({status}): Fehler in foo : kaputt; full log: ")) && error.ends_with("stderr.log"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn a_kill_on_close_job_ends_descendants_when_only_the_root_was_killed() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let mut root = Command::new("cmd.exe");
        root.raw_arg(r#"/c cmd.exe /c "echo started>started.txt & ping -n 4 127.0.0.1 >nul & echo survived>marker.txt""#);
        root.current_dir(dir.path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = spawn_suspended(&mut root).unwrap();
        let job = kill_on_close(&child).expect("a job object for the root process");
        resume(&child).unwrap();
        let started = Instant::now();
        while !dir.path().join("started.txt").exists() {
            assert!(started.elapsed() < Duration::from_secs(10), "the nested cmd.exe never started");
            std::thread::sleep(Duration::from_millis(50));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        drop(job);
        std::thread::sleep(Duration::from_secs(5));
        assert!(!dir.path().join("marker.txt").exists(), "a descendant outlived the job");
    }

    #[cfg(windows)]
    #[test]
    fn a_launcher_starts_no_descendant_before_it_joins_the_job() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let mut root = Command::new("cmd.exe");
        root.raw_arg(r#"/c start "" /b cmd.exe /c "ping -n 4 127.0.0.1 >nul & echo survived>marker.txt" & ping -n 2 127.0.0.1 >nul"#);
        root.current_dir(dir.path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = spawn_suspended(&mut root).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let job = kill_on_close(&child).expect("a job object for the launcher");
        resume(&child).unwrap();
        child.wait().unwrap();
        drop(job);
        std::thread::sleep(Duration::from_secs(4));
        assert!(!dir.path().join("marker.txt").exists(), "a descendant started before the job assignment escaped it");
    }

    #[cfg(windows)]
    #[test]
    fn a_descendant_left_by_a_finished_launcher_dies_when_execute_returns() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let mut root = Command::new("cmd.exe");
        root.raw_arg(r#"/c start "" /b cmd.exe /c "ping -n 3 127.0.0.1 >nul & echo survived>marker.txt""#);
        root.current_dir(dir.path()).stdin(Stdio::null());
        let progress = ProgressHandle::with_sink(Arc::new(|_| {}), 1);
        execute_labelled("Rscript", root, dir.path(), &progress, Some(Duration::from_secs(10))).unwrap();
        std::thread::sleep(Duration::from_secs(4));
        assert!(!dir.path().join("marker.txt").exists(), "a descendant outlived execute_labelled");
    }
}
