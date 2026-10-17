use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::*;
use crate::core::astrometry::healpix_cat::{write_catalog, CatHeader, CatalogStar, Record};
use crate::infra::net::test_server::{serve, Resource, TestServer};

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const PLATE_SOLVE: &str = "plate solve";

struct Rig {
    dir: TempDir,
    store: CatalogStore,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let roots = StoreRoots::under(&dir.path().join("data"), &dir.path().join("cache"));
        let store = CatalogStore::new(roots).with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
        Self { dir, store }
    }

    fn with(mut self, spec: DatasetSpec) -> Self {
        self.store = self.store.with_dataset(spec);
        self
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn installed(&self, id: &str) -> PathBuf {
        let spec = self.store.spec(id).unwrap();
        self.store.dataset_dir(spec).join(spec.file_name)
    }
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(InstallStage, u64, Option<u64>)>>,
    cancel_once_bytes_arrive: bool,
    cancelled: AtomicBool,
    cancelled_at: Mutex<Option<u64>>,
    calls_after_cancel: AtomicUsize,
}

impl Recorder {
    fn cancelling() -> Arc<Self> {
        Arc::new(Self {
            cancel_once_bytes_arrive: true,
            ..Self::default()
        })
    }

    fn stages(&self) -> Vec<(InstallStage, u64, Option<u64>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl InstallObserver for Recorder {
    fn progress(&self, stage: InstallStage, done: u64, total: Option<u64>) {
        if self.cancelled.load(Ordering::SeqCst) {
            self.calls_after_cancel.fetch_add(1, Ordering::SeqCst);
        }
        self.calls.lock().unwrap().push((stage, done, total));
        if self.cancel_once_bytes_arrive && done > 0 && !self.cancelled.swap(true, Ordering::SeqCst)
        {
            *self.cancelled_at.lock().unwrap() = Some(done);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

fn recorder() -> Arc<Recorder> {
    Arc::new(Recorder::default())
}

fn client() -> reqwest::Client {
    catalog_client().unwrap()
}

fn payload(len: u64, seed: u64) -> Vec<u8> {
    (0..len)
        .map(|i| ((i * 31 + i / 4099 + seed) % 251) as u8)
        .collect()
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex_of(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hosted(id: &'static str, file_name: &'static str, url: String, body: &[u8]) -> DatasetSpec {
    DatasetSpec {
        id,
        title: "test dataset",
        kind: DatasetKind::Hosted,
        file_name,
        url: Some(url),
        size_bytes: Some(body.len() as u64),
        sha256: Some(digest(body)),
        compressed: Compression::None,
        header: None,
        licence: "test licence",
        credit: "test credit",
    }
}

fn astrometric_rule(level: u8) -> HeaderRule {
    HeaderRule {
        cat_types: &[CatType::Astrometric],
        level,
        whole_sky: true,
    }
}

fn pixel_centre(level: u8, pixel: u32) -> (f64, f64) {
    let (lon, lat) = cdshealpix::nested::center(level, u64::from(pixel));
    (lon.to_degrees(), lat.to_degrees())
}

fn catalogue(header: &CatHeader, pixels: &[(u32, Vec<Record>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_catalog(header, pixels, &mut bytes).unwrap();
    bytes
}

fn astrometric_catalogue(
    level: u8,
    stars_per_pixel: usize,
    pixels: impl Iterator<Item = u32>,
) -> Vec<u8> {
    let listed: Vec<(u32, Vec<Record>)> = pixels
        .map(|pixel| {
            let (ra, dec) = pixel_centre(level, pixel);
            let stars = (0..stars_per_pixel)
                .map(|n| Record::Star(CatalogStar::at(ra, dec, Some(9.0 + n as f64 * 0.1))))
                .collect();
            (pixel, stars)
        })
        .collect();
    catalogue(&CatHeader::whole("test astrometric", level, 1), &listed)
}

fn siril_like_catalogue() -> Vec<u8> {
    let pixel = cdshealpix::nested::hash(8, 10f64.to_radians(), 20f64.to_radians()) as u32;
    astrometric_catalogue(8, 3, std::iter::once(pixel))
}

fn xp_sampled_catalogue() -> Vec<u8> {
    catalogue(&CatHeader::whole("test xp", 8, 2), &[])
}

fn write_file(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn bzip2_compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn serve_body(body: &[u8]) -> TestServer {
    serve(Resource::new(body.to_vec()))
}

async fn install_from(rig: &Rig, id: &str) -> InstallOutcome {
    rig.store.install(id, &client(), recorder()).await.unwrap()
}

#[test]
fn the_three_built_in_datasets_report_missing_on_a_clean_temp_root() {
    let rig = Rig::new();
    let config = AppConfig::default();
    let ids: Vec<&str> = built_in_datasets().iter().map(|spec| spec.id).collect();
    assert_eq!(ids, [TYCHO2_ID, TWOMASS_ID, SIRIL_GAIA_ID]);
    for id in ids {
        assert_eq!(
            rig.store.status(id, &config).unwrap(),
            DatasetStatus::Missing,
            "{id}"
        );
    }
    assert!(!rig.path("data").exists(), "a status call created folders");
}

#[test]
fn status_reports_missing_partial_installed_external_and_cache_from_a_temp_root() {
    let body = payload(4 * KIB, 1);
    let rig = Rig::new()
        .with(hosted(
            "test-status-partial",
            "p.dat",
            "https://example.org/p.dat".into(),
            &body,
        ))
        .with(hosted(
            "test-status-installed",
            "i.dat",
            "https://example.org/i.dat".into(),
            &body,
        ))
        .with(DatasetSpec {
            kind: DatasetKind::Cache { cap_bytes: 1000 },
            url: None,
            size_bytes: None,
            sha256: None,
            ..hosted("test-status-cache", "unused", String::new(), &body)
        });
    let mut config = AppConfig::default();

    let partial = rig.installed("test-status-partial");
    write_file(&sibling(&partial, ".part"), &body[..1000]);
    write_file(
        &sibling(&partial, ".part.json"),
        br#"{"version":1,"url":"https://example.org/p.dat","etag":null,"last_modified":null,"expected_size":4096,"received":1000,"started_unix_s":1}"#,
    );
    assert_eq!(
        rig.store.status("test-status-partial", &config).unwrap(),
        DatasetStatus::Partial {
            received: 1000,
            total: Some(4096)
        }
    );

    let installed = rig.installed("test-status-installed");
    write_file(&installed, &body);
    assert_eq!(
        rig.store.status("test-status-installed", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed.clone(),
            verified: false
        }
    );
    let spec = rig.store.spec("test-status-installed").unwrap().clone();
    rig.store.ensure_verified(&spec, &installed).unwrap();
    assert_eq!(
        rig.store.status("test-status-installed", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed.clone(),
            verified: true
        }
    );

    let cache = rig
        .store
        .dataset_dir(rig.store.spec("test-status-cache").unwrap());
    write_file(&cache.join("a.bin"), &[1; 10]);
    write_file(&cache.join("sub").join("b.bin"), &[2; 20]);
    assert_eq!(
        rig.store.status("test-status-cache", &config).unwrap(),
        DatasetStatus::Cache {
            bytes: 30,
            files: 2,
            cap: 1000
        }
    );

    let external = rig.path("elsewhere").join("siril_cat_healpix8_astro.dat");
    write_file(&external, &siril_like_catalogue());
    config.external_catalogs.insert(
        SIRIL_GAIA_ID.to_owned(),
        external.to_str().unwrap().to_owned(),
    );
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::External {
            path: external.clone(),
            valid: true,
            problem: None
        }
    );
    let gone = rig.path("gone").join("siril_cat_healpix8_astro.dat");
    config
        .external_catalogs
        .insert(SIRIL_GAIA_ID.to_owned(), gone.to_str().unwrap().to_owned());
    match rig.store.status(SIRIL_GAIA_ID, &config).unwrap() {
        DatasetStatus::External {
            path,
            valid: false,
            problem: Some(problem),
        } => {
            assert_eq!(path, gone);
            assert!(problem.contains("does not exist"), "{problem}");
        }
        other => panic!("a missing external file is reported as {other:?}"),
    }
}

#[test]
fn register_adds_a_dataset_and_refuses_a_duplicate_or_malformed_id() {
    let spec = hosted(
        "test-registered-once",
        "r.dat",
        "https://example.org/r.dat".into(),
        b"r",
    );
    register(spec.clone()).unwrap();
    let known = registered();
    assert_eq!(
        known.iter().take(3).map(|spec| spec.id).collect::<Vec<_>>(),
        [TYCHO2_ID, TWOMASS_ID, SIRIL_GAIA_ID]
    );
    assert!(known.contains(&spec));
    let again = register(spec).unwrap_err();
    assert!(matches!(again, StoreError::Duplicate { .. }), "{again}");
    for id in [
        "Bad_Id",
        "",
        "-leading",
        ".leading-dot",
        "trailing-dot.",
        "dot..dot",
        "../escape",
    ] {
        let refused = register(hosted(
            Box::leak(id.to_owned().into_boxed_str()),
            "x.dat",
            "https://example.org/x.dat".into(),
            b"x",
        ))
        .unwrap_err();
        assert!(
            matches!(refused, StoreError::BadId { .. }),
            "{id:?}: {refused}"
        );
    }
    let built_in = register(built_in_datasets().remove(0)).unwrap_err();
    assert!(built_in.to_string().contains(TYCHO2_ID), "{built_in}");
}

const XP_TEST_ID: &str = "gaia-dr3-xp-siril-1.0.1";

fn registered_xp_spec() -> DatasetSpec {
    DatasetSpec {
        id: XP_TEST_ID,
        title: "Gaia DR3 XP spectra (test)",
        kind: DatasetKind::Registered,
        file_name: "xp",
        url: None,
        size_bytes: None,
        sha256: None,
        compressed: Compression::None,
        header: None,
        licence: "test licence",
        credit: "test credit",
    }
}

#[test]
fn the_n5_xp_dataset_registers_under_its_planned_dotted_id() {
    assert!(check_id(XP_TEST_ID).is_ok());
    register(registered_xp_spec()).unwrap();
    assert!(registered().iter().any(|spec| spec.id == XP_TEST_ID));
}

#[tokio::test]
async fn a_registered_dataset_reports_and_removes_its_module_files_but_is_never_installed_or_opened(
) {
    let rig = Rig::new().with(registered_xp_spec());
    let spec = rig.store.spec(XP_TEST_ID).unwrap().clone();
    let dir = rig.store.dataset_dir(&spec);
    assert_eq!(dir, rig.path("data").join(CATALOGS_DIR).join(XP_TEST_ID));
    let mut config = AppConfig::default();
    assert_eq!(
        rig.store.status(XP_TEST_ID, &config).unwrap(),
        DatasetStatus::Registered {
            bytes: 0,
            files: 0,
            folder: None,
            folder_present: false
        }
    );

    write_file(&dir.join("xp").join("0").join("1.bin"), &[1; 40]);
    write_file(&dir.join("xp").join("3").join("9.bin"), &[2; 60]);
    let siril_folder = rig.path("siril-xp");
    fs::create_dir_all(&siril_folder).unwrap();
    config.external_catalogs.insert(
        XP_TEST_ID.to_owned(),
        siril_folder.to_str().unwrap().to_owned(),
    );
    assert_eq!(
        rig.store.status(XP_TEST_ID, &config).unwrap(),
        DatasetStatus::Registered {
            bytes: 100,
            files: 2,
            folder: Some(siril_folder.clone()),
            folder_present: true
        }
    );
    let row = rig
        .store
        .report(&config)
        .into_iter()
        .find(|row| row.id == XP_TEST_ID)
        .unwrap();
    assert_eq!((row.kind, row.installable), ("registered", false));
    assert_eq!(row.folder, dir);

    let install = rig
        .store
        .install(XP_TEST_ID, &client(), recorder())
        .await
        .unwrap_err();
    assert!(install.to_string().contains("its own module"), "{install}");
    let open = match rig.store.open(XP_TEST_ID, &config, PLATE_SOLVE) {
        Ok(_) => panic!("a registered dataset was opened as one catalogue file"),
        Err(error) => error.to_string(),
    };
    assert!(open.contains("its own module"), "{open}");

    let removed = rig.store.remove(XP_TEST_ID, &mut config).unwrap();
    assert_eq!(removed.freed_bytes, 100);
    assert!(removed.forgot_external);
    assert!(!dir.exists());
    assert!(siril_folder.is_dir(), "the user's own folder was deleted");
    assert!(config.external_catalogs.is_empty());
}

#[tokio::test]
async fn an_install_interrupted_at_half_resumes_byte_identically() {
    let body = payload(4 * MIB, 3);
    let server = serve(Resource::new(body.clone()).drop_after(2 * MIB));
    let rig = Rig::new().with(hosted(
        "test-resume",
        "resume.dat",
        server.url("/catalogs/resume.dat").to_string(),
        &body,
    ));
    let config = AppConfig::default();

    let interrupted = rig
        .store
        .install("test-resume", &client(), recorder())
        .await
        .unwrap_err();
    assert!(
        matches!(interrupted, StoreError::Download { .. }),
        "{interrupted}"
    );
    assert_eq!(
        rig.store.status("test-resume", &config).unwrap(),
        DatasetStatus::Partial {
            received: 2 * MIB,
            total: Some(4 * MIB)
        }
    );

    let outcome = install_from(&rig, "test-resume").await;
    assert_eq!(outcome.resumed_from, 2 * MIB);
    assert_eq!(outcome.bytes, 4 * MIB);
    assert!(!outcome.already_installed);
    assert_eq!(outcome.sha256_hex, hex_of(&digest(&body)));
    assert_eq!(outcome.path, rig.installed("test-resume"));
    assert!(
        fs::read(&outcome.path).unwrap() == body,
        "the resumed file differs from the source"
    );
    assert_eq!(
        rig.store.status("test-resume", &config).unwrap(),
        DatasetStatus::Installed {
            path: outcome.path.clone(),
            verified: true
        }
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), Some("bytes=2097152-"));

    let again = install_from(&rig, "test-resume").await;
    assert!(again.already_installed);
    assert_eq!(
        server.requests().len(),
        2,
        "a verified install was downloaded again"
    );
}

#[tokio::test]
async fn a_flipped_byte_is_refused_naming_both_sha256_values() {
    let body = payload(3 * MIB, 5);
    let at = 2 * MIB + 7;
    let mut served = body.clone();
    served[at as usize] ^= 0xFF;
    let server = serve(Resource::new(body.clone()).flip_byte_at(at));
    let rig = Rig::new().with(hosted(
        "test-flipped",
        "flipped.dat",
        server.url("/catalogs/flipped.dat").to_string(),
        &body,
    ));

    let refused = rig
        .store
        .install("test-flipped", &client(), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(
        message.contains(&hex_of(&digest(&body))),
        "expected digest missing: {message}"
    );
    assert!(
        message.contains(&hex_of(&digest(&served))),
        "actual digest missing: {message}"
    );
    let installed = rig.installed("test-flipped");
    assert!(!installed.exists());
    assert!(!sibling(&installed, ".part").exists());
    assert_eq!(
        rig.store
            .status("test-flipped", &AppConfig::default())
            .unwrap(),
        DatasetStatus::Missing
    );
}

#[tokio::test]
async fn an_install_is_cancelled_within_one_progress_tick_and_keeps_the_part_file() {
    let body = payload(3 * MIB, 6);
    let server = serve(Resource::new(body.clone()).slow_steady(2 * MIB));
    let rig = Rig::new().with(hosted(
        "test-cancel",
        "cancel.dat",
        server.url("/catalogs/cancel.dat").to_string(),
        &body,
    ));
    let observer = Recorder::cancelling();

    let cancelled = rig
        .store
        .install("test-cancel", &client(), observer.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(cancelled, StoreError::Cancelled { .. }),
        "{cancelled}"
    );
    let flagged = observer
        .cancelled_at
        .lock()
        .unwrap()
        .expect("progress reported bytes");
    assert_eq!(observer.calls_after_cancel.load(Ordering::SeqCst), 0);
    assert!(flagged > 0 && flagged < 3 * MIB, "{flagged}");
    let part = sibling(&rig.installed("test-cancel"), ".part");
    assert_eq!(fs::metadata(&part).unwrap().len(), flagged);
    assert!(fs::read(&part).unwrap() == body[..flagged as usize]);
    assert_eq!(
        rig.store
            .status("test-cancel", &AppConfig::default())
            .unwrap(),
        DatasetStatus::Partial {
            received: flagged,
            total: Some(3 * MIB)
        }
    );
    assert!(observer
        .stages()
        .iter()
        .all(|(stage, _, total)| *stage == InstallStage::Download && *total == Some(3 * MIB)));
}

#[tokio::test]
async fn install_refuses_a_dataset_without_a_published_size_or_digest() {
    let rig = Rig::new();
    for (id, needle) in [
        (TYCHO2_ID, "ships with the app"),
        (TWOMASS_ID, "download size"),
        (SIRIL_GAIA_ID, "download sha-256"),
    ] {
        let refused = rig
            .store
            .install(id, &client(), recorder())
            .await
            .unwrap_err();
        let message = refused.to_string();
        assert!(
            matches!(refused, StoreError::Refused { .. }),
            "{id}: {message}"
        );
        assert!(
            message.contains(id) && message.contains(needle),
            "{id}: {message}"
        );
    }
    let built_in = [TYCHO2_ID, TWOMASS_ID, SIRIL_GAIA_ID];
    let report = rig.store.report(&AppConfig::default());
    let rows: Vec<&DatasetReport> = report
        .iter()
        .filter(|row| built_in.contains(&row.id))
        .collect();
    assert_eq!(rows.len(), built_in.len());
    assert!(rows.iter().all(|row| !row.installable), "{rows:?}");
}

#[tokio::test]
async fn a_bzip2_dataset_is_decoded_to_disk_and_verified_against_the_decompressed_digest() {
    let decoded = astrometric_catalogue(5, 6, 0..12 * 1024);
    assert!(
        decoded.len() as u64 > MIB && (decoded.len() as u64) < 2 * MIB,
        "{}",
        decoded.len()
    );
    let compressed = bzip2_compressed(&decoded);
    let server = serve_body(&compressed);
    let rig = Rig::new().with(DatasetSpec {
        compressed: Compression::Bzip2 {
            decoded_size: Some(decoded.len() as u64),
            decoded_sha256: Some(digest(&decoded)),
        },
        header: Some(astrometric_rule(5)),
        ..hosted(
            "test-bzip2",
            "bzip2.dat",
            server.url("/catalogs/bzip2.dat.bz2").to_string(),
            &compressed,
        )
    });
    let observer = recorder();

    let outcome = rig
        .store
        .install("test-bzip2", &client(), observer.clone())
        .await
        .unwrap();
    let installed = rig.installed("test-bzip2");
    assert_eq!(outcome.path, installed);
    assert_eq!(outcome.bytes, decoded.len() as u64);
    assert_eq!(outcome.sha256_hex, hex_of(&digest(&decoded)));
    assert!(
        fs::read(&installed).unwrap() == decoded,
        "the decoded file differs"
    );
    for leftover in [".bz2", ".bz2.done.json", ".bz2.part", ".decoding"] {
        assert!(
            !sibling(&installed, leftover).exists(),
            "{leftover} was left behind"
        );
    }
    assert_eq!(
        rig.store
            .status("test-bzip2", &AppConfig::default())
            .unwrap(),
        DatasetStatus::Installed {
            path: installed,
            verified: true
        }
    );
    let stages = observer.stages();
    assert!(stages
        .iter()
        .any(|(stage, _, _)| *stage == InstallStage::Download));
    assert_eq!(
        stages.last().copied(),
        Some((
            InstallStage::Decode,
            decoded.len() as u64,
            Some(decoded.len() as u64)
        ))
    );

    #[derive(Default)]
    struct CountingSink {
        writes: Vec<usize>,
        bytes: Vec<u8>,
    }
    impl Write for CountingSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes.push(buf.len());
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let streamed = decode_bzip2(
        "test-bzip2",
        compressed.as_slice(),
        CountingSink::default(),
        recorder().as_ref(),
        None,
    )
    .unwrap();
    assert_eq!(streamed.bytes, decoded.len() as u64);
    assert_eq!(streamed.sha256, digest(&decoded));
    assert!(streamed.writer.bytes == decoded);
    let largest = streamed.writer.writes.iter().copied().max().unwrap();
    assert!(
        largest <= DECODE_WRITE_BUFFER_BYTES,
        "a single write of {largest} bytes"
    );
    assert!(
        streamed.writer.writes.len() >= 2,
        "{:?}",
        streamed.writer.writes
    );
}

#[tokio::test]
async fn a_bzip2_dataset_whose_decoded_digest_differs_is_refused_naming_both_digests() {
    let decoded = payload(300 * KIB, 9);
    let compressed = bzip2_compressed(&decoded);
    let server = serve_body(&compressed);
    let published = digest(b"a different file");
    let rig = Rig::new().with(DatasetSpec {
        compressed: Compression::Bzip2 {
            decoded_size: Some(decoded.len() as u64),
            decoded_sha256: Some(published),
        },
        ..hosted(
            "test-bzip2-digest",
            "digest.dat",
            server.url("/catalogs/digest.dat.bz2").to_string(),
            &compressed,
        )
    });

    let refused = rig
        .store
        .install("test-bzip2-digest", &client(), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(message.contains(&hex_of(&published)), "{message}");
    assert!(message.contains(&hex_of(&digest(&decoded))), "{message}");
    let installed = rig.installed("test-bzip2-digest");
    assert!(!installed.exists());
    assert!(!sibling(&installed, ".decoding").exists());
}

#[test]
fn use_external_file_refuses_a_type_2_file_for_the_astrometric_slot_naming_the_type() {
    let rig = Rig::new();
    let xp = rig.path("xp").join("siril_cat_healpix8_xpsamp.dat");
    write_file(&xp, &xp_sampled_catalogue());
    let mut config = AppConfig::default();

    let refused = rig
        .store
        .use_external_file(SIRIL_GAIA_ID, &xp, &mut config)
        .unwrap_err();
    let message = refused.to_string();
    assert!(
        matches!(refused, StoreError::InvalidFile { .. }),
        "{message}"
    );
    assert!(message.contains("type 2"), "{message}");
    assert!(message.contains("type 1"), "{message}");
    assert!(config.external_catalogs.is_empty());

    let relative = rig
        .store
        .use_external_file(SIRIL_GAIA_ID, Path::new("x.dat"), &mut config);
    assert!(relative.unwrap_err().to_string().contains("absolute"));
    let hosted_slot = rig
        .store
        .use_external_file(TWOMASS_ID, &xp, &mut config)
        .unwrap_err();
    assert!(
        hosted_slot.to_string().contains("only external datasets"),
        "{hosted_slot}"
    );
}

#[test]
fn use_external_file_records_the_path_in_app_config() {
    let rig = Rig::new();
    let file = rig.path("siril").join("my-astro.dat");
    write_file(&file, &siril_like_catalogue());
    let mut config = AppConfig::default();

    let used = rig
        .store
        .use_external_file(SIRIL_GAIA_ID, &file, &mut config)
        .unwrap();
    assert_eq!(used, file);
    assert_eq!(
        config
            .external_catalogs
            .get(SIRIL_GAIA_ID)
            .map(String::as_str),
        file.to_str()
    );
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::External {
            path: file.clone(),
            valid: true,
            problem: None
        }
    );

    let stored = rig.path("config.json");
    fs::write(&stored, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let loaded = crate::infra::config::load_config_from(&stored).unwrap();
    assert_eq!(loaded.external_catalogs, config.external_catalogs);

    let old = rig.path("old-config.json");
    fs::write(
        &old,
        br#"{"astrometry_api_key":null,"astrometry_api_url":"https://nova.astrometry.net","plate_solve_timeout_secs":120}"#,
    )
    .unwrap();
    let loaded_old = crate::infra::config::load_config_from(&old).unwrap();
    assert!(loaded_old.external_catalogs.is_empty());
    assert_eq!(loaded_old.siril_catalog_dir, None);
}

#[test]
fn use_siril_dir_validates_the_astrometric_file_and_records_the_folder() {
    let rig = Rig::new();
    let folder = rig.path("Siril catalogues");
    write_file(
        &folder.join(SIRIL_ASTROMETRIC_FILE),
        &siril_like_catalogue(),
    );
    let mut config = AppConfig::default();

    let file = rig.store.use_siril_dir(&folder, &mut config).unwrap();
    assert_eq!(file, folder.join(SIRIL_ASTROMETRIC_FILE));
    assert_eq!(config.siril_catalog_dir.as_deref(), folder.to_str());
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::External {
            path: file,
            valid: true,
            problem: None
        }
    );

    let wrong = rig.path("wrong level");
    write_file(
        &wrong.join(SIRIL_ASTROMETRIC_FILE),
        &catalogue(&CatHeader::whole("level 7", 7, 1), &[]),
    );
    let mut untouched = AppConfig::default();
    let refused = rig.store.use_siril_dir(&wrong, &mut untouched).unwrap_err();
    assert!(refused.to_string().contains("level 7"), "{refused}");
    assert_eq!(untouched.siril_catalog_dir, None);
    let empty = rig.path("empty");
    fs::create_dir_all(&empty).unwrap();
    let missing = rig.store.use_siril_dir(&empty, &mut untouched).unwrap_err();
    assert!(missing.to_string().contains("does not exist"), "{missing}");
}

#[tokio::test]
async fn install_is_refused_before_download_when_free_space_is_below_the_need() {
    let body = payload(2 * MIB, 11);
    let server = serve_body(&body);
    let rig = Rig::new().with(hosted(
        "test-no-space",
        "space.dat",
        server.url("/catalogs/space.dat").to_string(),
        &body,
    ));
    let store = rig.store.with_free_space(Arc::new(|_: &Path| Ok(MIB)));

    let refused = store
        .install("test-no-space", &client(), recorder())
        .await
        .unwrap_err();
    match &refused {
        StoreError::NoSpace {
            need, available, ..
        } => {
            assert_eq!(*need, 2 * MIB);
            assert_eq!(*available, MIB);
        }
        other => panic!("expected a free-space refusal, got {other}"),
    }
    assert!(
        server.requests().is_empty(),
        "a request was sent before the refusal"
    );
}

fn solver_catalogue_spec(id: &'static str, server: &TestServer, body: &[u8]) -> DatasetSpec {
    DatasetSpec {
        header: Some(astrometric_rule(8)),
        ..hosted(
            id,
            "solver.dat",
            server.url("/catalogs/solver.dat").to_string(),
            body,
        )
    }
}

#[tokio::test]
async fn remove_succeeds_after_a_cone_query_released_the_mapping() {
    let body = siril_like_catalogue();
    let server = serve_body(&body);
    let rig = Rig::new().with(solver_catalogue_spec(
        "test-remove-released",
        &server,
        &body,
    ));
    let mut config = AppConfig::default();
    let installed = install_from(&rig, "test-remove-released").await.path;

    let lease = rig
        .store
        .open("test-remove-released", &config, PLATE_SOLVE)
        .unwrap();
    assert_eq!(lease.path(), installed);
    let rows = lease.cone(10.0, 20.0, 1.0, None).unwrap();
    assert_eq!(rows.len(), 3);
    drop(lease);
    assert!(!locked(&OPEN).contains_key(&installed));

    let removed = rig
        .store
        .remove("test-remove-released", &mut config)
        .unwrap();
    assert!(removed.freed_bytes >= body.len() as u64);
    assert!(!removed.forgot_external);
    assert!(!installed.exists());
    assert!(!rig
        .store
        .dataset_dir(rig.store.spec("test-remove-released").unwrap())
        .exists());
    assert_eq!(
        rig.store.status("test-remove-released", &config).unwrap(),
        DatasetStatus::Missing
    );
}

#[tokio::test]
async fn remove_is_refused_while_a_catalogue_is_held_open_naming_the_holder() {
    let body = siril_like_catalogue();
    let server = serve_body(&body);
    let rig = Rig::new().with(solver_catalogue_spec("test-remove-held", &server, &body));
    let mut config = AppConfig::default();
    let installed = install_from(&rig, "test-remove-held").await.path;

    let lease = rig
        .store
        .open("test-remove-held", &config, PLATE_SOLVE)
        .unwrap();
    let refused = rig
        .store
        .remove("test-remove-held", &mut config)
        .unwrap_err();
    assert!(matches!(refused, StoreError::InUse { .. }), "{refused}");
    assert!(refused.to_string().contains(PLATE_SOLVE), "{refused}");
    assert!(installed.exists());
    let reinstall = rig
        .store
        .install("test-remove-held", &client(), recorder())
        .await
        .unwrap_err();
    assert!(reinstall.to_string().contains(PLATE_SOLVE), "{reinstall}");
    assert_eq!(lease.cone(10.0, 20.0, 1.0, None).unwrap().len(), 3);

    drop(lease);
    rig.store.remove("test-remove-held", &mut config).unwrap();
    assert!(!installed.exists());
}

fn external_rig(bytes: &[u8]) -> (Rig, PathBuf, AppConfig) {
    let rig = Rig::new();
    let file = rig.path("siril").join("external-astro.dat");
    write_file(&file, bytes);
    let mut config = AppConfig::default();
    rig.store
        .use_external_file(SIRIL_GAIA_ID, &file, &mut config)
        .unwrap();
    (rig, file, config)
}

fn replace_by_rename(path: &Path, bytes: &[u8]) {
    let staged = sibling(path, ".new");
    fs::write(&staged, bytes).unwrap();
    fs::rename(&staged, path).unwrap();
}

fn five_star_catalogue() -> Vec<u8> {
    let pixel = cdshealpix::nested::hash(8, 10f64.to_radians(), 20f64.to_radians()) as u32;
    astrometric_catalogue(8, 5, std::iter::once(pixel))
}

#[test]
fn the_mapping_is_released_when_the_last_lease_drops() {
    let (rig, file, config) = external_rig(&siril_like_catalogue());
    let first = rig.store.open(SIRIL_GAIA_ID, &config, PLATE_SOLVE).unwrap();
    let second = rig.store.open(SIRIL_GAIA_ID, &config, "SPCC").unwrap();
    drop(first);
    assert!(locked(&OPEN).contains_key(&file));
    drop(second);
    assert!(
        !locked(&OPEN).contains_key(&file),
        "an idle mapping kept the file pinned"
    );
}

#[test]
fn a_file_replaced_on_disk_is_read_afresh_by_the_next_open() {
    let (rig, file, config) = external_rig(&siril_like_catalogue());
    let lease = rig.store.open(SIRIL_GAIA_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(lease.cone(10.0, 20.0, 1.0, None).unwrap().len(), 3);
    drop(lease);

    replace_by_rename(&file, &five_star_catalogue());
    let reopened = rig.store.open(SIRIL_GAIA_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(reopened.cone(10.0, 20.0, 1.0, None).unwrap().len(), 5);
}

#[test]
fn a_file_replaced_while_a_lease_is_held_gives_the_next_open_the_new_file() {
    let (rig, file, config) = external_rig(&siril_like_catalogue());
    let held = rig.store.open(SIRIL_GAIA_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(held.cone(10.0, 20.0, 1.0, None).unwrap().len(), 3);

    replace_by_rename(&file, &five_star_catalogue());
    let fresh = rig.store.open(SIRIL_GAIA_ID, &config, "SPCC").unwrap();
    assert_eq!(fresh.cone(10.0, 20.0, 1.0, None).unwrap().len(), 5);
    assert_eq!(held.cone(10.0, 20.0, 1.0, None).unwrap().len(), 3);

    let refused = rig
        .store
        .remove(SIRIL_GAIA_ID, &mut config.clone())
        .unwrap_err();
    let holders = refused.to_string();
    assert!(
        holders.contains(PLATE_SOLVE) && holders.contains("SPCC"),
        "{holders}"
    );
    drop(held);
    drop(fresh);
    assert!(!locked(&OPEN).contains_key(&file));
}

#[test]
fn removing_the_external_slot_forgets_the_file_and_folder_but_keeps_the_users_file() {
    let rig = Rig::new();
    let folder = rig.path("siril");
    let file = folder.join(SIRIL_ASTROMETRIC_FILE);
    write_file(&file, &siril_like_catalogue());
    let mut config = AppConfig::default();
    rig.store.use_siril_dir(&folder, &mut config).unwrap();
    rig.store
        .use_external_file(SIRIL_GAIA_ID, &file, &mut config)
        .unwrap();

    let lease = rig.store.open(SIRIL_GAIA_ID, &config, "SPCC").unwrap();
    let refused = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap_err();
    assert!(refused.to_string().contains("SPCC"), "{refused}");
    drop(lease);

    let removed = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap();
    assert!(removed.forgot_external);
    assert!(config.external_catalogs.is_empty());
    assert_eq!(config.siril_catalog_dir, None);
    assert!(file.exists(), "the user's own file was deleted");
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::Missing
    );
}

#[test]
fn a_bundled_tier0_file_is_verified_against_its_sidecar_at_first_use() {
    let tier0 = catalogue(&CatHeader::whole("tier 0", 6, 128), &[]);
    let resources = tempfile::tempdir().unwrap();
    let file = resources.path().join(TYCHO2_FILE);
    write_file(&file, &tier0);
    write_file(
        &sibling(&file, ".sha256"),
        format!("{}  {TYCHO2_FILE}\n", hex_of(&digest(&tier0))).as_bytes(),
    );
    let mut rig = Rig::new();
    let roots = rig
        .store
        .roots()
        .clone()
        .with_tier0_dir(resources.path().to_path_buf());
    rig.store = CatalogStore::new(roots);
    let config = AppConfig::default();

    assert_eq!(
        rig.store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: file.clone(),
            verified: false
        }
    );
    let lease = rig.store.open(TYCHO2_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(lease.header().healpix_level, 6);
    drop(lease);
    assert_eq!(
        rig.store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: file.clone(),
            verified: true
        }
    );
    assert!(rig
        .installed(TYCHO2_ID)
        .parent()
        .unwrap()
        .join("tycho2-l6-v1.sha256.ok")
        .is_file());

    let other = tempfile::tempdir().unwrap();
    let tampered = other.path().join(TYCHO2_FILE);
    write_file(&tampered, &tier0);
    let wrong = digest(b"not the tier 0 file");
    write_file(&sibling(&tampered, ".sha256"), hex_of(&wrong).as_bytes());
    let fresh = Rig::new();
    let store = CatalogStore::new(
        fresh
            .store
            .roots()
            .clone()
            .with_tier0_dir(other.path().to_path_buf()),
    );
    let refused = match store.open(TYCHO2_ID, &config, PLATE_SOLVE) {
        Ok(_) => panic!("a tier 0 file with the wrong digest was opened"),
        Err(error) => error.to_string(),
    };
    assert!(refused.contains(&hex_of(&wrong)), "{refused}");
    assert!(refused.contains(&hex_of(&digest(&tier0))), "{refused}");
}

#[test]
fn download_addresses_must_be_https_or_loopback() {
    for accepted in [
        "https://github.com/o/r/releases/download/t/f.dat",
        "http://127.0.0.1:43123/f.dat",
        "http://localhost:8080/f.dat",
        "http://[::1]:9000/f.dat",
    ] {
        assert!(download_url(accepted).is_ok(), "{accepted}");
    }
    for refused in [
        "http://example.org/f.dat",
        "ftp://127.0.0.1/f.dat",
        "not a url",
    ] {
        assert!(download_url(refused).is_err(), "{refused}");
    }
    assert!(hosted_url("2mass-l8-v1.dat").ends_with("/2mass-l8-v1.dat"));
}

#[test]
fn a_chosen_data_folder_that_vanished_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let vanished = dir.path().join("unplugged");
    let env = Some(OsString::from(vanished.as_os_str()));
    assert!(chosen_data_root_missing(
        env.clone(),
        &AppConfig::default(),
        &vanished
    ));
    let configured = AppConfig {
        data_dir_override: Some(vanished.to_str().unwrap().to_owned()),
        ..AppConfig::default()
    };
    assert!(chosen_data_root_missing(None, &configured, &vanished));
    assert!(!chosen_data_root_missing(
        None,
        &AppConfig::default(),
        &vanished
    ));
    assert!(!chosen_data_root_missing(
        env,
        &AppConfig::default(),
        dir.path()
    ));
}

#[test]
fn tier0_folders_come_from_the_env_then_the_resource_dir() {
    let resource = PathBuf::from("res");
    assert_eq!(
        tier0_dirs(Some(OsString::from("tier0")), Some(&resource)),
        [
            PathBuf::from("tier0"),
            resource.join("resources").join(CATALOGS_DIR),
            resource.join(CATALOGS_DIR)
        ]
    );
    assert_eq!(
        tier0_dirs(Some(OsString::new()), None),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn the_planned_tier0_file_is_found_by_the_flatpak_env_path_and_the_bundled_resource_layout() {
    assert_eq!(TYCHO2_FILE, "tycho2-l6-v1.abcat");
    let tier0 = catalogue(&CatHeader::whole("tier 0", 6, 128), &[]);
    let config = AppConfig::default();

    let flatpak = tempfile::tempdir().unwrap();
    let share = flatpak
        .path()
        .join("app/share/astroburst/catalogs")
        .join(TYCHO2_FILE);
    write_file(&share, &tier0);
    let rig = Rig::new();
    let mut roots = rig.store.roots().clone();
    roots.tier0_dirs = tier0_dirs(Some(share.clone().into_os_string()), None);
    assert_eq!(
        CatalogStore::new(roots).status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: share.clone(),
            verified: false
        }
    );

    let bundle = tempfile::tempdir().unwrap();
    let bundled = bundle
        .path()
        .join("resources")
        .join(CATALOGS_DIR)
        .join(TYCHO2_FILE);
    write_file(&bundled, &tier0);
    let mut roots = rig.store.roots().clone();
    roots.tier0_dirs = tier0_dirs(None, Some(bundle.path()));
    assert_eq!(
        CatalogStore::new(roots).status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: bundled,
            verified: false
        }
    );
}

#[derive(Default)]
struct CancelOnDecode(AtomicBool);

impl InstallObserver for CancelOnDecode {
    fn progress(&self, stage: InstallStage, done: u64, _total: Option<u64>) {
        if stage == InstallStage::Decode && done > 0 {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn flip_in_place(path: &Path, at: u64) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at as usize] ^= 0xFF;
    fs::write(path, &bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
}

fn bzip2_hosted(
    id: &'static str,
    file_name: &'static str,
    server: &TestServer,
    decoded: &[u8],
    compressed: &[u8],
) -> DatasetSpec {
    DatasetSpec {
        compressed: Compression::Bzip2 {
            decoded_size: Some(decoded.len() as u64),
            decoded_sha256: Some(digest(decoded)),
        },
        ..hosted(
            id,
            file_name,
            server
                .url(&format!("/catalogs/{file_name}.bz2"))
                .to_string(),
            compressed,
        )
    }
}

#[tokio::test]
async fn install_rehashes_an_unverified_installed_file_and_replaces_a_flipped_byte() {
    let body = payload(2 * MIB, 13);
    let server = serve_body(&body);
    let rig = Rig::new().with(hosted(
        "test-repair",
        "repair.dat",
        server.url("/catalogs/repair.dat").to_string(),
        &body,
    ));
    let config = AppConfig::default();
    let installed = install_from(&rig, "test-repair").await.path;
    flip_in_place(&installed, MIB + 3);
    assert_eq!(
        rig.store.status("test-repair", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed.clone(),
            verified: false
        }
    );

    let repaired = install_from(&rig, "test-repair").await;
    assert!(!repaired.already_installed);
    assert!(
        fs::read(&installed).unwrap() == body,
        "the flipped byte was kept and marked verified"
    );
    assert_eq!(
        rig.store.status("test-repair", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed,
            verified: true
        }
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), None);
}

#[tokio::test]
async fn an_installed_file_whose_marker_no_longer_matches_is_verified_locally_without_a_request() {
    let decoded = astrometric_catalogue(5, 2, 0..2048);
    let compressed = bzip2_compressed(&decoded);
    let archive_server = serve_body(&compressed);
    let plain = payload(MIB, 17);
    let plain_server = serve_body(&plain);
    let specs = [
        bzip2_hosted(
            "test-moved-bzip2",
            "moved.dat",
            &archive_server,
            &decoded,
            &compressed,
        ),
        hosted(
            "test-moved-plain",
            "plain.dat",
            plain_server.url("/catalogs/plain.dat").to_string(),
            &plain,
        ),
    ];
    let mut rig = Rig::new();
    for spec in &specs {
        rig = rig.with(spec.clone());
        install_from(&rig, spec.id).await;
    }
    let moved = rig.path("moved data");
    fs::rename(rig.path("data"), &moved).unwrap();
    let mut store = CatalogStore::new(StoreRoots::under(&moved, &rig.path("cache")))
        .with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
    for spec in &specs {
        store = store.with_dataset(spec.clone());
    }
    let config = AppConfig::default();

    for (spec, server, bytes) in [
        (&specs[0], &archive_server, &decoded),
        (&specs[1], &plain_server, &plain),
    ] {
        let path = store.dataset_dir(spec).join(spec.file_name);
        assert_eq!(
            store.status(spec.id, &config).unwrap(),
            DatasetStatus::Installed {
                path: path.clone(),
                verified: false
            }
        );
        let again = store.install(spec.id, &client(), recorder()).await.unwrap();
        assert!(again.already_installed, "{}: installed again", spec.id);
        assert_eq!(again.sha256_hex, hex_of(&digest(bytes)));
        assert_eq!(
            server.requests().len(),
            1,
            "{}: a request was sent for a file already on disk",
            spec.id
        );
        assert_eq!(
            store.status(spec.id, &config).unwrap(),
            DatasetStatus::Installed {
                path,
                verified: true
            }
        );
    }
}

#[tokio::test]
async fn an_update_that_pins_a_new_size_replaces_the_installed_file_on_the_next_install() {
    let old = payload(MIB, 19);
    let new = payload(MIB + 4 * KIB, 23);
    let server = serve(Resource::new(old.clone()).change_etag_after(1, new.clone()));
    let url = server.url("/catalogs/update.dat").to_string();
    let rig = Rig::new().with(hosted("test-update", "update.dat", url.clone(), &old));
    let config = AppConfig::default();
    let installed = install_from(&rig, "test-update").await.path;
    let rig = rig.with(hosted("test-update", "update.dat", url, &new));
    assert_eq!(
        rig.store.status("test-update", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed.clone(),
            verified: false
        }
    );

    let updated = install_from(&rig, "test-update").await;
    assert!(!updated.already_installed);
    assert_eq!(updated.bytes, new.len() as u64);
    assert!(
        fs::read(&installed).unwrap() == new,
        "the old file was kept"
    );
    assert_eq!(
        rig.store.status("test-update", &config).unwrap(),
        DatasetStatus::Installed {
            path: installed,
            verified: true
        }
    );
}

#[tokio::test]
async fn a_decode_cancelled_midway_restarts_from_the_kept_archive_without_a_request() {
    let decoded = astrometric_catalogue(5, 6, 0..12 * 1024);
    let compressed = bzip2_compressed(&decoded);
    let server = serve_body(&compressed);
    let rig = Rig::new().with(bzip2_hosted(
        "test-decode-cancel",
        "kept.dat",
        &server,
        &decoded,
        &compressed,
    ));
    let installed = rig.installed("test-decode-cancel");

    let cancelled = rig
        .store
        .install(
            "test-decode-cancel",
            &client(),
            Arc::new(CancelOnDecode::default()),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(cancelled, StoreError::Cancelled { .. }),
        "{cancelled}"
    );
    assert!(sibling(&installed, ".bz2").is_file());
    assert!(!installed.exists());

    let resumed = install_from(&rig, "test-decode-cancel").await;
    assert_eq!(resumed.resumed_from, compressed.len() as u64);
    assert!(fs::read(&installed).unwrap() == decoded);
    assert!(!sibling(&installed, ".bz2").exists());
    assert_eq!(
        server.requests().len(),
        1,
        "the kept archive was fetched or probed again"
    );
}

#[tokio::test]
async fn a_corrupted_kept_archive_is_downloaded_again_instead_of_decoded() {
    let decoded = astrometric_catalogue(5, 6, 0..12 * 1024);
    let compressed = bzip2_compressed(&decoded);
    let server = serve_body(&compressed);
    let rig = Rig::new().with(bzip2_hosted(
        "test-kept-corrupt",
        "corrupt.dat",
        &server,
        &decoded,
        &compressed,
    ));
    let installed = rig.installed("test-kept-corrupt");
    rig.store
        .install(
            "test-kept-corrupt",
            &client(),
            Arc::new(CancelOnDecode::default()),
        )
        .await
        .unwrap_err();
    flip_in_place(&sibling(&installed, ".bz2"), compressed.len() as u64 / 2);

    let repaired = install_from(&rig, "test-kept-corrupt").await;
    assert_eq!(repaired.resumed_from, 0);
    assert!(fs::read(&installed).unwrap() == decoded);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), None);
}

#[test]
fn open_refuses_an_install_that_started_while_it_waited_for_the_registry() {
    let body = siril_like_catalogue();
    let rig = Rig::new().with(DatasetSpec {
        header: Some(astrometric_rule(8)),
        ..hosted(
            "test-open-race",
            "race.dat",
            "https://example.org/race.dat".into(),
            &body,
        )
    });
    let config = AppConfig::default();
    write_file(&rig.installed("test-open-race"), &body);
    let spec = rig.store.spec("test-open-race").unwrap().clone();
    let dir = rig.store.dataset_dir(&spec);

    let registry = locked(&OPEN);
    let opened = std::thread::scope(|scope| {
        let opening = scope.spawn(|| {
            rig.store
                .open("test-open-race", &config, PLATE_SOLVE)
                .map(|lease| lease.path().to_path_buf())
        });
        std::thread::sleep(Duration::from_millis(300));
        let installing = InstallGuard::take(&dir, spec.id).unwrap();
        drop(registry);
        let opened = opening.join().unwrap();
        drop(installing);
        opened
    });
    let refused = opened.unwrap_err();
    assert!(refused.to_string().contains("being installed"), "{refused}");
}

#[test]
fn register_refuses_a_file_name_that_leaves_the_dataset_folder() {
    let mut names = vec![
        "../escape.dat",
        "nested/inner.dat",
        "",
        ".",
        "..",
        "/rooted.dat",
    ];
    if cfg!(windows) {
        names.extend([r"C:\elsewhere.dat", r"..\escape.dat", r"C:relative.dat"]);
    }
    for (n, name) in names.into_iter().enumerate() {
        let id: &'static str = Box::leak(format!("test-file-name-{n}").into_boxed_str());
        let refused =
            register(hosted(id, name, "https://example.org/x.dat".into(), b"x")).expect_err(name);
        assert!(
            matches!(refused, StoreError::Refused { .. })
                && refused.to_string().contains("file name"),
            "{name:?}: {refused}"
        );
        assert!(!registered().iter().any(|spec| spec.id == id), "{name:?}");
    }
}
