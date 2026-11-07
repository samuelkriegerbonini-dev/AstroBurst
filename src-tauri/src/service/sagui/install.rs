use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use anyhow::{anyhow, ensure, Context, Result};
use serde::Serialize;
use serde_json::json;
use crate::core::history::stamp::utc_timestamp;
use crate::infra::progress::ProgressHandle;
use crate::infra::paths::plain;
use super::{library_fingerprint, runtime, sha256, valid_run_id};

const SCRIPT: &str = include_str!("install.R");
const INSTALL_STAGE: &str = "Preparing private SAGUI packages and checking exact, starlet and sparse modes";

pub(crate) fn r_version_advice() -> String {
    let archive = if cfg!(windows) { format!(" (Windows: https://cran.r-project.org/bin/windows/base/old/{}/)", runtime::R_VERSION) } else { String::new() };
    format!("Install R {} alongside your current R{archive} and select its bin/Rscript{}", runtime::R_VERSION, if cfg!(windows) { ".exe" } else { "" })
}

pub(crate) fn r_version_hint(issues: &[String]) -> String {
    if issues.iter().any(|issue| issue.starts_with("Expected R")) { format!(". {}", r_version_advice()) } else { String::new() }
}

fn install_failure(work: &Path, error: anyhow::Error) -> anyhow::Error {
    let issues: Vec<String> = fs::read_to_string(work.join("stdout.log")).map(|text| text.lines().filter_map(|line| line.strip_prefix("ASTROBURST_ISSUE\t").map(str::to_string)).collect()).unwrap_or_default();
    if issues.is_empty() { error } else { anyhow!("{}. {}", issues.join("; "), r_version_advice()) }
}

#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub library_path: String,
    pub rscript_path: String,
    pub manifest_path: String,
    pub warnings: Vec<String>,
}

