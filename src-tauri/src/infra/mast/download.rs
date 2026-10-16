use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::api::endpoint::MissionEndpoint;
use super::client::{ImageDownload, ImageOp};
use super::error::{MastError, MastResult};
use crate::core::mast::types::{MastProgressEvent, ProgressStage};

pub const HEAD_KEEP: usize = 64 * 1024;

static PART_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct PartFile {
    path: PathBuf,
    file: Option<File>,
    keep: bool,
}

impl PartFile {
    pub fn create(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let n = PART_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("dl-{}-{n}.part", std::process::id()));
        let file = File::create(&path)?;
        Ok(Self { path, file: Some(file), keep: false })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.write_all(bytes),
            None => Err(io::Error::other("part file already closed")),
        }
    }

    pub fn commit(mut self, final_path: &Path) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }
        match std::fs::rename(&self.path, final_path) {
            Ok(()) => {
                self.keep = true;
                Ok(())
            }
            Err(_) if final_path.is_file() => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub struct Fetched {
    part: PartFile,
    pub download: ImageDownload,
    pub head: Vec<u8>,
}

impl Fetched {
    pub fn commit(self, final_path: &Path) -> MastResult<()> {
        self.part.commit(final_path).map_err(MastError::Io)
    }
}

pub type ProgressFn<'a> = dyn FnMut(u64, Option<u64>) + Send + 'a;

pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ProgressThrottle {
    request_id: String,
    received: u64,
    total: Option<u64>,
    last: Option<(Instant, ProgressStage)>,
}

impl ProgressThrottle {
    pub fn new(request_id: &str) -> Self {
        Self { request_id: request_id.to_owned(), received: 0, total: None, last: None }
    }

    fn event(&mut self, stage: ProgressStage, now: Instant) -> MastProgressEvent {
        self.last = Some((now, stage));
        MastProgressEvent { request_id: self.request_id.clone(), received: self.received, total: self.total, stage }
    }

    pub fn download(&mut self, received: u64, total: Option<u64>, now: Instant) -> Option<MastProgressEvent> {
        self.received = received;
        self.total = total;
        let due = match self.last {
            Some((at, ProgressStage::Download)) => now.saturating_duration_since(at) >= PROGRESS_INTERVAL,
            _ => true,
        };
        due.then(|| self.event(ProgressStage::Download, now))
    }

    pub fn stage(&mut self, stage: ProgressStage, now: Instant) -> MastProgressEvent {
        self.event(stage, now)
    }
}

pub async fn fetch(
    endpoint: &MissionEndpoint,
    op: ImageOp,
    filename: Option<&str>,
    fileset_name: Option<&str>,
    cap: u64,
    dir: &Path,
    progress: &mut ProgressFn<'_>,
) -> MastResult<Fetched> {
    let mut part = PartFile::create(dir).map_err(MastError::Io)?;
    let mut head = Vec::new();
    let mut received = 0u64;
    let download = endpoint
        .download_image(op, filename, fileset_name, cap, &mut |chunk: &[u8], total: Option<u64>| {
            if head.len() < HEAD_KEEP {
                let take = chunk.len().min(HEAD_KEEP - head.len());
                head.extend_from_slice(&chunk[..take]);
            }
            part.write(chunk)?;
            received += chunk.len() as u64;
            progress(received, total);
            Ok(())
        })
        .await?;
    Ok(Fetched { part, download, head })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mast::api::mission::Mission;
    use crate::infra::mast::client::tests::{test_client, JPEG_HEAD};
    use crate::infra::mast::client::THUMBNAIL_CAP;
    use crate::infra::mast::testing::{serve_routes, Route};

    fn files_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn thumbnail_body_over_256k_is_too_large_and_leaves_no_part_file() {
        let over = THUMBNAIL_CAP as usize + 40_000;
        let server = serve_routes(vec![
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=declared").oversized(JPEG_HEAD.to_vec(), over, true),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=streamed").oversized(JPEG_HEAD.to_vec(), over, false),
        ])
        .await;
        let endpoint = MissionEndpoint::new(test_client(&server.base), Mission::Jwst);
        let dir = tempfile::tempdir().unwrap();
        for name in ["declared", "streamed"] {
            let error = fetch(&endpoint, ImageOp::Thumbnail, Some(name), None, THUMBNAIL_CAP, dir.path(), &mut |_, _| {})
                .await
                .err()
                .unwrap();
            match error {
                MastError::TooLarge { bytes, limit, .. } => {
                    assert_eq!(limit, THUMBNAIL_CAP);
                    assert!(bytes > THUMBNAIL_CAP, "{name}: {bytes}");
                }
                other => panic!("{name}: unexpected {other:?}"),
            }
            assert!(files_in(dir.path()).is_empty(), "{name}: {:?}", files_in(dir.path()));
        }
    }

    #[test]
    fn progress_is_throttled_to_100ms_with_final_ready() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::new("g1:preview:a:b:pane");
        let offers = [0u64, 10, 50, 99, 100, 150, 199, 201, 250, 400];
        let emitted: Vec<(u64, MastProgressEvent)> = offers
            .iter()
            .enumerate()
            .filter_map(|(i, ms)| {
                throttle
                    .download((i as u64 + 1) * 1000, Some(10_000), start + Duration::from_millis(*ms))
                    .map(|event| (*ms, event))
            })
            .collect();
        let times: Vec<u64> = emitted.iter().map(|(ms, _)| *ms).collect();
        assert_eq!(times, [0, 100, 201, 400]);
        for (_, event) in &emitted {
            assert_eq!(event.request_id, "g1:preview:a:b:pane");
            assert_eq!(event.stage, ProgressStage::Download);
            assert_eq!(event.total, Some(10_000));
        }
        assert_eq!(emitted[1].1.received, 5000);
        let decode = throttle.stage(ProgressStage::Decode, start + Duration::from_millis(401));
        assert_eq!(decode.stage, ProgressStage::Decode);
        assert_eq!(decode.received, 10_000);
        assert_eq!(decode.total, Some(10_000));
        let ready = throttle.stage(ProgressStage::Ready, start + Duration::from_millis(403));
        assert_eq!(ready.stage, ProgressStage::Ready);
        assert_eq!(ready.received, 10_000);
        let mut unknown = ProgressThrottle::new("g1:x");
        let first = unknown.download(512, None, start).unwrap();
        assert_eq!((first.received, first.total), (512, None));
        assert_eq!(unknown.stage(ProgressStage::Ready, start).total, None);
    }

    #[tokio::test]
    async fn fetched_image_commits_to_its_final_name() {
        let mut jpeg = JPEG_HEAD.to_vec();
        jpeg.extend(std::iter::repeat_n(1u8, 1000));
        let server = serve_routes(vec![Route::get("/search/hst/api/v0.1/get_image_preview").bytes(jpeg.clone())]).await;
        let endpoint = MissionEndpoint::new(test_client(&server.base), Mission::Hst);
        let dir = tempfile::tempdir().unwrap();
        let mut seen = Vec::new();
        let fetched = fetch(&endpoint, ImageOp::Preview, Some("x_drc.jpg"), Some("X"), 1 << 20, dir.path(), &mut |received, total| {
            seen.push((received, total));
        })
        .await
        .unwrap();
        assert_eq!(fetched.head, jpeg);
        assert_eq!(seen.last(), Some(&(jpeg.len() as u64, Some(jpeg.len() as u64))));
        let target = dir.path().join("x_drc.jpg");
        fetched.commit(&target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), jpeg);
        assert_eq!(files_in(dir.path()), vec!["x_drc.jpg".to_owned()]);
    }
}
