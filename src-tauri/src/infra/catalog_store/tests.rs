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
    let mut unpinned = built_in_datasets()
        .into_iter()
        .find(|spec| spec.id == TWOMASS_ID)
        .unwrap();
    unpinned.size_bytes = None;
    unpinned.sha256 = None;
    let rig = Rig::new().with(unpinned);
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
    let report = rig.store.report(&AppConfig::default());
    let installable: Vec<(&str, bool)> = report
        .iter()
        .filter(|row| [TYCHO2_ID, TWOMASS_ID, SIRIL_GAIA_ID].contains(&row.id))
        .map(|row| (row.id, row.installable))
        .collect();
    assert_eq!(
        installable,
        [
            (TYCHO2_ID, false),
            (SIRIL_GAIA_ID, false),
            (TWOMASS_ID, false)
        ]
    );
}

#[test]
fn the_built_in_catalogues_carry_the_pins_of_the_catalogs_v1_release() {
    assert_eq!(
        DEFAULT_CATALOG_BASE_URL,
        "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download/catalogs-v1/"
    );
    assert_eq!(hex_of(&TYCHO2_SHA256), TYCHO2_SHA256_HEX);
    assert_eq!(hex_of(&TWOMASS_SHA256), TWOMASS_SHA256_HEX);
    let specs = built_in_datasets();
    let spec = |id: &str| specs.iter().find(|spec| spec.id == id).unwrap();
    let tycho = spec(TYCHO2_ID);
    assert_eq!(
        (tycho.file_name, tycho.size_bytes, tycho.sha256),
        (TYCHO2_FILE, Some(15_541_742), Some(TYCHO2_SHA256))
    );
    assert_eq!(
        tycho.url, None,
        "tier 0 is bundled at build time, never downloaded by the app"
    );
    let twomass = spec(TWOMASS_ID);
    assert_eq!(
        (twomass.file_name, twomass.size_bytes, twomass.sha256),
        (TWOMASS_FILE, Some(154_140_794), Some(TWOMASS_SHA256))
    );
    assert_eq!(
        twomass.url.as_deref(),
        Some(hosted_url(TWOMASS_FILE).as_str())
    );
    assert_eq!(twomass.compressed, Compression::None);

    let rig = Rig::new();
    let report = rig.store.report(&AppConfig::default());
    let row = |id: &str| report.iter().find(|row| row.id == id).unwrap();
    assert!(row(TWOMASS_ID).installable, "{:?}", row(TWOMASS_ID));
    assert_eq!(
        row(TWOMASS_ID).url_host,
        url_host(&hosted_url(TWOMASS_FILE))
    );
    assert_eq!(row(TWOMASS_ID).size_bytes, Some(TWOMASS_BYTES));
    assert_eq!(row(TYCHO2_ID).size_bytes, Some(TYCHO2_BYTES));
    assert!(!row(TYCHO2_ID).installable);
}

fn script_constant<'a>(script: &'a str, name: &str) -> &'a str {
    let prefix = format!("export const {name} = ");
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .and_then(|value| value.strip_suffix(';'))
        .unwrap_or_else(|| panic!("scripts/fetch-tier0.mjs has no {name}"))
}

#[test]
fn the_bundled_tier0_pins_match_the_build_fetch_script_and_the_bundle_mapping() {
    let script = include_str!("../../../../scripts/fetch-tier0.mjs");
    assert_eq!(
        script_constant(script, "TIER0_FILE"),
        format!("\"{TYCHO2_FILE}\"")
    );
    assert_eq!(
        script_constant(script, "TIER0_BYTES"),
        TYCHO2_BYTES.to_string()
    );
    assert_eq!(
        script_constant(script, "TIER0_SHA256"),
        format!("\"{TYCHO2_SHA256_HEX}\"")
    );
    assert_eq!(
        script_constant(script, "DEFAULT_BASE_URL"),
        format!("\"{DEFAULT_CATALOG_BASE_URL}\"")
    );
    assert_eq!(
        script_constant(script, "ENV_BASE_URL"),
        format!("\"{ENV_CATALOG_BASE_URL}\"")
    );
    assert!(
        script_constant(script, "DEFAULT_OUT")
            .contains("\"src-tauri\", \"resources\", \"catalogs\", TIER0_FILE"),
        "the script writes outside the bundled resource folder"
    );

    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
    let mapped = conf["bundle"]["resources"]["resources/catalogs/"]
        .as_str()
        .expect("tauri.conf.json bundles resources/catalogs/");
    let resource_dir = PathBuf::from("installed-app");
    let probed = tier0_dirs(None, Some(&resource_dir));
    assert!(
        probed.contains(&resource_dir.join(mapped.trim_end_matches('/'))),
        "the bundle puts tier 0 under {mapped}, the store probes {probed:?}"
    );
    let before_build = conf["build"]["beforeBuildCommand"].as_str().unwrap();
    assert!(
        before_build.contains("node scripts/fetch-tier0.mjs"),
        "{before_build}"
    );
    let ignore = include_str!("../../../../.gitignore");
    assert!(
        ignore
            .lines()
            .any(|line| line.trim() == "/src-tauri/resources/catalogs/*.abcat"),
        "the fetched catalogue must stay out of git"
    );
}