struct InstallLock {
    path: PathBuf,
    file: Option<File>,
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

fn prepare_work(output: &Path, run_id: &str, progress: &ProgressHandle) -> Result<(PathBuf, InstallLock)> {
    ensure!(valid_run_id(run_id), "Invalid SAGUI installation identifier");
    ensure!(output.is_absolute(), "Choose an absolute installation parent directory");
    ensure!(!progress.is_cancelled(), "SAGUI installation cancelled");
    let existing_parent = output.ancestors().find(|path| path.exists()).context("The installation parent has no existing ancestor")?.canonicalize()?;
    ensure!(!existing_parent.ancestors().any(|path| path.join("sagui/DESCRIPTION").is_file()), "Choose an installation parent outside an existing SAGUI library; recovery preserves the previous library");
    fs::create_dir_all(output)?;
    let work = plain(&output.canonicalize()?).join(format!("install-{run_id}"));
    fs::create_dir(&work).context("This installation identifier already exists; retry with a new identifier")?;
    let path = work.join(".install.lock");
    let file = OpenOptions::new().create_new(true).write(true).open(&path)?;
    let lock = InstallLock { path, file: Some(file) };
    fs::create_dir(work.join("library"))?;
    Ok((work, lock))
}

fn verify_self_tests(text: &str) -> Result<()> {
    let mut lines = text.lines();
    ensure!(lines.next() == Some("case\twidth\theight\tregions\tlabelled\tdeterministic"), "SAGUI installation did not return valid self-tests");
    let mut cases = std::collections::BTreeSet::new();
    for line in lines {
        let fields: Vec<_> = line.split('\t').collect();
        ensure!(fields.len() == 6, "Malformed SAGUI self-test record");
        ensure!(["exact", "starlet", "sparse"].contains(&fields[0]) && cases.insert(fields[0]), "Unexpected or repeated SAGUI self-test");
        ensure!(fields[1] == "11" && fields[2] == "7" && fields[3] == "2" && fields[5] == "TRUE", "SAGUI non-square self-test failed");
        let labelled: usize = fields[4].parse()?;
        ensure!((2..=77).contains(&labelled), "Invalid SAGUI self-test support");
    }
    ensure!(cases.len() == 3, "Exact, starlet and sparse SAGUI self-tests are all required");
    Ok(())
}

fn script_command(executable: &str, library: &str, work: &Path, action: &str) -> Result<std::process::Command> {
    let mut command = runtime::command(executable, library, work)?;
    fs::write(work.join("bridge.R"), SCRIPT)?;
    command.arg(action).arg(library).arg(work).arg(runtime::R_VERSION).arg(runtime::SAGUI_COMMIT);
    Ok(command)
}

fn publish(work: &Path, manifest: &serde_json::Value, progress: &ProgressHandle) -> Result<()> {
    let mut pending = tempfile::NamedTempFile::new_in(work)?;
    pending.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    pending.as_file().sync_all()?;
    ensure!(!progress.is_cancelled(), "SAGUI installation cancelled; no ready installation was published");
    pending.persist_noclobber(work.join("manifest.json"))?;
    Ok(())
}

fn install_in_dir(executable: &str, work: &Path, progress: &ProgressHandle) -> Result<InstallResult> {
    let library = work.join("library");
    let library_text = library.to_string_lossy().into_owned();
    let executable_hash = sha256(Path::new(executable), progress)?;
    fs::write(work.join("config.json"), serde_json::to_vec_pretty(&json!({
        "rscript_path": executable, "r_version": runtime::R_VERSION, "sagui_commit": runtime::SAGUI_COMMIT,
        "dependency_policy": "Depends, Imports, LinkingTo; no Suggests; upgrade=never",
        "started_at_utc": utc_timestamp(SystemTime::now()),
    }))?)?;
    let command = script_command(executable, &library_text, work, "install")?;
    progress.report(1, 4, INSTALL_STAGE);
    runtime::execute_labelled(INSTALL_STAGE, command, work, progress, None).map_err(|error| install_failure(work, error))?;
    let output = fs::read_to_string(work.join("stdout.log"))?;
    ensure!(output.lines().any(|line| line == "ASTROBURST_INSTALL_READY"), "SAGUI installation did not complete its verification");
    let identity = runtime::parse_runtime(&output)?;
    ensure!(identity.ready, "SAGUI installation returned an unsupported runtime: {}", identity.issues.join("; "));
    verify_self_tests(&fs::read_to_string(work.join("self-tests.tsv"))?)?;
    progress.report(2, 4, "Verifying the installed runtime independently");
    let checked = runtime::inspect_with_progress(executable, &library_text, progress);
    ensure!(checked.ready, "The installed SAGUI runtime is not ready: {}", checked.issues.join("; "));
    ensure!(checked.r_version == identity.r_version && checked.sagui_commit == identity.sagui_commit && checked.sagui_version == identity.sagui_version, "The installed runtime identity changed during verification");
    progress.report(3, 4, "Recording package versions and integrity hashes");
    let files = library_fingerprint(&library, progress)?;
    ensure!(!files.is_empty(), "The installed private library is empty");
    ensure!(sha256(Path::new(executable), progress)? == executable_hash, "Rscript changed during installation");
    let artifacts = ["bridge.R", "config.json", "stdout.log", "stderr.log", "self-tests.tsv", "packages.tsv", "session.txt", "build-tool-stdout.log", "build-tool-stderr.log"]
        .iter().map(|name| Ok(json!({ "path": name, "sha256": sha256(&work.join(name), progress)? }))).collect::<Result<Vec<_>>>()?;
    let result = InstallResult {
        library_path: library_text,
        rscript_path: executable.into(),
        manifest_path: work.join("manifest.json").to_string_lossy().into_owned(),
        warnings: vec![
            "Dependency versions are captured from this installation; this is not a universal lockfile for other dates or platforms".into(),
            "Keep this private library unchanged. Recovery creates a separate installation and preserves the previous one".into(),
        ],
    };
    let manifest = json!({ "schema_version": 1, "status": "ready", "astroburst": env!("CARGO_PKG_VERSION"),
        "runtime": checked, "rscript_sha256": executable_hash, "private_library": files, "artifacts": artifacts,
        "platform": std::env::consts::OS, "dependency_versions": "captured snapshot; not a distributed dependency lock",
        "self_tests": ["non-square exact Ward", "non-square starlet", "non-square sparse Ward", "determinism with seed 1"], "result": result });
    publish(work, &manifest, progress)?;
    progress.report(4, 4, "Private SAGUI installation verified");
    Ok(result)
}

pub fn install(executable: &str, output_dir: &str, run_id: &str, progress: &ProgressHandle) -> Result<InstallResult> {
    ensure!(Path::new(executable).is_absolute() && Path::new(executable).is_file(), "Choose an existing absolute Rscript executable path");
    progress.report(0, 4, "Preparing a new private SAGUI installation");
    let (work, _lock) = prepare_work(Path::new(output_dir), run_id, progress)?;
    let result = install_in_dir(executable, &work, progress);
    if let Err(error) = &result {
        let _ = fs::write(work.join("failure.json"), serde_json::to_vec_pretty(&json!({
            "status": if progress.is_cancelled() { "cancelled" } else { "failed" },
            "error": format!("{error:#}"), "published": false,
        }))?);
    }
    result.with_context(|| format!("SAGUI installation diagnostics: {}", work.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn progress() -> ProgressHandle { ProgressHandle::with_sink(Arc::new(|_| {}), 4) }

    #[test]
    fn installation_jobs_are_exclusive_and_never_overwrite_an_earlier_library() {
        let dir = tempfile::tempdir().unwrap();
        let (work, lock) = prepare_work(dir.path(), "first", &progress()).unwrap();
        fs::write(work.join("library/keep.txt"), "previous installation").unwrap();
        assert!(work.join(".install.lock").exists());
        assert!(prepare_work(dir.path(), "first", &progress()).is_err());
        drop(lock);
        assert!(!work.join(".install.lock").exists());
        assert!(prepare_work(dir.path(), "first", &progress()).is_err());
        assert_eq!(fs::read_to_string(work.join("library/keep.txt")).unwrap(), "previous installation");
        assert!(prepare_work(dir.path(), "recovery", &progress()).is_ok());
    }

    #[test]
    fn invalid_or_precancelled_installations_create_no_job() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["../escape", "a/b", "a\\b", "", "a:stream"] { assert!(prepare_work(dir.path(), id, &progress()).is_err()); }
        assert!(prepare_work(Path::new("relative"), "run", &progress()).is_err());
        let cancelled = progress().with_cancel_source(Arc::new(|| true));
        assert!(prepare_work(dir.path(), "cancelled", &cancelled).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn recovery_cannot_create_files_inside_an_existing_sagui_library() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("old-library");
        fs::create_dir_all(library.join("sagui")).unwrap();
        fs::write(library.join("sagui/DESCRIPTION"), "Package: sagui\n").unwrap();
        let before = library_fingerprint(&library, &progress()).unwrap();
        for output in [library.clone(), library.join("new/nested-parent")] {
            let error = prepare_work(&output, "recovery", &progress()).err().unwrap();
            assert!(error.to_string().contains("outside an existing SAGUI library"));
        }
        assert_eq!(before, library_fingerprint(&library, &progress()).unwrap());
        assert!(!library.join("new").exists());
    }

    #[test]
    fn installation_publication_is_atomic_nonoverwriting_and_cancelable() {
        let dir = tempfile::tempdir().unwrap();
        let cancelled = progress().with_cancel_source(Arc::new(|| true));
        assert!(publish(dir.path(), &json!({"status":"ready"}), &cancelled).is_err());
        assert!(!dir.path().join("manifest.json").exists());
        publish(dir.path(), &json!({"status":"ready","identity":"first"}), &progress()).unwrap();
        let before = fs::read(dir.path().join("manifest.json")).unwrap();
        assert!(publish(dir.path(), &json!({"identity":"replacement"}), &progress()).is_err());
        assert_eq!(fs::read(dir.path().join("manifest.json")).unwrap(), before);
    }

    #[test]
    fn failed_installer_preserves_diagnostics_without_publishing_or_holding_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("not-an-executable.exe");
        fs::write(&executable, "invalid executable").unwrap();
        let output = dir.path().join("installations");
        let error = install(executable.to_str().unwrap(), output.to_str().unwrap(), "failed", &progress()).unwrap_err();
        assert!(format!("{error:#}").contains("SAGUI installation diagnostics"));
        let work = output.join("install-failed");
        let failure: serde_json::Value = serde_json::from_slice(&fs::read(work.join("failure.json")).unwrap()).unwrap();
        assert_eq!(failure["status"], "failed");
        assert_eq!(failure["published"], false);
        for name in ["config.json", "bridge.R", "stdout.log", "stderr.log"] { assert!(work.join(name).exists(), "{name}"); }
        assert!(!work.join("manifest.json").exists());
        assert!(!work.join(".install.lock").exists());
    }

    #[test]
    fn r_version_issue_lines_turn_into_an_actionable_refusal() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("stdout.log"), "ASTROBURST_ISSUE\tExpected R 4.6.2 before installing packages; found 4.6.3\n").unwrap();
        let error = install_failure(dir.path(), anyhow::anyhow!("Rscript failed (exit code: 1); see stderr.log")).to_string();
        assert!(error.contains("found 4.6.3") && error.contains("Install R 4.6.2 alongside your current R"), "{error}");
        assert_eq!(error.contains("cran.r-project.org/bin/windows/base/old/4.6.2/"), cfg!(windows), "{error}");
        assert!(!error.contains("stderr.log"), "{error}");
        fs::write(dir.path().join("stdout.log"), "nothing useful").unwrap();
        assert_eq!(install_failure(dir.path(), anyhow::anyhow!("original")).to_string(), "original");
        assert_eq!(install_failure(&dir.path().join("absent"), anyhow::anyhow!("original")).to_string(), "original");
    }

