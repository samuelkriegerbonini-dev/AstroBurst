use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use crate::infra::net::paths;
use crate::infra::samples::{
    download_set, find_set, local_status, ready_paths, sample_sets, set_dir, LocalState,
    LocalStatus, Origins, SampleObserver, SampleSet,
};

pub const SAMPLE_DOWNLOAD_EVENT: &str = "sample:download";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const ALREADY_RUNNING: &str = "a download of this sample set is already running";
const NOT_DOWNLOADED: &str = "not downloaded";

static RUNNING: Mutex<BTreeMap<String, Arc<AtomicBool>>> = Mutex::new(BTreeMap::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleState {
    Absent,
    Partial,
    Downloading,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SampleSetReport {
    pub id: &'static str,
    pub title: &'static str,
    pub credit: &'static str,
    pub licence: &'static str,
    pub demonstrates: &'static str,
    pub total_bytes: u64,
    pub file_count: usize,
    pub state: SampleState,
    pub received_bytes: u64,
    pub folder: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SampleOpen {
    pub id: String,
    pub paths: Vec<String>,
    pub credit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SampleProgress {
    pub id: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

#[derive(Debug)]
struct Running {
    id: String,
    cancel: Arc<AtomicBool>,
}

fn running_downloads() -> MutexGuard<'static, BTreeMap<String, Arc<AtomicBool>>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Running {
    fn claim(id: &str) -> Result<Self, String> {
        let mut held = running_downloads();
        if held.contains_key(id) {
            return Err(ALREADY_RUNNING.to_owned());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        held.insert(id.to_owned(), Arc::clone(&cancel));
        Ok(Self {
            id: id.to_owned(),
            cancel,
        })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        running_downloads().remove(&self.id);
    }
}

fn is_running(id: &str) -> bool {
    running_downloads().contains_key(id)
}

fn cancel_running(id: &str) -> bool {
    match running_downloads().get(id) {
        Some(cancel) => {
            cancel.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

struct ProgressEmitter<F> {
    id: &'static str,
    cancel: Arc<AtomicBool>,
    last: Mutex<Option<Instant>>,
    emit: F,
}

impl<F: Fn(SampleProgress) + Sync> ProgressEmitter<F> {
    fn new(id: &'static str, cancel: Arc<AtomicBool>, emit: F) -> Self {
        Self {
            id,
            cancel,
            last: Mutex::new(None),
            emit,
        }
    }

    fn send(&self, received_bytes: u64, total_bytes: u64, file: &str) {
        (self.emit)(SampleProgress {
            id: self.id.to_owned(),
            received_bytes,
            total_bytes,
            file: file.to_owned(),
        });
    }

    fn finish(&self, set: &SampleSet) {
        let total = set.total_bytes();
        let last_file = set.files.last().map_or("", |file| file.name);
        self.send(total, total, last_file);
    }
}

impl<F: Fn(SampleProgress) + Sync> SampleObserver for ProgressEmitter<F> {
    fn progress(&self, received_bytes: u64, total_bytes: u64, file: &str) {
        {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|at| at.elapsed() < PROGRESS_INTERVAL) {
                return;
            }
            *last = Some(Instant::now());
        }
        self.send(received_bytes, total_bytes, file);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

fn set_report(
    set: &'static SampleSet,
    status: LocalStatus,
    running: bool,
    folder: PathBuf,
) -> SampleSetReport {
    let state = match (running, status.state) {
        (true, _) => SampleState::Downloading,
        (false, LocalState::Absent) => SampleState::Absent,
        (false, LocalState::Partial) => SampleState::Partial,
        (false, LocalState::Ready) => SampleState::Ready,
    };
    SampleSetReport {
        id: set.id,
        title: set.title,
        credit: set.credit,
        licence: set.licence,
        demonstrates: set.demonstrates,
        total_bytes: status.total_bytes,
        file_count: set.files.len(),
        state,
        received_bytes: status.received_bytes,
        folder,
    }
}

fn opened(set: &SampleSet, frames: Vec<PathBuf>) -> SampleOpen {
    SampleOpen {
        id: set.id.to_owned(),
        paths: frames
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        credit: set.credit.to_owned(),
    }
}

fn known_set(id: &str) -> Result<&'static SampleSet, String> {
    find_set(id).ok_or_else(|| format!("unknown sample set {id}"))
}

fn unreadable_root(error: anyhow::Error) -> String {
    format!("the Data folder setting could not be read: {error:#}")
}

#[tauri::command]
pub async fn sample_sets_cmd() -> Result<Vec<SampleSetReport>, String> {
    let root = paths::try_data_root().map_err(unreadable_root)?;
    Ok(sample_sets()
        .iter()
        .map(|set| {
            set_report(
                set,
                local_status(&root, set),
                is_running(set.id),
                set_dir(&root, set),
            )
        })
        .collect())
}

#[tauri::command]
pub async fn sample_download_cmd(app: tauri::AppHandle, id: String) -> Result<SampleOpen, String> {
    let set = known_set(&id)?;
    let running = Running::claim(set.id)?;
    let data = paths::resolved_data_root().map_err(unreadable_root)?;
    let root = data.require_present()?;
    let origins = Origins::from_env();
    let emitter = ProgressEmitter::new(set.id, Arc::clone(&running.cancel), move |progress| {
        if let Err(error) = app.emit(SAMPLE_DOWNLOAD_EVENT, progress) {
            log::debug!("sample download progress was not delivered: {error}");
        }
    });
    let frames = download_set(&origins, root, set, &emitter)
        .await
        .map_err(|error| error.to_string())?;
    emitter.finish(set);
    Ok(opened(set, frames))
}

#[tauri::command]
pub async fn sample_cancel_cmd(id: String) -> Result<bool, String> {
    Ok(cancel_running(&id))
}

#[tauri::command]
pub async fn sample_open_cmd(id: String) -> Result<SampleOpen, String> {
    let set = known_set(&id)?;
    let root = paths::try_data_root().map_err(unreadable_root)?;
    let frames = ready_paths(&root, set).ok_or_else(|| NOT_DOWNLOADED.to_owned())?;
    Ok(opened(set, frames))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use regex::Regex;
    use serde_json::json;

    use super::*;
    use crate::infra::samples::{find_set, LocalState, LocalStatus, SampleObserver};

    const COMMANDS: [&str; 4] = [
        "sample_sets_cmd",
        "sample_download_cmd",
        "sample_cancel_cmd",
        "sample_open_cmd",
    ];

    #[test]
    fn c1_a_set_downloads_once_at_a_time_and_its_slot_is_freed_on_every_exit() {
        let id = "c1-registry-set";
        let first = Running::claim(id).unwrap();
        assert!(is_running(id));
        assert_eq!(
            Running::claim(id).unwrap_err(),
            "a download of this sample set is already running"
        );
        assert!(cancel_running(id));
        assert!(first.cancel.load(Ordering::SeqCst));
        drop(first);
        assert!(!is_running(id));
        assert!(!cancel_running(id));
        let again = Running::claim(id).unwrap();
        assert!(!again.cancel.load(Ordering::SeqCst));
        drop(again);
        assert!(!cancel_running("c1-unknown-set"));
        let failing = || -> Result<(), String> {
            let _running = Running::claim(id)?;
            Err("cancelled".to_owned())
        };
        assert_eq!(failing(), Err("cancelled".to_owned()));
        assert!(!is_running(id));
    }

    #[test]
    fn c2_progress_is_emitted_at_most_every_100_ms_plus_a_final_event() {
        let set = find_set("hst-m16-wfpc2").unwrap();
        let sent = Arc::new(Mutex::new(Vec::<SampleProgress>::new()));
        let sink = Arc::clone(&sent);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let emitter = ProgressEmitter::new(set.id, Arc::clone(&cancel), move |progress| {
            sink.lock().unwrap().push(progress)
        });
        for received in 0..50 {
            emitter.progress(received, set.total_bytes(), "502nmos.fits");
        }
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(!emitter.is_cancelled());
        cancel.store(true, Ordering::SeqCst);
        assert!(emitter.is_cancelled());
        emitter.finish(set);
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(
            serde_json::to_value(&sent[1]).unwrap(),
            json!({
                "id": "hst-m16-wfpc2",
                "received_bytes": 30_896_640u64,
                "total_bytes": 30_896_640u64,
                "file": "673nmos.fits",
            })
        );
    }

    #[test]
    fn c3_payloads_use_the_snake_case_contract() {
        let set = find_set("hst-m16-wfpc2").unwrap();
        let folder = PathBuf::from("D:/data/samples/hst-m16-wfpc2");
        let partial = LocalStatus {
            state: LocalState::Partial,
            received_bytes: 5,
            total_bytes: set.total_bytes(),
        };
        let report = set_report(set, partial, true, folder.clone());
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "id": "hst-m16-wfpc2",
                "title": set.title,
                "credit": set.credit,
                "licence": set.licence,
                "demonstrates": set.demonstrates,
                "total_bytes": 30_896_640u64,
                "file_count": 3,
                "state": "downloading",
                "received_bytes": 5,
                "folder": "D:/data/samples/hst-m16-wfpc2",
            })
        );
        let ifu = find_set("jwst-gc-nirspec-ifu").expect("the IFU set is published");
        let absent = LocalStatus {
            state: LocalState::Absent,
            received_bytes: 0,
            total_bytes: ifu.total_bytes(),
        };
        let listed = serde_json::to_value(set_report(
            ifu,
            absent,
            false,
            PathBuf::from("D:/data/samples/jwst-gc-nirspec-ifu"),
        ))
        .unwrap();
        assert_eq!(
            (
                &listed["id"],
                &listed["state"],
                &listed["file_count"],
                &listed["total_bytes"],
                &listed["licence"],
                &listed["demonstrates"],
            ),
            (
                &json!("jwst-gc-nirspec-ifu"),
                &json!("absent"),
                &json!(2),
                &json!(175_680_000u64),
                &json!(ifu.licence),
                &json!(ifu.demonstrates),
            )
        );
        assert_eq!(listed.get("newer_at_mast"), None);
        assert!(!ifu.demonstrates.is_empty());
        let states = [
            (LocalState::Absent, false, "absent"),
            (LocalState::Partial, false, "partial"),
            (LocalState::Ready, false, "ready"),
            (LocalState::Absent, true, "downloading"),
        ];
        for (state, running, text) in states {
            let status = LocalStatus { state, ..partial };
            let value =
                serde_json::to_value(set_report(set, status, running, folder.clone())).unwrap();
            assert_eq!(value["state"], text);
        }
        let opened = SampleOpen {
            id: set.id.to_owned(),
            paths: vec!["D:/a.fits".to_owned()],
            credit: set.credit.to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&opened).unwrap(),
            json!({ "id": "hst-m16-wfpc2", "paths": ["D:/a.fits"], "credit": set.credit })
        );
    }

    fn handler_list(lib: &str) -> &str {
        let start = lib
            .find("tauri::generate_handler![")
            .expect("lib.rs registers its commands with generate_handler!");
        let end = lib[start..]
            .find("])")
            .expect("the generate_handler! list is closed");
        &lib[start..start + end]
    }

    #[test]
    fn c5_the_four_sample_commands_are_defined_and_registered_behind_the_net_feature() {
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        let handlers = handler_list(&lib);
        let defined: Vec<String> = Regex::new(r"#\[tauri::command\]\s*pub async fn (\w+)\(")
            .unwrap()
            .captures_iter(include_str!("sample_data.rs"))
            .map(|found| found[1].to_owned())
            .collect();
        assert_eq!(defined, COMMANDS);
        let registered: Vec<String> =
            Regex::new(r#"#\[cfg\(feature = "net"\)\]\s*cmd::sample_data::(\w+),"#)
                .unwrap()
                .captures_iter(handlers)
                .map(|found| found[1].to_owned())
                .collect();
        assert_eq!(registered, COMMANDS);
        assert_eq!(
            handlers.matches("cmd::sample_data::").count(),
            COMMANDS.len()
        );
        let imported = format!("use crate::cmd::sample_data::SAMPLE_DOWNLOAD_EVENT;\n{lib}");
        assert_eq!(
            handler_list(&imported)
                .matches("cmd::sample_data::")
                .count(),
            COMMANDS.len(),
            "an import was counted as a registration"
        );
        assert!(!lib.contains("sample_data_paths_cmd"));
        let cmd_mod = include_str!("mod.rs").replace("\r\n", "\n");
        assert!(cmd_mod.contains("#[cfg(feature = \"net\")]\npub mod sample_data;"));
    }
}