#[test]
fn tier0_resource_resolves_and_verifies_its_pinned_hash() {
    let tier0 = catalogue(&CatHeader::whole("tier 0", 6, 128), &[]);
    let resource_dir = tempfile::tempdir().unwrap();
    let mapped = resource_dir.path().join(CATALOGS_DIR).join(TYCHO2_FILE);
    write_file(&mapped, &tier0);
    let rig = Rig::new();
    let mut roots = rig.store.roots().clone();
    roots.tier0_dirs = tier0_dirs(None, Some(resource_dir.path()));
    let config = AppConfig::default();

    let pinned = CatalogStore::new(roots.clone());
    let refused = match pinned.open(TYCHO2_ID, &config, PLATE_SOLVE) {
        Ok(_) => panic!("a file that is not the published tier 0 was opened"),
        Err(error) => error.to_string(),
    };
    assert!(refused.contains(TYCHO2_SHA256_HEX), "{refused}");
    assert!(refused.contains(&hex_of(&digest(&tier0))), "{refused}");

    let mut spec = pinned.spec(TYCHO2_ID).unwrap().clone();
    spec.sha256 = Some(digest(&tier0));
    spec.size_bytes = Some(tier0.len() as u64);
    let store = CatalogStore::new(roots).with_dataset(spec.clone());
    assert_eq!(
        store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: mapped.clone(),
            verified: false
        }
    );
    let lease = store.open(TYCHO2_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(lease.path(), mapped.as_path());
    drop(lease);
    assert_eq!(
        store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: mapped.clone(),
            verified: true
        }
    );

    let flipped_dir = tempfile::tempdir().unwrap();
    let flipped = flipped_dir.path().join(CATALOGS_DIR).join(TYCHO2_FILE);
    let mut bytes = tier0.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    write_file(&flipped, &bytes);
    let fresh = Rig::new();
    let mut roots = fresh.store.roots().clone();
    roots.tier0_dirs = tier0_dirs(None, Some(flipped_dir.path()));
    let store = CatalogStore::new(roots).with_dataset(spec);
    let refused = match store.open(TYCHO2_ID, &config, PLATE_SOLVE) {
        Ok(_) => panic!("a tier 0 file with a flipped byte was opened"),
        Err(error) => error,
    };
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(message.contains(&hex_of(&digest(&tier0))), "{message}");
    assert!(message.contains(&hex_of(&digest(&bytes))), "{message}");
}

#[test]
fn a_bundled_tier0_that_matches_its_pin_opens_when_the_data_folder_cannot_be_written() {
    let tier0 = catalogue(&CatHeader::whole("tier 0", 6, 128), &[]);
    let resource_dir = tempfile::tempdir().unwrap();
    let mapped = resource_dir.path().join(CATALOGS_DIR).join(TYCHO2_FILE);
    write_file(&mapped, &tier0);
    let rig = Rig::new();
    write_file(
        &rig.store.roots().catalogs,
        b"a file where the catalogue folder should be",
    );
    let mut roots = rig.store.roots().clone();
    roots.tier0_dirs = tier0_dirs(None, Some(resource_dir.path()));
    let mut spec = CatalogStore::new(roots.clone())
        .spec(TYCHO2_ID)
        .unwrap()
        .clone();
    spec.sha256 = Some(digest(&tier0));
    spec.size_bytes = Some(tier0.len() as u64);
    let store = CatalogStore::new(roots).with_dataset(spec);
    let config = AppConfig::default();

    for attempt in 1..=2 {
        let lease = store
            .open(TYCHO2_ID, &config, PLATE_SOLVE)
            .unwrap_or_else(|error| panic!("open {attempt} of the verified tier 0: {error}"));
        assert_eq!(lease.path(), mapped.as_path());
    }

    let mut flipped = tier0.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x01;
    write_file(&mapped, &flipped);
    let refused = match store.open(TYCHO2_ID, &config, PLATE_SOLVE) {
        Ok(_) => panic!("a tier 0 file changed after an unrecorded verification was opened"),
        Err(error) => error,
    };
    assert!(matches!(refused, StoreError::Checksum { .. }), "{refused}");
}

