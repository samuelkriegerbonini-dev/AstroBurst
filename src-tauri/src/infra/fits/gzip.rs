use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Once, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use flate2::bufread::GzDecoder;
use tempfile::TempDir;

use crate::core::stacking::memory::{
    effective_memory, format_bytes, MemoryInfo, MEMORY_BUDGET_ENV,
};

pub(crate) const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
const GZIP_SUFFIX: &str = ".gz";
const FALLBACK_NAME: &str = "decompressed.fits";
const TEMP_PREFIX: &str = "astroburst-gunzip-";
const CHUNK_BYTES: usize = 1 << 20;
const WARM_COPIES: usize = 2;
const REMEMBERED_REFUSALS: usize = 16;
const TEMP_FREE_MARGIN: u64 = 1 << 30;
const STALE_COPY_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub const GZIP_BYTES_REFUSAL: &str =
    "this tool does not read gzip-compressed FITS yet; decompress the file (for example with gunzip) and open the plain FITS file";

pub type SharedTempDir = Arc<TempDir>;

static COPIES: CopyCache = CopyCache::new();
static SWEEP: Once = Once::new();

#[cfg(test)]
thread_local! {
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub fn starts_with_gzip(head: &[u8]) -> bool {
    head.starts_with(&GZIP_MAGIC)
}

pub fn resolve_gzip(path: &Path) -> Result<(PathBuf, Option<SharedTempDir>)> {
    if !is_gzip_file(path) {
        return Ok((path.to_path_buf(), None));
    }
    let root = std::env::temp_dir();
    SWEEP.call_once(|| sweep_stale_copies(&root, SystemTime::now(), STALE_COPY_AGE));
    let (copy, dir) = COPIES.resolve(path, &root, &temp_folder_cap)?;
    Ok((copy, Some(dir)))
}

pub(crate) fn temp_folder_cap() -> Cap {
    let root = std::env::temp_dir();
    decompression_cap(effective_memory(), temp_volume_free(&root), &root)
}

pub(crate) fn gunzip_or_copy(
    mut input: impl Read,
    output: &mut impl Write,
    cap: &dyn Fn() -> Cap,
    label: &str,
) -> Result<u64> {
    let mut head = Vec::with_capacity(GZIP_MAGIC.len());
    input
        .by_ref()
        .take(GZIP_MAGIC.len() as u64)
        .read_to_end(&mut head)
        .map_err(|e| anyhow!("{label} could not be extracted: {e}"))?;
    let stream = head.as_slice().chain(input);
    if !starts_with_gzip(&head) {
        return copy_capped(stream, output, &cap(), label);
    }
    Ok(gunzip_capped(stream, output, &cap(), label)?)
}

pub(crate) fn copy_capped(input: impl Read, output: &mut impl Write, cap: &Cap, label: &str) -> Result<u64> {
    let copied = std::io::copy(&mut input.take(cap.bytes.saturating_add(1)), output)
        .map_err(|e| anyhow!("{label} could not be extracted: {e}"))?;
    if copied > cap.bytes {
        return Err(anyhow!(cap.refusal(label, "the extracted member")));
    }
    Ok(copied)
}

pub(crate) fn split_gz_suffix(name: &str) -> (&str, Option<&str>) {
    let stem_len = name.len().checked_sub(GZIP_SUFFIX.len()).filter(|&len| {
        len > 0 && name.is_char_boundary(len) && name[len..].eq_ignore_ascii_case(GZIP_SUFFIX)
    });
    match stem_len {
        Some(len) => (&name[..len], Some(&name[len..])),
        None => (name, None),
    }
}

fn is_gzip_file(path: &Path) -> bool {
    let mut head = [0u8; GZIP_MAGIC.len()];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && starts_with_gzip(&head)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CapSource {
    MemoryBudget,
    TempVolume(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cap {
    bytes: u64,
    source: CapSource,
}

impl Cap {
    fn refusal(&self, label: &str, subject: &str) -> String {
        let limit = format_bytes(self.bytes);
        match &self.source {
            CapSource::MemoryBudget => format!(
                "{label}: {subject} is larger than {limit}, the memory budget for opening a compressed file, so it was refused before decompressing further. If the system can hold it, set {MEMORY_BUDGET_ENV} to the memory to allow (in MB) and restart."
            ),
            CapSource::TempVolume(root) => format!(
                "{label}: {subject} is larger than {limit}, the free space in the temporary folder {} once {} is kept free, so it was refused before decompressing further. Free space on that drive, or set TMP and TEMP (TMPDIR on Linux and macOS) to a folder on a larger drive and restart.",
                root.display(),
                format_bytes(TEMP_FREE_MARGIN)
            ),
        }
    }
}

fn decompression_cap(memory: Option<MemoryInfo>, temp_free: Option<u64>, root: &Path) -> Cap {
    let budget = memory.map_or(isize::MAX as u64, |info| info.commit_available);
    match temp_free.map(|free| free.saturating_sub(TEMP_FREE_MARGIN)) {
        Some(room) if room < budget => Cap {
            bytes: room,
            source: CapSource::TempVolume(root.to_path_buf()),
        },
        _ => Cap {
            bytes: budget,
            source: CapSource::MemoryBudget,
        },
    }
}

#[cfg(feature = "net")]
fn temp_volume_free(root: &Path) -> Option<u64> {
    crate::infra::net::paths::free_space(root).ok()
}

#[cfg(not(feature = "net"))]
fn temp_volume_free(_root: &Path) -> Option<u64> {
    None
}

fn sweep_stale_copies(root: &Path, now: SystemTime, max_age: Duration) {
    let Ok(listing) = std::fs::read_dir(root) else {
        return;
    };
    for entry in listing.flatten() {
        let ours = entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX);
        let stale = entry
            .metadata()
            .ok()
            .filter(|meta| meta.is_dir())
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if ours && stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[derive(Debug)]
enum GunzipError {
    TooLarge { message: String, cap: u64 },
    Damaged(String),
    Io(String),
}

impl std::fmt::Display for GunzipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { message, .. } | Self::Damaged(message) | Self::Io(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for GunzipError {}

fn io_failure(context: String) -> impl FnOnce(std::io::Error) -> GunzipError {
    move |error| GunzipError::Io(format!("{context}: {error}"))
}

type Slot = Arc<OnceLock<Outcome>>;

enum Outcome {
    Ready { copy: PathBuf, dir: Weak<TempDir> },
    Refused { message: String, cap: Option<u64> },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    fn of(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

struct Entry {
    stamp: Stamp,
    slot: Slot,
}

struct CacheState {
    entries: Vec<Entry>,
    warm: VecDeque<SharedTempDir>,
}

struct CopyCache {
    state: Mutex<CacheState>,
}

impl CacheState {
    fn prune(&mut self) {
        let mut refusals = 0;
        let mut keep: Vec<bool> = self
            .entries
            .iter()
            .rev()
            .map(|entry| match entry.slot.get() {
                None => true,
                Some(Outcome::Ready { dir, .. }) => dir.strong_count() > 0,
                Some(Outcome::Refused { .. }) => {
                    refusals += 1;
                    refusals <= REMEMBERED_REFUSALS
                }
                Some(Outcome::Failed(_)) => false,
            })
            .collect();
        keep.reverse();
        let mut flags = keep.into_iter();
        self.entries.retain(|_| flags.next().unwrap_or(true));
    }
}

impl CopyCache {
    const fn new() -> Self {
        Self {
            state: Mutex::new(CacheState {
                entries: Vec::new(),
                warm: VecDeque::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn resolve(
        &self,
        path: &Path,
        root: &Path,
        cap: &dyn Fn() -> Cap,
    ) -> Result<(PathBuf, SharedTempDir)> {
        let stamp = Stamp::of(path)?;
        loop {
            let slot = self.slot_for(&stamp, cap);
            let mut fresh = None;
            let outcome = slot.get_or_init(|| {
                let (outcome, dir) = decode(path, root, &cap());
                fresh = dir;
                outcome
            });
            let (copy, dir) = match outcome {
                Outcome::Ready { copy, dir } => (copy, fresh.or_else(|| dir.upgrade())),
                Outcome::Refused { message, .. } | Outcome::Failed(message) => {
                    return Err(anyhow!("{message}"));
                }
            };
            match dir {
                Some(dir) if copy.is_file() => {
                    self.keep_warm(&dir);
                    return Ok((copy.clone(), dir));
                }
                _ => self.forget(&slot),
            }
        }
    }

    fn forget(&self, slot: &Slot) {
        self.lock()
            .entries
            .retain(|entry| !Arc::ptr_eq(&entry.slot, slot));
    }

    fn slot_for(&self, stamp: &Stamp, cap: &dyn Fn() -> Cap) -> Slot {
        let mut state = self.lock();
        state.prune();
        let found = state
            .entries
            .iter()
            .position(|entry| entry.stamp == *stamp)
            .map(|index| state.entries.remove(index).slot);
        let slot = found.filter(|slot| !outgrown(slot, cap)).unwrap_or_default();
        state.entries.push(Entry {
            stamp: stamp.clone(),
            slot: Arc::clone(&slot),
        });
        slot
    }

    fn keep_warm(&self, dir: &SharedTempDir) {
        let evicted = {
            let mut state = self.lock();
            state.warm.retain(|warm| !Arc::ptr_eq(warm, dir));
            state.warm.push_front(Arc::clone(dir));
            let kept = WARM_COPIES.min(state.warm.len());
            state.warm.split_off(kept)
        };
        drop(evicted);
    }
}

fn outgrown(slot: &Slot, cap: &dyn Fn() -> Cap) -> bool {
    matches!(
        slot.get(),
        Some(Outcome::Refused { cap: Some(refused_at), .. })
            if cap().bytes > refused_at.saturating_add(refused_at / 4)
    )
}

fn decode(path: &Path, root: &Path, cap: &Cap) -> (Outcome, Option<SharedTempDir>) {
    #[cfg(test)]
    DECODES.with(|count| count.set(count.get() + 1));
    match gunzip_into(path, cap, root) {
        Ok((copy, dir)) => {
            let dir = Arc::new(dir);
            let ready = Outcome::Ready {
                copy,
                dir: Arc::downgrade(&dir),
            };
            (ready, Some(dir))
        }
        Err(GunzipError::TooLarge { message, cap }) => (
            Outcome::Refused {
                message,
                cap: Some(cap),
            },
            None,
        ),
        Err(GunzipError::Damaged(message)) => (Outcome::Refused { message, cap: None }, None),
        Err(GunzipError::Io(message)) => (Outcome::Failed(message), None),
    }
}

fn decompressed_name(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || FALLBACK_NAME.to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    split_gz_suffix(&name).0.to_string()
}

fn gunzip_into(path: &Path, cap: &Cap, root: &Path) -> Result<(PathBuf, TempDir), GunzipError> {
    let label = path.display().to_string();
    let source = File::open(path).map_err(io_failure(format!("Failed to open {label}")))?;
    let guard = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir_in(root)
        .map_err(io_failure(format!(
            "cannot create a temporary folder in {} to decompress {label}",
            root.display()
        )))?;
    let copy = guard.path().join(decompressed_name(path));
    let file = File::create(&copy).map_err(io_failure(format!(
        "cannot create {} to decompress {label}",
        copy.display()
    )))?;
    let mut out = BufWriter::with_capacity(CHUNK_BYTES, file);
    gunzip_capped(source, &mut out, cap, &label)?;
    out.flush().map_err(io_failure(format!(
        "{label}: writing the decompressed copy to {} failed",
        copy.display()
    )))?;
    Ok((copy, guard))
}

fn gunzip_capped(
    input: impl Read,
    output: &mut impl Write,
    cap: &Cap,
    label: &str,
) -> Result<u64, GunzipError> {
    let mut reader = BufReader::with_capacity(CHUNK_BYTES, input);
    let mut chunk = vec![0u8; CHUNK_BYTES];
    let mut total = 0u64;
    loop {
        let mut member = GzDecoder::new(&mut reader);
        loop {
            let read = match member.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(stream_error(&e, label)),
            };
            total = total.saturating_add(read as u64);
            if total > cap.bytes {
                return Err(GunzipError::TooLarge {
                    message: cap.refusal(label, "the decompressed gzip stream"),
                    cap: cap.bytes,
                });
            }
            output
                .write_all(&chunk[..read])
                .map_err(io_failure(format!("{label}: writing the decompressed data failed")))?;
        }
        drop(member);
        match byte_after_zero_padding(&mut reader).map_err(|e| stream_error(&e, label))? {
            None => return Ok(total),
            Some(byte) if byte == GZIP_MAGIC[0] => {}
            Some(_) => {
                return Err(GunzipError::Damaged(format!(
                    "{label} has data after its last gzip member that is neither another member nor zero padding; the file is corrupt"
                )));
            }
        }
    }
}

fn byte_after_zero_padding(reader: &mut impl BufRead) -> std::io::Result<Option<u8>> {
    loop {
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buffer.is_empty() {
            return Ok(None);
        }
        let zeros = buffer.iter().take_while(|&&byte| byte == 0).count();
        let next = buffer.get(zeros).copied();
        reader.consume(zeros);
        if next.is_some() {
            return Ok(next);
        }
    }
}

fn stream_error(error: &std::io::Error, label: &str) -> GunzipError {
    match error.kind() {
        ErrorKind::UnexpectedEof => GunzipError::Damaged(format!(
            "{label} is a truncated gzip file: the compressed stream ends before its end marker ({error})"
        )),
        ErrorKind::InvalidInput | ErrorKind::InvalidData => GunzipError::Damaged(format!(
            "{label} is not a valid gzip stream ({error}); the file is corrupt or truncated"
        )),
        _ => GunzipError::Io(format!("{label}: reading the gzip stream failed: {error}")),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    pub fn gzip_bytes(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    pub fn gzip_copy(plain: &Path, name: &str) -> PathBuf {
        let target = plain.with_file_name(name);
        std::fs::write(&target, gzip_bytes(&std::fs::read(plain).unwrap())).unwrap();
        target
    }

    pub fn decodes_on_this_thread() -> usize {
        super::DECODES.with(std::cell::Cell::get)
    }

    pub fn memory_budget(bytes: u64) -> super::Cap {
        super::Cap {
            bytes,
            source: super::CapSource::MemoryBudget,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use super::test_support::{decodes_on_this_thread, gzip_bytes, gzip_copy};
    use super::*;
    use crate::core::analysis::x1d::read_x1d_spectrum;
    use crate::core::analysis::x1d::test_fixtures::write_x1d_fixture;
    use crate::core::cube::companions::open_cube_set;
    use crate::core::cube::lazy::test_support::write_sci_err_dq_mef;
    use crate::core::cube::lazy::LazyCube;
    use crate::core::stacking::memory::{format_bytes, frame_sizes, MEMORY_BUDGET_ENV};
    use crate::infra::fits::dispatcher::resolve_single_image;
    use crate::infra::fits::reader::test_fixtures::{
        empty_primary_cards, plane_hdu, sci_err_dq_mef, write_raw_hdus,
    };
    use crate::infra::fits::reader::{load_fits_image, read_primary_header, HduInfo};
    use crate::infra::fits::spectrum1d::load_spectrum1d;
    use crate::infra::fits::table::load_bintable;
    use crate::infra::image_source::{list_planes, load_plane, resolve_plane_info};
    use crate::types::ImageRef;

    const BOMB_BYTES: usize = 64 << 20;
    const BOMB_CAP: u64 = 1 << 20;

    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    fn budget(bytes: u64) -> Cap {
        Cap {
            bytes,
            source: CapSource::MemoryBudget,
        }
    }

    fn unlimited() -> Cap {
        budget(u64::MAX)
    }

    fn noisy_image(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut sci = plane_hdu("SCI", 48, 40);
        let mut state: u32 = 2_463_534_242;
        sci.1 = (0..48 * 40)
            .flat_map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32).to_be_bytes()
            })
            .collect();
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), sci]);
        path
    }

    fn write_bomb(dir: &Path) -> PathBuf {
        let bomb = dir.join("bomb.fits.gz");
        std::fs::write(&bomb, gzip_bytes(&vec![0u8; BOMB_BYTES])).unwrap();
        bomb
    }

    fn document_under(path: &str, plain: &str) -> serde_json::Value {
        let mut document = serde_json::to_value(load_spectrum1d(path, None).unwrap()).unwrap();
        assert_eq!(document["source"]["path"], path);
        document["source"]["path"] = plain.into();
        document
    }

    #[test]
    fn a_gzip_stream_decodes_to_its_original_bytes() {
        let original: Vec<u8> = (0..250_000u32)
            .flat_map(|i| i.wrapping_mul(2_654_435_761).to_be_bytes())
            .collect();
        let mut out = Vec::new();
        let written = gunzip_capped(
            gzip_bytes(&original).as_slice(),
            &mut out,
            &unlimited(),
            "noise.gz",
        )
        .unwrap();
        assert_eq!(written, original.len() as u64);
        assert_eq!(out, original);
    }

    #[test]
    fn concatenated_gzip_members_decode_as_one_stream() {
        let mut joined = gzip_bytes(b"first member, ");
        joined.extend(gzip_bytes(b"second member"));
        let mut out = Vec::new();
        gunzip_capped(joined.as_slice(), &mut out, &unlimited(), "joined.gz").unwrap();
        assert_eq!(out, b"first member, second member");
    }

    #[test]
    fn zero_padding_after_a_member_is_ignored_and_other_trailing_bytes_are_refused() {
        let original: Vec<u8> = (0..20_000u32).flat_map(|i| i.to_be_bytes()).collect();
        let mut padded = gzip_bytes(&original);
        padded.extend([0u8; 2880]);
        let mut out = Vec::new();
        gunzip_capped(padded.as_slice(), &mut out, &unlimited(), "padded.fits.gz").unwrap();
        assert_eq!(out, original);

        let mut spaced = gzip_bytes(b"first member, ");
        spaced.extend([0u8; 512]);
        spaced.extend(gzip_bytes(b"second member"));
        let mut out = Vec::new();
        gunzip_capped(spaced.as_slice(), &mut out, &unlimited(), "spaced.gz").unwrap();
        assert_eq!(out, b"first member, second member");

        let mut junk = gzip_bytes(&original);
        junk.extend(b"junk");
        let err = gunzip_capped(junk.as_slice(), &mut Vec::new(), &unlimited(), "junk.fits.gz")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("junk.fits.gz") && err.contains("corrupt"),
            "{err}"
        );
    }

    #[test]
    fn a_stream_exactly_at_the_cap_passes_and_one_byte_over_is_refused() {
        let gz = gzip_bytes(&[7u8; 4096]);
        assert_eq!(
            gunzip_capped(gz.as_slice(), &mut Vec::new(), &budget(4096), "exact.gz").unwrap(),
            4096
        );
        let mut out = Vec::new();
        let err = gunzip_capped(gz.as_slice(), &mut out, &budget(4095), "over.gz")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("over.gz") && err.contains("memory budget"),
            "{err}"
        );
        assert!(
            out.len() <= 4095,
            "{} bytes written past the cap",
            out.len()
        );
    }

    #[test]
    fn plain_bytes_are_copied_up_to_the_cap_and_refused_past_it() {
        let plain = vec![7u8; 4096];
        let mut out = Vec::new();
        let copied = gunzip_or_copy(plain.as_slice(), &mut out, &|| budget(4096), "exact.fits").unwrap();
        assert_eq!((copied, out), (4096, plain.clone()));

        let mut out = Vec::new();
        let err = gunzip_or_copy(plain.as_slice(), &mut out, &|| budget(4095), "over.fits")
            .unwrap_err()
            .to_string();
        for expected in ["over.fits", format_bytes(4095).as_str(), MEMORY_BUDGET_ENV] {
            assert!(err.contains(expected), "missing {expected}: {err}");
        }
        assert!(!err.contains("gzip stream"), "{err}");
        assert!(out.len() <= 4096, "{} bytes written past the cap", out.len());
    }

    #[test]
    fn an_inflate_bomb_is_refused_naming_the_cap_before_its_output_passes_it() {
        let gz = gzip_bytes(&vec![0u8; BOMB_BYTES]);
        assert!(
            (gz.len() as u64) < BOMB_CAP / 8,
            "the archive must be a bomb: {} bytes",
            gz.len()
        );
        let mut out = Vec::new();
        let err = gunzip_capped(gz.as_slice(), &mut out, &budget(BOMB_CAP), "bomb.fits.gz")
            .unwrap_err()
            .to_string();
        assert!(
            out.len() as u64 <= BOMB_CAP,
            "{} bytes written past the cap",
            out.len()
        );
        for expected in [
            "bomb.fits.gz",
            format_bytes(BOMB_CAP).as_str(),
            MEMORY_BUDGET_ENV,
        ] {
            assert!(err.contains(expected), "missing {expected}: {err}");
        }
    }

    #[test]
    fn an_inflate_bomb_on_disk_is_refused_and_leaves_no_partial_copy() {
        let dir = tempfile::tempdir().unwrap();
        let bomb = write_bomb(dir.path());
        let root = tempfile::tempdir().unwrap();
        let err = format!(
            "{:#}",
            gunzip_into(&bomb, &budget(BOMB_CAP), root.path()).unwrap_err()
        );
        assert!(
            err.contains(&format_bytes(BOMB_CAP)) && err.contains("bomb.fits.gz"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            0,
            "the partial copy must be removed"
        );
    }

    #[test]
    fn a_refused_bomb_is_remembered_and_decoded_again_only_when_the_cap_grows() {
        let dir = tempfile::tempdir().unwrap();
        let bomb = write_bomb(dir.path());
        let root = tempfile::tempdir().unwrap();
        let cache = CopyCache::new();
        let before = decodes_on_this_thread();
        let refuse = |cap: u64| {
            format!(
                "{:#}",
                cache.resolve(&bomb, root.path(), &|| budget(cap)).unwrap_err()
            )
        };
        let first = refuse(BOMB_CAP);
        let second = refuse(BOMB_CAP);
        assert_eq!(decodes_on_this_thread() - before, 1, "the refusal must be remembered");
        assert_eq!(first, second);
        assert!(first.contains(&format_bytes(BOMB_CAP)), "{first}");
        refuse(BOMB_CAP + BOMB_CAP / 8);
        assert_eq!(decodes_on_this_thread() - before, 1, "a cap that barely moved reuses the refusal");
        let roomier = refuse(BOMB_CAP * 2);
        assert_eq!(decodes_on_this_thread() - before, 2, "a cap that grew by more than a quarter decodes again");
        assert!(roomier.contains(&format_bytes(BOMB_CAP * 2)), "{roomier}");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_damaged_file_is_remembered_and_a_failed_temp_folder_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let gz = gzip_bytes(&std::fs::read(&plain).unwrap());
        let cut = dir.path().join("cut.fits.gz");
        std::fs::write(&cut, &gz[..gz.len() / 2]).unwrap();
        let root = tempfile::tempdir().unwrap();
        let cache = CopyCache::new();
        let before = decodes_on_this_thread();
        for _ in 0..2 {
            let err = format!("{:#}", cache.resolve(&cut, root.path(), &unlimited).unwrap_err());
            assert!(err.contains("truncated"), "{err}");
        }
        assert_eq!(decodes_on_this_thread() - before, 1);

        let whole = gzip_copy(&plain, "whole.fits.gz");
        let missing = root.path().join("not-yet");
        for _ in 0..2 {
            let err = format!("{:#}", cache.resolve(&whole, &missing, &unlimited).unwrap_err());
            assert!(err.contains("cannot create a temporary folder"), "{err}");
        }
        assert_eq!(decodes_on_this_thread() - before, 3, "an I/O failure is not remembered");
        std::fs::create_dir(&missing).unwrap();
        let (copy, _dir) = cache.resolve(&whole, &missing, &unlimited).unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), std::fs::read(&plain).unwrap());
    }

    #[test]
    fn concurrent_callers_of_one_file_share_a_single_decode() {
        let dir = tempfile::tempdir().unwrap();
        let gz = gzip_copy(&noisy_image(dir.path(), "frame.fits"), "frame.fits.gz");
        let root = tempfile::tempdir().unwrap();
        let cache = CopyCache::new();
        let barrier = Barrier::new(4);
        let results: Vec<(usize, PathBuf)> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        let (copy, _dir) = cache.resolve(&gz, root.path(), &unlimited).unwrap();
                        (decodes_on_this_thread(), copy)
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        assert_eq!(results.iter().map(|r| r.0).sum::<usize>(), 1, "{results:?}");
        assert!(results.iter().all(|r| r.1 == results[0].1), "{results:?}");
    }

    #[test]
    fn an_idle_copy_stays_warm_until_two_newer_copies_push_it_out() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let files: Vec<PathBuf> = ["a", "b", "c"]
            .iter()
            .map(|n| gzip_copy(&noisy_image(dir.path(), &format!("{n}.fits")), &format!("{n}.fits.gz")))
            .collect();
        let cache = CopyCache::new();
        let before = decodes_on_this_thread();
        let first = cache.resolve(&files[0], root.path(), &unlimited).unwrap().0;
        assert_eq!(cache.resolve(&files[0], root.path(), &unlimited).unwrap().0, first);
        assert_eq!(decodes_on_this_thread() - before, 1, "an idle copy is reused while warm");
        assert!(first.exists());
        let (held, guard) = cache.resolve(&files[1], root.path(), &unlimited).unwrap();
        cache.resolve(&files[2], root.path(), &unlimited).unwrap();
        assert!(!first.exists(), "pushed out of the warm list with no other holder");
        cache.resolve(&files[0], root.path(), &unlimited).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 4);
        cache.resolve(&files[1], root.path(), &unlimited).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 4, "a held copy is found even when no longer warm");
        drop(cache);
        assert!(held.exists(), "a held copy outlives the cache");
        drop(guard);
        assert!(!held.exists(), "the copy goes with its last holder");
    }

    #[test]
    fn a_copy_removed_behind_the_cache_is_decoded_again() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let gz = gzip_copy(&plain, "frame.fits.gz");
        let root = tempfile::tempdir().unwrap();
        let cache = CopyCache::new();
        let before = decodes_on_this_thread();
        let first = cache.resolve(&gz, root.path(), &unlimited).unwrap().0;
        std::fs::remove_file(&first).unwrap();
        let (second, _dir) = cache.resolve(&gz, root.path(), &unlimited).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 2);
        assert_eq!(std::fs::read(second).unwrap(), std::fs::read(&plain).unwrap());
    }

    #[test]
    fn a_changed_file_is_decoded_again() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let gz = gzip_copy(&plain, "frame.fits.gz");
        let root = tempfile::tempdir().unwrap();
        let cache = CopyCache::new();
        let (first, _held) = cache.resolve(&gz, root.path(), &unlimited).unwrap();
        let mut longer = std::fs::read(&plain).unwrap();
        longer.extend([0u8; 2880]);
        std::fs::write(&gz, gzip_bytes(&longer)).unwrap();
        let (second, _dir) = cache.resolve(&gz, root.path(), &unlimited).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(second).unwrap(), longer);
    }

    #[test]
    fn copies_left_by_an_earlier_session_are_swept_after_a_day() {
        let root = tempfile::tempdir().unwrap();
        let leftover = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempdir_in(root.path())
            .unwrap()
            .keep();
        std::fs::write(leftover.join("frame.fits"), b"SIMPLE").unwrap();
        let unrelated = root.path().join("unrelated-folder");
        std::fs::create_dir(&unrelated).unwrap();
        let now = SystemTime::now();
        sweep_stale_copies(root.path(), now, STALE_COPY_AGE);
        assert!(leftover.exists(), "a fresh copy may still be in use");
        sweep_stale_copies(
            root.path(),
            now + STALE_COPY_AGE + Duration::from_secs(3600),
            STALE_COPY_AGE,
        );
        assert!(!leftover.exists(), "a day-old copy is swept");
        assert!(unrelated.exists(), "other folders are left alone");
    }

    #[test]
    fn a_truncated_fits_gz_is_a_clean_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let gz = gzip_bytes(&std::fs::read(&plain).unwrap());
        let cut = dir.path().join("cut.fits.gz");
        std::fs::write(&cut, &gz[..gz.len() / 2]).unwrap();
        let err = format!(
            "{:#}",
            load_plane(&ImageRef::parse(&text(&cut)))
                .err()
                .expect("a truncated gzip must not load")
        );
        assert!(
            err.contains("cut.fits.gz") && err.contains("truncated"),
            "{err}"
        );
    }

    #[test]
    fn a_corrupt_gzip_checksum_is_a_clean_error() {
        let mut gz = gzip_bytes(b"SIMPLE  =                    T");
        let crc = gz.len() - 8;
        gz[crc] ^= 0xFF;
        let err = gunzip_capped(gz.as_slice(), &mut Vec::new(), &unlimited(), "crc.fits.gz")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("crc.fits.gz") && err.contains("corrupt"),
            "{err}"
        );
    }

    #[test]
    fn plain_fits_is_not_copied_even_when_named_gz() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let misnamed = dir.path().join("misnamed.fits.gz");
        std::fs::copy(&plain, &misnamed).unwrap();
        for path in [&plain, &misnamed] {
            let (resolved, guard) = resolve_gzip(path).unwrap();
            assert_eq!(&resolved, path);
            assert!(guard.is_none());
        }
        assert!(!is_gzip_file(&dir.path().join("missing.fits.gz")));
    }

    #[test]
    fn gzip_is_recognised_by_its_magic_and_copied_without_the_gz_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let original = std::fs::read(&plain).unwrap();
        for (name, expected) in [
            ("frame.fits.gz", "frame.fits"),
            ("LUM.FIT.GZ", "LUM.FIT"),
            ("renamed.fits", "renamed.fits"),
        ] {
            let gz = gzip_copy(&plain, name);
            assert!(is_gzip_file(&gz), "{name}");
            let (copy, guard) = resolve_gzip(&gz).unwrap();
            assert!(guard.is_some(), "{name}");
            assert_eq!(copy.file_name().unwrap().to_str().unwrap(), expected);
            assert_eq!(std::fs::read(&copy).unwrap(), original, "{name}");
        }
    }

    #[test]
    fn the_cap_is_the_memory_budget_or_the_free_temp_space_whichever_is_smaller() {
        let info = MemoryInfo {
            physical_available: 3 << 30,
            commit_available: 5 << 30,
        };
        let root = Path::new("T:/scratch");
        assert_eq!(decompression_cap(Some(info), None, root), budget(5 << 30));
        assert_eq!(decompression_cap(None, None, root), budget(isize::MAX as u64));
        assert_eq!(decompression_cap(Some(info), Some(64 << 30), root), budget(5 << 30));
        let tight = decompression_cap(Some(info), Some(3 << 30), root);
        assert_eq!(
            tight,
            Cap {
                bytes: (3 << 30) - TEMP_FREE_MARGIN,
                source: CapSource::TempVolume(root.to_path_buf()),
            }
        );
        assert_eq!(decompression_cap(None, Some(TEMP_FREE_MARGIN / 2), root).bytes, 0);
        let message = tight.refusal("big.fits.gz", "the decompressed gzip stream");
        for expected in [
            "big.fits.gz",
            "T:/scratch",
            "free space",
            format_bytes((3 << 30) - TEMP_FREE_MARGIN).as_str(),
        ] {
            assert!(message.contains(expected), "missing {expected}: {message}");
        }
        assert!(!message.contains(MEMORY_BUDGET_ENV), "{message}");
    }

    #[test]
    fn one_compressed_mef_is_decoded_once_for_listing_loading_companions_and_headers() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("i2d.fits");
        sci_err_dq_mef(&plain, 6, 5, (0..30).map(|i| i % 3).collect());
        let gz = gzip_copy(&plain, "i2d.fits.gz");
        let gz_key = text(&gz);
        let before = decodes_on_this_thread();
        let (_, _pin) = resolve_gzip(&gz).unwrap();
        list_planes(&gz_key).unwrap();
        let science = load_plane(&ImageRef::parse(&gz_key)).unwrap();
        load_plane(&science.companions.err.expect("ERR companion")).unwrap();
        load_plane(&science.companions.dq.expect("DQ companion")).unwrap();
        resolve_plane_info(&ImageRef::hdu(&gz_key, 2)).unwrap();
        read_primary_header(&gz_key).unwrap();
        load_fits_image(&gz_key).unwrap();
        frame_sizes(std::slice::from_ref(&gz_key)).unwrap();
        resolve_single_image(&gz_key).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 1);
    }

    #[test]
    fn one_compressed_cube_set_shares_one_copy_for_sci_err_and_dq() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("shared_s3d.fits");
        write_sci_err_dq_mef(&plain, 4, 3, 5, |z, y, x| (z + y + x) as f32, |_, _, _| 0.25, |_, _, _| 0);
        let gz = gzip_copy(&plain, "shared_s3d.fits.gz");
        let gz_key = text(&gz);
        let before = decodes_on_this_thread();
        let (_, _pin) = resolve_gzip(&gz).unwrap();
        let set = open_cube_set(&gz_key).unwrap();
        list_planes(&gz_key).unwrap();
        read_primary_header(&gz_key).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 1);
        let (err, dq) = (set.err.expect("ERR cube"), set.dq.expect("DQ cube"));
        assert_eq!(set.sci.source_path, err.source_path);
        assert_eq!(err.source_path, dq.source_path);
    }

    #[test]
    fn a_single_image_fits_gz_reads_like_the_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let plain = noisy_image(dir.path(), "frame.fits");
        let gz = gzip_copy(&plain, "frame.fits.gz");
        let (plain_key, gz_key) = (text(&plain), text(&gz));
        let expected = load_plane(&ImageRef::parse(&plain_key)).unwrap();
        let loaded = load_plane(&ImageRef::parse(&gz_key)).unwrap();
        assert_eq!(loaded.arr, expected.arr);
        assert_eq!(loaded.header.cards, expected.header.cards);
        assert_eq!(
            load_fits_image(&gz_key).unwrap(),
            load_fits_image(&plain_key).unwrap()
        );
        assert_eq!(
            read_primary_header(&gz_key).unwrap().cards,
            read_primary_header(&plain_key).unwrap().cards
        );
        assert_eq!(
            frame_sizes(std::slice::from_ref(&gz_key)).unwrap(),
            frame_sizes(&[plain_key]).unwrap()
        );
        let (resolved, guard) = resolve_single_image(&gz_key).unwrap();
        assert!(guard.is_some() && resolved != gz);
    }

    #[test]
    fn a_mef_fits_gz_lists_and_loads_every_extension_with_its_companions() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("cal.fits");
        sci_err_dq_mef(&plain, 6, 5, (0..30).map(|i| i % 3).collect());
        let gz = gzip_copy(&plain, "cal.fits.gz");
        let (plain_key, gz_key) = (text(&plain), text(&gz));
        let shape = |planes: Vec<HduInfo>| {
            planes
                .into_iter()
                .map(|h| (h.index, h.extname, h.naxis1, h.naxis2, h.has_data))
                .collect::<Vec<_>>()
        };
        let (expected, _) = list_planes(&plain_key).unwrap();
        let (listed, is_asdf) = list_planes(&gz_key).unwrap();
        assert!(!is_asdf);
        assert_eq!(listed.len(), 5);
        assert_eq!(shape(listed), shape(expected));
        for hdu in 1..=4 {
            assert_eq!(
                load_plane(&ImageRef::hdu(&gz_key, hdu)).unwrap().arr,
                load_plane(&ImageRef::hdu(&plain_key, hdu)).unwrap().arr,
                "HDU {hdu}"
            );
        }
        let science = load_plane(&ImageRef::parse(&gz_key)).unwrap();
        let err = science
            .companions
            .err
            .expect("the ERR companion of the compressed MEF");
        let dq = science
            .companions
            .dq
            .expect("the DQ companion of the compressed MEF");
        assert_eq!(
            (err.path.as_str(), dq.path.as_str()),
            (gz_key.as_str(), gz_key.as_str())
        );
        assert_eq!(
            load_plane(&err).unwrap().arr,
            load_plane(&ImageRef::hdu(&plain_key, 2)).unwrap().arr
        );
    }

    #[test]
    fn a_cube_fits_gz_opens_lazily_with_its_err_and_dq_companions() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("line_s3d.fits");
        write_sci_err_dq_mef(
            &plain,
            4,
            3,
            6,
            |z, y, x| (100 * z + 10 * y + x) as f32,
            |_, _, _| 0.5,
            |z, _, _| i32::from(z == 2),
        );
        let gz = gzip_copy(&plain, "line_s3d.fits.gz");
        let (plain_key, gz_key) = (text(&plain), text(&gz));
        let expected = LazyCube::open(&plain_key).unwrap();
        let cube = LazyCube::open(&gz_key).unwrap();
        assert_eq!(cube.geometry.depth, 6);
        for z in 0..6 {
            assert_eq!(
                *cube.get_frame(z).unwrap(),
                *expected.get_frame(z).unwrap(),
                "frame {z}"
            );
        }
        let set = open_cube_set(&gz_key).unwrap();
        let (err, dq) = (set.err.expect("ERR cube"), set.dq.expect("DQ cube"));
        assert!(err.get_frame(0).unwrap().iter().all(|&v| v == 0.5));
        assert!(dq.get_frame(2).unwrap().iter().all(|&v| v == 1.0));
        assert!(set.notes.is_empty(), "{:?}", set.notes);
    }

    #[test]
    fn an_x1d_fits_gz_opens_as_a_spectrum_document_under_its_own_path() {
        let dir = tempfile::tempdir().unwrap();
        let plain = write_x1d_fixture(dir.path(), "spec_x1d.fits", 12, &[1], "um");
        let gz = gzip_copy(&plain, "spec_x1d.fits.gz");
        let (plain_key, gz_key) = (text(&plain), text(&gz));
        let x1d = read_x1d_spectrum(&gz_key, None).unwrap();
        assert_eq!(x1d.path, gz_key);
        assert_eq!(x1d.flux, read_x1d_spectrum(&plain_key, None).unwrap().flux);
        assert_eq!(
            load_bintable(&gz_key, 1)
                .unwrap()
                .column("FLUX")
                .unwrap()
                .values,
            load_bintable(&plain_key, 1)
                .unwrap()
                .column("FLUX")
                .unwrap()
                .values
        );
        let expected = serde_json::to_value(load_spectrum1d(&plain_key, None).unwrap()).unwrap();
        assert_eq!(document_under(&gz_key, &plain_key), expected);
    }

    #[test]
    fn a_wcs1d_fits_gz_opens_as_a_spectrum_document_under_its_own_path() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("bess.fits");
        let cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "1".into()),
            ("NAXIS1", "64".into()),
            ("CTYPE1", "'WAVE    '".into()),
            ("CUNIT1", "'Angstrom'".into()),
            ("CRVAL1", "6500.0".into()),
            ("CDELT1", "0.5".into()),
            ("CRPIX1", "1.0".into()),
            ("BUNIT", "'adu     '".into()),
        ];
        let flux = (0..64)
            .flat_map(|i| (1.0f32 + 0.01 * i as f32).to_be_bytes())
            .collect();
        write_raw_hdus(&plain, &[(cards, flux)]);
        let gz = gzip_copy(&plain, "bess.fits.gz");
        let (plain_key, gz_key) = (text(&plain), text(&gz));
        let expected = serde_json::to_value(load_spectrum1d(&plain_key, None).unwrap()).unwrap();
        assert_eq!(document_under(&gz_key, &plain_key), expected);
    }
}