    #[test]
    fn installation_requires_all_three_non_square_deterministic_self_tests() {
        let header = "case\twidth\theight\tregions\tlabelled\tdeterministic\n";
        let good = format!("{header}exact\t11\t7\t2\t77\tTRUE\nstarlet\t11\t7\t2\t50\tTRUE\nsparse\t11\t7\t2\t77\tTRUE\n");
        verify_self_tests(&good).unwrap();
        for bad in [String::new(), good.replace("starlet", "exact"), good.replace("TRUE", "FALSE"), good.replace("11\t7", "7\t11"), good.replace("\t77\t", "\t78\t"), good.lines().take(3).collect::<Vec<_>>().join("\n")] {
            assert!(verify_self_tests(&bad).is_err());
        }
    }

    #[test]
    #[ignore = "Requires pinned R and SAGUI private library; performs no installation or download"]
    fn managed_install_verification_runs_offline_on_the_pinned_private_library() {
        let executable = std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT");
        let library = std::env::var("ASTROBURST_SAGUI_LIBRARY").expect("ASTROBURST_SAGUI_LIBRARY");
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().canonicalize().unwrap();
        let progress = progress();
        let before = library_fingerprint(Path::new(&library), &progress).unwrap();
        let command = script_command(&executable, &library, &work, "verify").unwrap();
        runtime::execute(command, &work, &progress, None).unwrap();
        verify_self_tests(&fs::read_to_string(work.join("self-tests.tsv")).unwrap()).unwrap();
        assert!(runtime::parse_runtime(&fs::read_to_string(work.join("stdout.log")).unwrap()).unwrap().ready);
        assert_eq!(before, library_fingerprint(Path::new(&library), &progress).unwrap());
        assert!(!work.join("manifest.json").exists());
    }
}