#[tokio::test]
async fn a_tier1_install_before_the_release_is_published_says_not_published_yet() {
    let body = payload(64 * KIB, 9);
    let server = serve(Resource::new(Vec::new()).status(404));
    let url = server.url("/catalogs-v1/2mass-l8-v1.dat").to_string();
    let rig = Rig::new().with(hosted(TWOMASS_ID, TWOMASS_FILE, url.clone(), &body));
    let refused = rig
        .store
        .install(TWOMASS_ID, &client(), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(
        matches!(refused, StoreError::NotPublished { .. }),
        "{message}"
    );
    assert!(message.contains("not published yet"), "{message}");
    assert!(
        message.contains(&url) && message.contains("HTTP 404"),
        "{message}"
    );
    assert_eq!(
        rig.store.status(TWOMASS_ID, &AppConfig::default()).unwrap(),
        DatasetStatus::Missing
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
#[ignore]
async fn ac11_the_real_2mass_cut_resumes_after_an_interruption_and_a_flipped_byte_is_refused() {
    let Some(path) = std::env::var_os("ASTROBURST_TIER1_PATH") else {
        eprintln!("skipped: set ASTROBURST_TIER1_PATH to the 2mass-l8-v1.dat file");
        return;
    };
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, TWOMASS_BYTES);
    let half = TWOMASS_BYTES / 2;
    let built_in = built_in_datasets()
        .into_iter()
        .find(|spec| spec.id == TWOMASS_ID)
        .unwrap();
    let mut config = AppConfig::default();

    let started = std::time::Instant::now();
    let server = serve(Resource::new(bytes.clone()).drop_after(half));
    let spec = DatasetSpec {
        url: Some(server.url("/catalogs-v1/2mass-l8-v1.dat").to_string()),
        ..built_in.clone()
    };
    let rig = Rig::new().with(spec);
    let interrupted = rig
        .store
        .install(TWOMASS_ID, &client(), recorder())
        .await
        .unwrap_err();
    assert!(
        matches!(interrupted, StoreError::Download { .. }),
        "{interrupted}"
    );
    assert_eq!(
        rig.store.status(TWOMASS_ID, &config).unwrap(),
        DatasetStatus::Partial {
            received: half,
            total: Some(TWOMASS_BYTES)
        }
    );
    let outcome = install_from(&rig, TWOMASS_ID).await;
    assert_eq!(outcome.resumed_from, half);
    assert_eq!(outcome.bytes, TWOMASS_BYTES);
    assert_eq!(outcome.sha256_hex, TWOMASS_SHA256_HEX);
    let lease = rig.store.open(TWOMASS_ID, &config, PLATE_SOLVE).unwrap();
    let pleiades = lease.cone(56.75, 24.12, 1.0, None).unwrap().len();
    assert_eq!(pleiades, 1911, "the tier-1 cone at the Pleiades");
    drop(lease);
    let removed = rig.store.remove(TWOMASS_ID, &mut config).unwrap();
    assert!(removed.freed_bytes >= TWOMASS_BYTES, "{removed:?}");
    assert!(!rig.installed(TWOMASS_ID).exists());
    println!(
        "AC-11 resume: {} bytes, resumed from {}, sha-256 {}, Pleiades cone {} rows, removed {} bytes, {:.1} s",
        outcome.bytes,
        outcome.resumed_from,
        outcome.sha256_hex,
        pleiades,
        removed.freed_bytes,
        started.elapsed().as_secs_f64()
    );
    drop(server);

    let flip_at = TWOMASS_BYTES / 3;
    let mut served = bytes.clone();
    served[flip_at as usize] ^= 0xFF;
    let flipped_digest = hex_of(&digest(&served));
    drop(served);
    let server = serve(Resource::new(bytes).flip_byte_at(flip_at));
    let spec = DatasetSpec {
        url: Some(server.url("/catalogs-v1/2mass-l8-v1.dat").to_string()),
        ..built_in
    };
    let rig = Rig::new().with(spec);
    let refused = rig
        .store
        .install(TWOMASS_ID, &client(), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(
        message.contains(TWOMASS_SHA256_HEX) && message.contains(&flipped_digest),
        "{message}"
    );
    assert!(!rig.installed(TWOMASS_ID).exists());
    assert_eq!(
        rig.store.status(TWOMASS_ID, &config).unwrap(),
        DatasetStatus::Missing
    );
    println!("AC-11 flipped byte at {flip_at}: {message}");
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
        u64::MAX,
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

fn unpinned_tier0() -> DatasetSpec {
    DatasetSpec {
        size_bytes: None,
        sha256: None,
        ..built_in_datasets()
            .into_iter()
            .find(|spec| spec.id == TYCHO2_ID)
            .unwrap()
    }
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
    rig.store = CatalogStore::new(roots).with_dataset(unpinned_tier0());
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
    )
    .with_dataset(unpinned_tier0());
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

#[test]
#[ignore]
fn the_real_tier0_file_verifies_against_its_pin_and_holds_the_pleiades() {
    let Some(path) = std::env::var_os("ASTROBURST_TIER0_PATH") else {
        eprintln!("skipped: set ASTROBURST_TIER0_PATH to the tycho2-l6-v1.abcat file");
        return;
    };
    let path = PathBuf::from(path);
    let rig = Rig::new();
    let roots = rig.store.roots().clone().with_tier0_dir(path.clone());
    let store = CatalogStore::new(roots);
    let config = AppConfig::default();
    assert_eq!(
        store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: path.clone(),
            verified: false
        }
    );
    let started = std::time::Instant::now();
    let lease = store.open(TYCHO2_ID, &config, PLATE_SOLVE).unwrap();
    let verified_in = started.elapsed();
    let pleiades = lease.cone(56.75, 24.12, 1.0, None).unwrap();
    assert_eq!(
        pleiades.len(),
        171,
        "Tycho-2 within 1 deg of the Pleiades at J2026.0"
    );
    drop(lease);
    assert_eq!(
        store.status(TYCHO2_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path,
            verified: true
        }
    );
    println!(
        "tier 0 verified against {TYCHO2_SHA256_HEX} in {:.0} ms; Pleiades cone {} rows",
        verified_in.as_secs_f64() * 1000.0,
        pleiades.len()
    );
}

const ZENODO_RECORD_PATH: &str = "/api/records/14692304";

struct ZenodoRig {
    record: TestServer,
    archive: TestServer,
    sums: Vec<TestServer>,
    compressed: Vec<u8>,
    decoded: Vec<u8>,
}

impl ZenodoRig {
    fn source(&self) -> ZenodoSource {
        ZenodoSource {
            record_url: self.record.url(ZENODO_RECORD_PATH).to_string(),
            decoded_max_bytes: SIRIL_GAIA_DECODED_MAX_BYTES,
        }
    }

    fn sum_requests(&self) -> usize {
        self.sums.iter().map(|server| server.requests().len()).sum()
    }
}

type PublishedSums = Option<([u8; 32], [u8; 32])>;

fn zenodo_catalogue() -> Vec<u8> {
    astrometric_catalogue(8, 4, (0..786_432u32).step_by(16))
}

fn sha256sum(digest: &[u8; 32], name: &str) -> Vec<u8> {
    format!("{}  {name}\n", hex_of(digest)).into_bytes()
}

fn zenodo_file(key: &str, size: usize, link: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "5da2012c-8247-449f-a9aa-9508cfa5ba41",
        "key": key,
        "size": size,
        "checksum": "md5:2e62f2365023e621afbd336c82980208",
        "links": { "self": link }
    })
}

fn zenodo_record(files: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "id": 14692304,
        "doi": "10.5281/zenodo.14692304",
        "metadata": {
            "title": "Siril Astrometry Catalogue extracted from Gaia DR3",
            "license": { "id": "cc-by-4.0" }
        },
        "links": { "self_html": "https://zenodo.org/records/14692304" },
        "files": files
    }))
    .unwrap()
}

fn content_link(server: &TestServer, key: &str) -> String {
    server
        .url(&format!("{ZENODO_RECORD_PATH}/files/{key}/content"))
        .to_string()
}

fn zenodo_rig(
    archive: impl FnOnce(Vec<u8>) -> Resource,
    sums: impl FnOnce(&[u8], &[u8]) -> PublishedSums,
) -> ZenodoRig {
    let decoded = zenodo_catalogue();
    let compressed = bzip2_compressed(&decoded);
    let archive_server = serve(archive(compressed.clone()));
    let mut files = vec![zenodo_file(
        SIRIL_GAIA_ARCHIVE_KEY,
        compressed.len(),
        &content_link(&archive_server, SIRIL_GAIA_ARCHIVE_KEY),
    )];
    let mut sum_servers = Vec::new();
    if let Some((archive_digest, decoded_digest)) = sums(&compressed, &decoded) {
        for (key, body) in [
            (
                SIRIL_GAIA_DECODED_SUM_KEY,
                sha256sum(&decoded_digest, SIRIL_ASTROMETRIC_FILE),
            ),
            (
                SIRIL_GAIA_ARCHIVE_SUM_KEY,
                sha256sum(&archive_digest, SIRIL_GAIA_ARCHIVE_KEY),
            ),
        ] {
            let server = serve_body(&body);
            files.push(zenodo_file(key, body.len(), &content_link(&server, key)));
            sum_servers.push(server);
        }
    }
    ZenodoRig {
        record: serve_body(&zenodo_record(files)),
        archive: archive_server,
        sums: sum_servers,
        compressed,
        decoded,
    }
}

fn true_sums(archive: &[u8], decoded: &[u8]) -> PublishedSums {
    Some((digest(archive), digest(decoded)))
}

fn consent_for(offer: &ZenodoOffer) -> ZenodoConsent {
    ZenodoConsent {
        archive_bytes: offer.archive_bytes,
        archive_sha256: offer.archive_sha256.clone(),
        licence_acknowledged: true,
    }
}

async fn zenodo_consent(rig: &Rig, zenodo: &ZenodoRig) -> ZenodoConsent {
    consent_for(
        &rig.store
            .zenodo_offer(&client(), &zenodo.source())
            .await
            .unwrap(),
    )
}

fn siril_dir(rig: &Rig) -> PathBuf {
    rig.store
        .dataset_dir(rig.store.spec(SIRIL_GAIA_ID).unwrap())
}

#[tokio::test]
async fn the_zenodo_offer_lists_the_record_files_size_sha256_and_terms_without_the_archive() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();

    let offer = rig
        .store
        .zenodo_offer(&client(), &zenodo.source())
        .await
        .unwrap();
    let keys: Vec<&str> = offer.files.iter().map(|file| file.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            SIRIL_GAIA_ARCHIVE_KEY,
            SIRIL_GAIA_DECODED_SUM_KEY,
            SIRIL_GAIA_ARCHIVE_SUM_KEY
        ]
    );
    assert_eq!(
        offer.files[0].checksum.as_deref(),
        Some("md5:2e62f2365023e621afbd336c82980208")
    );
    assert_eq!(offer.id, SIRIL_GAIA_ID);
    assert_eq!(offer.archive_key, SIRIL_GAIA_ARCHIVE_KEY);
    assert_eq!(offer.archive_bytes, zenodo.compressed.len() as u64);
    assert_eq!(
        offer.archive_url,
        content_link(&zenodo.archive, SIRIL_GAIA_ARCHIVE_KEY)
    );
    assert_eq!(
        offer.archive_sha256.as_deref(),
        Some(hex_of(&digest(&zenodo.compressed)).as_str())
    );
    assert_eq!(
        offer.decoded_sha256.as_deref(),
        Some(hex_of(&digest(&zenodo.decoded)).as_str())
    );
    assert_eq!(offer.doi.as_deref(), Some("10.5281/zenodo.14692304"));
    assert_eq!(offer.record_licence.as_deref(), Some("cc-by-4.0"));
    assert_eq!(
        offer.record_page.as_deref(),
        Some("https://zenodo.org/records/14692304")
    );
    assert_eq!(offer.host, "127.0.0.1");
    for needle in [
        "Gaia DR3",
        "ESA/Gaia/DPAC",
        "CC BY-NC 3.0 IGO",
        "this computer",
        "never redistributed",
    ] {
        assert!(
            offer.gaia_terms.contains(needle),
            "{needle}: {}",
            offer.gaia_terms
        );
    }
    assert_eq!(offer.decoded_max_bytes, SIRIL_GAIA_DECODED_MAX_BYTES);
    assert_eq!(offer.resume_from, 0);
    assert_eq!(
        offer.need_bytes,
        zenodo.compressed.len() as u64 + SIRIL_GAIA_DECODED_MAX_BYTES
    );
    assert_eq!(offer.free_space_bytes, Some(u64::MAX));
    assert_eq!(offer.folder, siril_dir(&rig));
    assert_eq!(zenodo.record.requests().len(), 1);
    assert_eq!(
        zenodo.record.requests()[0].header("accept"),
        Some("application/json")
    );
    assert_eq!(zenodo.sum_requests(), 2);
    assert!(
        zenodo.archive.requests().is_empty(),
        "the offer fetched catalogue bytes"
    );
    assert!(!siril_dir(&rig).exists());
}

#[tokio::test]
async fn a_zenodo_install_without_consent_is_refused_before_any_request() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();
    let unacknowledged = ZenodoConsent {
        archive_bytes: zenodo.compressed.len() as u64,
        archive_sha256: Some(hex_of(&digest(&zenodo.compressed))),
        licence_acknowledged: false,
    };

    for consent in [None, Some(&unacknowledged)] {
        let refused = rig
            .store
            .install_siril_from_zenodo(&client(), &zenodo.source(), consent, recorder())
            .await
            .unwrap_err();
        let message = refused.to_string();
        assert!(matches!(refused, StoreError::Refused { .. }), "{message}");
        assert!(
            message.contains(SIRIL_GAIA_ID) && message.contains("consent"),
            "{message}"
        );
    }
    assert!(zenodo.record.requests().is_empty());
    assert_eq!(zenodo.sum_requests(), 0);
    assert!(zenodo.archive.requests().is_empty());
    assert!(!siril_dir(&rig).exists());
}

#[tokio::test]
async fn a_zenodo_install_whose_record_changed_since_the_consent_is_refused_before_the_archive() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();
    let shown = zenodo_consent(&rig, &zenodo).await;

    for stale in [
        ZenodoConsent {
            archive_bytes: shown.archive_bytes + 1,
            ..shown.clone()
        },
        ZenodoConsent {
            archive_sha256: Some(hex_of(&digest(b"an older upload"))),
            ..shown.clone()
        },
    ] {
        let refused = rig
            .store
            .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&stale), recorder())
            .await
            .unwrap_err();
        let message = refused.to_string();
        assert!(matches!(refused, StoreError::Refused { .. }), "{message}");
        assert!(message.contains("changed"), "{message}");
        assert!(
            message.contains(&zenodo.compressed.len().to_string()),
            "{message}"
        );
    }
    assert_eq!(zenodo.record.requests().len(), 3);
    assert!(zenodo.archive.requests().is_empty());
}

#[tokio::test]
async fn a_zenodo_download_interrupted_midway_resumes_decodes_and_registers_the_siril_folder() {
    let zenodo = zenodo_rig(
        |body| {
            let half = body.len() as u64 / 2;
            Resource::new(body).drop_after(half)
        },
        true_sums,
    );
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    let half = zenodo.compressed.len() as u64 / 2;
    let total = zenodo.compressed.len() as u64;
    let mut config = AppConfig::default();

    let interrupted = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap_err();
    assert!(
        matches!(interrupted, StoreError::Download { .. }),
        "{interrupted}"
    );
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::Partial {
            received: half,
            total: Some(total)
        }
    );
    let resumable = rig
        .store
        .zenodo_offer(&client(), &zenodo.source())
        .await
        .unwrap();
    assert_eq!(resumable.resume_from, half);
    assert_eq!(
        resumable.need_bytes,
        total - half + SIRIL_GAIA_DECODED_MAX_BYTES
    );

    let observer = recorder();
    let outcome = rig
        .store
        .install_siril_from_zenodo(
            &client(),
            &zenodo.source(),
            Some(&consent),
            observer.clone(),
        )
        .await
        .unwrap();
    let installed = rig.installed(SIRIL_GAIA_ID);
    assert_eq!(outcome.id, SIRIL_GAIA_ID);
    assert_eq!(outcome.path, installed);
    assert_eq!(outcome.resumed_from, half);
    assert_eq!(outcome.bytes, zenodo.decoded.len() as u64);
    assert_eq!(outcome.sha256_hex, hex_of(&digest(&zenodo.decoded)));
    assert!(outcome.sha256_verified);
    assert!(!outcome.already_installed);
    assert!(
        fs::read(&installed).unwrap() == zenodo.decoded,
        "the decoded catalogue differs"
    );
    for leftover in [
        ".bz2",
        ".bz2.done.json",
        ".bz2.part",
        ".bz2.part.json",
        ".decoding",
    ] {
        assert!(
            !sibling(&installed, leftover).exists(),
            "{leftover} was left behind"
        );
    }
    let archive_requests = zenodo.archive.requests();
    assert_eq!(archive_requests.len(), 2);
    assert_eq!(
        archive_requests[1].header("range"),
        Some(format!("bytes={half}-").as_str())
    );
    assert_eq!(
        observer.stages().last().copied(),
        Some((InstallStage::Decode, zenodo.decoded.len() as u64, None))
    );

    config
        .external_catalogs
        .insert(SIRIL_GAIA_ID.to_owned(), "D:/old/choice.dat".to_owned());
    let registered = rig.store.use_downloaded_siril(&mut config).unwrap();
    assert_eq!(registered, installed);
    assert_eq!(
        config.siril_catalog_dir.as_deref(),
        installed.parent().and_then(Path::to_str)
    );
    assert!(config.external_catalogs.is_empty());
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::External {
            path: installed.clone(),
            valid: true,
            problem: None
        }
    );
    let lease = rig.store.open(SIRIL_GAIA_ID, &config, PLATE_SOLVE).unwrap();
    assert_eq!(lease.path(), installed);
    drop(lease);

    let again = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap();
    assert!(again.already_installed);
    assert_eq!(again.sha256_hex, hex_of(&digest(&zenodo.decoded)));
    assert_eq!(
        zenodo.archive.requests().len(),
        2,
        "a verified install was downloaded again"
    );
    println!(
        "Zenodo resume: archive {total} bytes, dropped at {half}, resumed with {:?}, decoded {} bytes",
        archive_requests[1].header("range"),
        outcome.bytes
    );
}

#[tokio::test]
async fn a_zenodo_archive_that_differs_from_its_published_sha256_is_refused_naming_both() {
    let published = digest(b"a different archive");
    let zenodo = zenodo_rig(Resource::new, |_, decoded| {
        Some((published, digest(decoded)))
    });
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    assert_eq!(
        consent.archive_sha256.as_deref(),
        Some(hex_of(&published).as_str())
    );

    let refused = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(message.contains(&hex_of(&published)), "{message}");
    assert!(
        message.contains(&hex_of(&digest(&zenodo.compressed))),
        "{message}"
    );
    let installed = rig.installed(SIRIL_GAIA_ID);
    for leftover in ["", ".bz2", ".bz2.part", ".bz2.done.json"] {
        assert!(
            !sibling(&installed, leftover).exists(),
            "{leftover} was left behind"
        );
    }
    assert_eq!(
        rig.store
            .status(SIRIL_GAIA_ID, &AppConfig::default())
            .unwrap(),
        DatasetStatus::Missing
    );
    println!("Zenodo checksum mismatch: {message}");
}

#[tokio::test]
async fn a_decoded_zenodo_catalogue_that_differs_from_its_published_sha256_is_refused() {
    let published = digest(b"a different catalogue");
    let zenodo = zenodo_rig(Resource::new, |archive, _| {
        Some((digest(archive), published))
    });
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;

    let refused = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(matches!(refused, StoreError::Checksum { .. }), "{message}");
    assert!(message.contains(&hex_of(&published)), "{message}");
    assert!(
        message.contains(&hex_of(&digest(&zenodo.decoded))),
        "{message}"
    );
    let installed = rig.installed(SIRIL_GAIA_ID);
    assert!(!installed.exists());
    assert!(!sibling(&installed, ".decoding").exists());
}

#[tokio::test]
async fn a_zenodo_download_cancelled_midway_keeps_its_part_and_reports_partial() {
    let zenodo = zenodo_rig(
        |body| {
            let rate = body.len() as u64;
            Resource::new(body).slow_steady(rate)
        },
        true_sums,
    );
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    let total = zenodo.compressed.len() as u64;
    let observer = Recorder::cancelling();

    let cancelled = rig
        .store
        .install_siril_from_zenodo(
            &client(),
            &zenodo.source(),
            Some(&consent),
            observer.clone(),
        )
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
    assert!(flagged > 0 && flagged < total, "{flagged} of {total}");
    assert_eq!(observer.calls_after_cancel.load(Ordering::SeqCst), 0);
    let part = sibling(&rig.installed(SIRIL_GAIA_ID), ".bz2.part");
    assert_eq!(fs::metadata(&part).unwrap().len(), flagged);
    assert!(fs::read(&part).unwrap() == zenodo.compressed[..flagged as usize]);
    assert_eq!(
        rig.store
            .status(SIRIL_GAIA_ID, &AppConfig::default())
            .unwrap(),
        DatasetStatus::Partial {
            received: flagged,
            total: Some(total)
        }
    );
    assert!(observer
        .stages()
        .iter()
        .all(|(stage, _, seen)| *stage == InstallStage::Download && *seen == Some(total)));
    println!(
        "Zenodo cancel: kept {flagged} of {total} bytes in {}",
        part.display()
    );
}

#[tokio::test]
async fn a_zenodo_record_without_sums_installs_on_its_size_alone_and_says_so() {
    let zenodo = zenodo_rig(Resource::new, |_, _| None);
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    assert_eq!(consent.archive_sha256, None);

    let outcome = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap();
    assert!(!outcome.sha256_verified);
    assert_eq!(outcome.sha256_hex, hex_of(&digest(&zenodo.decoded)));
    assert!(fs::read(&outcome.path).unwrap() == zenodo.decoded);

    let truncated = zenodo_rig(
        |body| {
            let short = body[..body.len() - 1].to_vec();
            Resource::new(short)
        },
        |_, _| None,
    );
    let other = Rig::new();
    let consent = zenodo_consent(&other, &truncated).await;
    let refused = other
        .store
        .install_siril_from_zenodo(&client(), &truncated.source(), Some(&consent), recorder())
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains(&truncated.compressed.len().to_string()),
        "{refused}"
    );
    assert!(!other.installed(SIRIL_GAIA_ID).exists());
}

#[tokio::test]
async fn an_archive_that_decodes_past_the_cap_is_refused_and_leaves_no_catalogue() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    let cap = zenodo.decoded.len() as u64 - 1;
    let source = ZenodoSource {
        decoded_max_bytes: cap,
        ..zenodo.source()
    };

    let refused = rig
        .store
        .install_siril_from_zenodo(&client(), &source, Some(&consent), recorder())
        .await
        .unwrap_err();
    let message = refused.to_string();
    assert!(message.contains(&cap.to_string()), "{message}");
    let installed = rig.installed(SIRIL_GAIA_ID);
    assert!(!installed.exists());
    assert!(!sibling(&installed, ".decoding").exists());
}

#[tokio::test]
async fn a_zenodo_record_off_its_host_or_without_the_archive_is_refused() {
    let record_url = Url::parse("http://127.0.0.1:9/api/records/14692304").unwrap();
    let elsewhere = zenodo_record(vec![zenodo_file(
        SIRIL_GAIA_ARCHIVE_KEY,
        10,
        "https://elsewhere.example/siril_cat_healpix8_astro.dat.bz2",
    )]);
    let refused = parse_zenodo_record(&record_url, &elsewhere).unwrap_err();
    assert!(
        refused.contains("elsewhere.example") && refused.contains("127.0.0.1"),
        "{refused}"
    );
    let plain_http = zenodo_record(vec![zenodo_file(
        SIRIL_GAIA_ARCHIVE_KEY,
        10,
        "http://zenodo.org/api/records/14692304/files/x/content",
    )]);
    let refused = parse_zenodo_record(&record_url, &plain_http).unwrap_err();
    assert!(refused.contains("https"), "{refused}");
    let without_archive = zenodo_record(vec![zenodo_file(
        SIRIL_GAIA_DECODED_SUM_KEY,
        95,
        "http://127.0.0.1:9/api/records/14692304/files/uncompressed_files.sha256sum/content",
    )]);
    let refused = parse_zenodo_record(&record_url, &without_archive).unwrap_err();
    assert!(
        refused.contains(SIRIL_GAIA_ARCHIVE_KEY) && refused.contains(SIRIL_GAIA_DECODED_SUM_KEY),
        "{refused}"
    );
    let refused = parse_zenodo_record(&record_url, b"<html>maintenance</html>").unwrap_err();
    assert!(refused.contains("JSON"), "{refused}");

    let rig = Rig::new();
    let remote_http = ZenodoSource {
        record_url: "http://zenodo.org/api/records/14692304".to_owned(),
        decoded_max_bytes: SIRIL_GAIA_DECODED_MAX_BYTES,
    };
    let refused = rig
        .store
        .zenodo_offer(&client(), &remote_http)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("https"), "{refused}");
}

#[test]
fn the_sha256sum_line_naming_the_file_gives_its_digest() {
    let wanted = digest(b"siril");
    let line = format!("{}  {SIRIL_ASTROMETRIC_FILE}\n", hex_of(&wanted));
    assert_eq!(parse_sha256sum(&line, SIRIL_ASTROMETRIC_FILE), Some(wanted));
    let binary = format!(
        "{} *{SIRIL_GAIA_ARCHIVE_KEY}\r\n",
        hex_of(&wanted).to_ascii_uppercase()
    );
    assert_eq!(
        parse_sha256sum(&binary, SIRIL_GAIA_ARCHIVE_KEY),
        Some(wanted)
    );
    let two = format!(
        "{}  other.dat\n{}  {SIRIL_ASTROMETRIC_FILE}\n",
        hex_of(&digest(b"other")),
        hex_of(&wanted)
    );
    assert_eq!(parse_sha256sum(&two, SIRIL_ASTROMETRIC_FILE), Some(wanted));
    assert_eq!(parse_sha256sum(&line, SIRIL_GAIA_ARCHIVE_KEY), None);
    assert_eq!(
        parse_sha256sum("abc  siril_cat_healpix8_astro.dat", SIRIL_ASTROMETRIC_FILE),
        None
    );
    assert_eq!(parse_sha256sum("", SIRIL_ASTROMETRIC_FILE), None);
}

#[test]
fn the_zenodo_record_comes_from_the_zenodo_api_unless_a_loopback_override_is_set() {
    assert_eq!(
        DEFAULT_ZENODO_RECORD_URL,
        format!("https://zenodo.org/api/records/{ZENODO_RECORD_ID}")
    );
    assert_eq!(ENV_ZENODO_RECORD_URL, "ASTROBURST_ZENODO_RECORD_URL");
    assert_eq!(zenodo_source(None).record_url, DEFAULT_ZENODO_RECORD_URL);
    assert_eq!(
        zenodo_source(Some(OsString::from("  "))).record_url,
        DEFAULT_ZENODO_RECORD_URL
    );
    let loopback = zenodo_source(Some(OsString::from(
        " http://127.0.0.1:8765/api/records/14692304 ",
    )));
    assert_eq!(
        loopback.record_url,
        "http://127.0.0.1:8765/api/records/14692304"
    );
    assert_eq!(loopback.decoded_max_bytes, SIRIL_GAIA_DECODED_MAX_BYTES);
    assert_eq!(
        CatType::Astrometric.record_len(),
        Some(ASTROMETRIC_RECORD_BYTES as usize)
    );
    assert_eq!(SIRIL_GAIA_DECODED_MAX_BYTES, 1_601_175_680);
    assert_eq!(SIRIL_GAIA_BZ2_BYTES, 1_142_276_749);
}

#[tokio::test]
#[ignore]
async fn the_real_zenodo_record_lists_the_siril_archive_and_its_published_sums() {
    let rig = Rig::new();
    let offer = rig
        .store
        .zenodo_offer(&client(), &zenodo_source(None))
        .await
        .unwrap();
    assert_eq!(offer.host, "zenodo.org");
    assert_eq!(offer.archive_bytes, SIRIL_GAIA_BZ2_BYTES);
    assert_eq!(
        offer.archive_sha256.as_deref(),
        Some("846ad4b12c50865df0cb8c5b23453f22eec78bbe9969e17d669ae19eb49d421f")
    );
    assert_eq!(
        offer.decoded_sha256.as_deref(),
        Some("2fa40c93fe115235d35c5050757f2ef60a326a6f3030f87be1598c016fcb2388")
    );
    assert_eq!(offer.record_licence.as_deref(), Some("cc-by-4.0"));
    assert_eq!(offer.doi.as_deref(), Some("10.5281/zenodo.14692304"));
    assert!(!siril_dir(&rig).exists());
    println!(
        "real Zenodo record: {} files {:?}, archive {} bytes at {}, sha256 {:?}, decoded sha256 {:?}",
        offer.files.len(),
        offer.files.iter().map(|file| file.key.as_str()).collect::<Vec<_>>(),
        offer.archive_bytes,
        offer.archive_url,
        offer.archive_sha256,
        offer.decoded_sha256
    );
}

fn siril_row(rig: &Rig, config: &AppConfig) -> DatasetReport {
    rig.store
        .report(config)
        .into_iter()
        .find(|row| row.id == SIRIL_GAIA_ID)
        .unwrap()
}

#[tokio::test]
async fn forget_on_the_users_own_siril_choice_keeps_the_zenodo_download() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();
    let consent = zenodo_consent(&rig, &zenodo).await;
    rig.store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap();
    let mut config = AppConfig::default();
    let downloaded = rig.store.use_downloaded_siril(&mut config).unwrap();
    let folder = siril_dir(&rig);
    let (kept_bytes, _) = dir_usage(&folder);
    assert!(kept_bytes > zenodo.decoded.len() as u64);
    assert_eq!(siril_row(&rig, &config).folder_bytes, kept_bytes);

    let own = rig.path("own-siril");
    let own_file = own.join(SIRIL_ASTROMETRIC_FILE);
    write_file(&own_file, &siril_like_catalogue());
    rig.store.use_siril_dir(&own, &mut config).unwrap();
    let row = siril_row(&rig, &config);
    assert_eq!(row.folder_bytes, kept_bytes);
    assert_eq!(
        row.status,
        DatasetStatus::External {
            path: own_file.clone(),
            valid: true,
            problem: None
        }
    );

    let removed = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap();
    assert_eq!((removed.freed_bytes, removed.forgot_external), (0, true));
    assert_eq!(config.siril_catalog_dir, None);
    assert!(
        downloaded.is_file() && fs::read(&downloaded).unwrap() == zenodo.decoded,
        "Forget on the user's own folder deleted the Zenodo download"
    );
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::Installed {
            path: downloaded.clone(),
            verified: true
        }
    );

    rig.store.use_downloaded_siril(&mut config).unwrap();
    rig.store
        .use_external_file(SIRIL_GAIA_ID, &own_file, &mut config)
        .unwrap();
    let removed = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap();
    assert_eq!((removed.freed_bytes, removed.forgot_external), (0, true));
    assert!(config.external_catalogs.is_empty());
    assert_eq!(config.siril_catalog_dir.as_deref(), folder.to_str());
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::External {
            path: downloaded.clone(),
            valid: true,
            problem: None
        }
    );

    let removed = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap();
    assert_eq!(
        (removed.freed_bytes, removed.forgot_external),
        (kept_bytes, false)
    );
    assert_eq!(config.siril_catalog_dir, None);
    assert!(!folder.exists());
    assert!(own_file.is_file(), "the user's own file was deleted");
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::Missing
    );
    println!(
        "Forget kept the {kept_bytes}-byte Zenodo download twice, then Remove freed {} bytes",
        removed.freed_bytes
    );
}

#[test]
fn forget_on_the_users_own_siril_folder_keeps_a_partial_zenodo_download() {
    let rig = Rig::new();
    let archive = sibling(&rig.installed(SIRIL_GAIA_ID), BZIP2_SUFFIX);
    write_file(&sibling(&archive, ".part"), &[7; 1000]);
    write_file(
        &sibling(&archive, ".part.json"),
        br#"{"version":1,"url":"https://zenodo.org/x","etag":null,"last_modified":null,"expected_size":4096,"received":1000,"started_unix_s":1}"#,
    );
    let own = rig.path("own-siril");
    write_file(&own.join(SIRIL_ASTROMETRIC_FILE), &siril_like_catalogue());
    let mut config = AppConfig::default();
    rig.store.use_siril_dir(&own, &mut config).unwrap();
    assert!(siril_row(&rig, &config).folder_bytes >= 1000);

    let removed = rig.store.remove(SIRIL_GAIA_ID, &mut config).unwrap();
    assert_eq!((removed.freed_bytes, removed.forgot_external), (0, true));
    assert_eq!(
        rig.store.status(SIRIL_GAIA_ID, &config).unwrap(),
        DatasetStatus::Partial {
            received: 1000,
            total: Some(4096)
        }
    );
}

#[tokio::test]
async fn a_decode_left_by_a_crash_does_not_count_against_the_free_space_of_the_next_install() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let Rig { dir, store } = Rig::new();
    let folder = store.dataset_dir(store.spec(SIRIL_GAIA_ID).unwrap());
    let stale = sibling(&folder.join(SIRIL_ASTROMETRIC_FILE), ".decoding");
    write_file(&stale, &vec![0; MIB as usize]);
    let need = zenodo.compressed.len() as u64 + SIRIL_GAIA_DECODED_MAX_BYTES;
    let budget = need + MIB / 2;
    let measured = folder.clone();
    let store = store.with_free_space(Arc::new(move |_: &Path| {
        Ok(budget.saturating_sub(dir_usage(&measured).0))
    }));
    let rig = Rig { dir, store };
    let consent = zenodo_consent(&rig, &zenodo).await;

    let outcome = rig
        .store
        .install_siril_from_zenodo(&client(), &zenodo.source(), Some(&consent), recorder())
        .await
        .unwrap();
    assert_eq!(outcome.bytes, zenodo.decoded.len() as u64);
    assert!(!stale.exists(), "the stale decode was left behind");
}

#[tokio::test]
async fn the_zenodo_terms_name_only_the_gaia_dr3_terms_and_the_offer_carries_the_record_licence() {
    let zenodo = zenodo_rig(Resource::new, true_sums);
    let rig = Rig::new();
    let offer = rig
        .store
        .zenodo_offer(&client(), &zenodo.source())
        .await
        .unwrap();
    assert!(!GAIA_TERMS.contains("CC BY 4.0"), "{GAIA_TERMS}");
    assert!(GAIA_TERMS.contains("CC BY-NC 3.0 IGO"), "{GAIA_TERMS}");
    assert_eq!(offer.record_licence.as_deref(), Some("cc-by-4.0"));
    assert_eq!(offer.host, "127.0.0.1");
    assert_eq!(
        url_host(&offer.archive_url).as_deref(),
        Some(offer.host.as_str())
    );
}

fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn an_unreachable_zenodo_record_host_is_named_once_with_an_english_reason() {
    let port = closed_port();
    let rig = Rig::new();
    let source = ZenodoSource {
        record_url: format!("http://127.0.0.1:{port}{ZENODO_RECORD_PATH}"),
        decoded_max_bytes: SIRIL_GAIA_DECODED_MAX_BYTES,
    };

    let err = rig
        .store
        .zenodo_offer(&client(), &source)
        .await
        .unwrap_err()
        .to_string();

    assert_eq!(
        err,
        format!("siril-gaia-astro-v1: 127.0.0.1:{port} refused the connection; check the internet connection or a firewall, then try again")
    );
    assert!(!err.contains("http://") && !err.contains("os error"), "{err}");
}

#[tokio::test]
async fn a_zenodo_sum_list_that_cannot_be_read_names_the_file_and_its_host() {
    let closed = closed_port();
    let link = |key: &str| format!("http://127.0.0.1:{closed}{ZENODO_RECORD_PATH}/files/{key}/content");
    let record = serve_body(&zenodo_record(vec![
        zenodo_file(SIRIL_GAIA_ARCHIVE_KEY, 1000, &link(SIRIL_GAIA_ARCHIVE_KEY)),
        zenodo_file(SIRIL_GAIA_ARCHIVE_SUM_KEY, 100, &link(SIRIL_GAIA_ARCHIVE_SUM_KEY)),
    ]));
    let rig = Rig::new();
    let source = ZenodoSource {
        record_url: record.url(ZENODO_RECORD_PATH).to_string(),
        decoded_max_bytes: SIRIL_GAIA_DECODED_MAX_BYTES,
    };

    let err = rig
        .store
        .zenodo_offer(&client(), &source)
        .await
        .unwrap_err()
        .to_string();

    assert_eq!(
        err,
        format!("siril-gaia-astro-v1: {SIRIL_GAIA_ARCHIVE_SUM_KEY}: 127.0.0.1:{closed} refused the connection; check the internet connection or a firewall, then try again")
    );
}
