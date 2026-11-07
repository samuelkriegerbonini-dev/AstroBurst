use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use astroburst_lib::core::stacking::calibration::stack_from_paths_with_memory;
use astroburst_lib::core::synth::pipeline::{self, FrameVariation, SynthConfig};
use astroburst_lib::infra::image_source::{
    is_dq_name, is_err_name, list_planes, load_plane, plane_ref,
};
use astroburst_lib::infra::progress::{ProgressHandle, ProgressPayload};
use astroburst_lib::service::cubeprep::filter_registry;
use astroburst_lib::types::constants::STAGE_LOAD_FRAME;
use astroburst_lib::types::error::AppError as CoreError;
use astroburst_lib::types::header::HduHeader;
use astroburst_lib::types::image_ref::ImageRef;
use astroburst_lib::types::stacking::StackConfig;
use astroburst_lib::types::ImageStats;
use ndarray::Array2;

use super::config::ScienceConfig;
use super::error::ScienceError;
#[cfg(windows)]
use super::guard::names_a_device_or_foreign_share;
use super::jobs::{run_science_job, Finished, Product};
use super::outcome::{JobOutcome, FINISHED_JOBS_PER_SESSION};
use super::resolve::{require_file_source, resolve_ref};
use super::state::ScienceState;
use crate::config::ServerConfig;
use crate::error::AppError;
use crate::job::{new_job, Job, JobStatus, SseEvent};
use crate::route_docs::registered_routes;
use crate::router::{build_router, build_router_with};
use crate::session::Session;
use crate::state::AppState;
use crate::tests::v2_fixtures::{write_mef_with_dq, write_pixels_fits, DQ_HIGH_PIXEL};
use crate::v2::images::register_and_respond;

const TOKEN: &str = "s3cret-token-0123456789";
const MAST_TOKEN: &str = "mast-token-abcdef9876543210";
const MANIFEST: &str = include_str!("../../../Cargo.toml");

fn cfg_with(bind: &str, vars: &[(&str, &str)]) -> ScienceConfig {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    ScienceConfig::from_lookup(bind.parse().unwrap(), move |name| {
        vars.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    })
}

fn guarded_cfg(root: &Path) -> ScienceConfig {
    cfg_with(
        "0.0.0.0:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_SERVER_ROOT", root.to_str().unwrap()),
        ],
    )
}

fn test_state() -> AppState {
    AppState::new(Arc::new(ServerConfig::default()))
}

fn science_app(cfg: ScienceConfig, state: &AppState) -> Router {
    build_router_with(state.clone(), Arc::new(ScienceState::new(cfg)))
}

fn req(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "127.0.0.1")
}

fn authorized(method: Method, uri: &str) -> axum::http::request::Builder {
    req(method, uri).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
}

async fn send(app: &Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn health(app: &Router, request: axum::http::request::Builder) -> Response {
    send(app, request.body(Body::empty()).unwrap()).await
}

fn fixture(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    write_pixels_fits(&path, 2, 2, &[1.0, 2.0, 3.0, 4.0]);
    path
}

fn open_uris(sid: &str) -> [String; 2] {
    [
        format!("/sessions/{sid}/fits/open"),
        format!("/v2/sessions/{sid}/open"),
    ]
}

async fn open(app: &Router, uri: &str, path: &str) -> Response {
    send(
        app,
        authorized(Method::POST, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "path": path }).to_string()))
            .unwrap(),
    )
    .await
}

fn loopback_cfg(root: &Path) -> ScienceConfig {
    cfg_with(
        "127.0.0.1:8080",
        &[("ASTROBURST_SERVER_ROOT", root.to_str().unwrap())],
    )
}

async fn refusal(error: ScienceError) -> (StatusCode, Value) {
    let response = error.into_response();
    (response.status(), body_json(response).await)
}

fn derived(session: &Session, name: &str) {
    session.cache.insert_synthetic(
        name,
        Arc::new(Array2::<f32>::zeros((2, 3))),
        ImageStats::default(),
    );
    let entry = session.cache.get(name).unwrap();
    register_and_respond(session, name.into(), None, None, &entry, Value::Null);
}

async fn post(app: &Router, uri: &str, body: Value) -> Response {
    send(
        app,
        req(Method::POST, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

fn profile(name: &str) -> Vec<(String, String)> {
    let heading = format!("[profile.{name}]");
    MANIFEST
        .lines()
        .skip_while(|line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) {
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_file(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link)
        .unwrap_or_else(|e| panic!("cannot link {}: {e}", link.display()));
}

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn science_config_debug_never_prints_secrets() {
    let cfg = cfg_with(
        "0.0.0.0:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_MAST_TOKEN", MAST_TOKEN),
        ],
    );
    assert!(cfg.token.is_some());
    #[cfg(feature = "mast")]
    assert!(cfg.mast.token.is_some());
    for printed in [format!("{cfg:?}"), format!("{cfg:#?}")] {
        assert!(!printed.contains(TOKEN), "{printed}");
        assert!(!printed.contains(MAST_TOKEN), "{printed}");
        assert!(printed.contains("***"), "{printed}");
    }
}

#[tokio::test]
async fn coded_error_renders_lees_envelope_with_retry_after() {
    let response = ScienceError::unavailable(
        "mast_rate_limited",
        "MAST is rate limiting this server",
        Some(7),
    )
    .into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "7");
    assert_eq!(
        body_json(response).await,
        json!({ "success": false, "error": { "code": "mast_rate_limited", "message": "MAST is rate limiting this server" } })
    );

    let coded =
        ScienceError::bad_request("bad_request", "no such thing", Some("try another".into()))
            .into_response();
    let lees = AppError::BadRequestWithHint {
        code: "bad_request",
        message: "no such thing".into(),
        hint: Some("try another".into()),
    }
    .into_response();
    assert_eq!(coded.status(), lees.status());
    assert!(coded.headers().get(header::RETRY_AFTER).is_none());
    assert_eq!(body_json(coded).await, body_json(lees).await);

    let wrapped = ScienceError::from(AppError::NotFound(
        "image ref x not found in session".into(),
    ))
    .into_response();
    assert_eq!(wrapped.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(wrapped).await["error"]["code"], "not_found");
}

#[tokio::test]
async fn science_routes_do_not_overlap_lees() {
    let response = send(
        &build_router(test_state()),
        req(Method::GET, "/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_non_loopback_bind_answers_401_without_the_token_and_passes_with_it() {
    let state = test_state();
    let app = science_app(
        cfg_with("0.0.0.0:8080", &[("ASTROBURST_TOKEN", TOKEN)]),
        &state,
    );

    let refused = health(&app, req(Method::GET, "/health")).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(refused.headers()[header::WWW_AUTHENTICATE], "Bearer");
    let body = body_json(refused).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(!body.to_string().contains(TOKEN));

    let refused = health(&app, req(Method::POST, "/v2/sessions")).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let refused = health(&app, req(Method::POST, "/sessions")).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(state.sessions.is_empty());

    assert_eq!(
        health(&app, authorized(Method::GET, "/health"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        health(&app, authorized(Method::POST, "/v2/sessions"))
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        health(&app, authorized(Method::POST, "/sessions"))
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(state.sessions.len(), 2);
}

#[tokio::test]
async fn a_wrong_token_or_scheme_is_401() {
    let app = science_app(
        cfg_with("0.0.0.0:8080", &[("ASTROBURST_TOKEN", TOKEN)]),
        &test_state(),
    );
    let same_length: String = TOKEN.chars().rev().collect();
    let refused = [
        format!("Bearer {same_length}"),
        "Bearer short".to_owned(),
        format!("Bearer {}", &TOKEN[..TOKEN.len() - 1]),
        format!("Bearer {TOKEN}x"),
        format!("Bearer {TOKEN} extra"),
        format!("Basic {TOKEN}"),
        "Bearer".to_owned(),
        "Bearer ".to_owned(),
        TOKEN.to_owned(),
    ];
    for value in refused {
        let response = health(
            &app,
            req(Method::GET, "/health").header(header::AUTHORIZATION, &value),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{value}");
        let text = body_json(response).await.to_string();
        assert!(
            !text.contains(TOKEN) && !text.contains(&same_length),
            "{text}"
        );
    }
    for value in [format!("bearer {TOKEN}"), format!("Bearer   {TOKEN}  ")] {
        let response = health(
            &app,
            req(Method::GET, "/health").header(header::AUTHORIZATION, &value),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{value}");
    }
}

#[tokio::test]
async fn a_non_loopback_bind_without_a_token_is_503_guard_unconfigured() {
    let app = science_app(cfg_with("0.0.0.0:8080", &[]), &test_state());
    for request in [
        req(Method::GET, "/health"),
        authorized(Method::GET, "/health"),
    ] {
        let response = health(&app, request).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "guard_unconfigured");
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ASTROBURST_TOKEN"));
    }
}

#[tokio::test]
async fn a_foreign_or_missing_host_is_421_on_a_non_loopback_bind() {
    let cfg = cfg_with(
        "192.0.2.10:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            (
                "ASTROBURST_ALLOWED_HOSTS",
                "astro.lan, [2001:db8::5]:9000, ::7,bad host",
            ),
        ],
    );
    let app = science_app(cfg, &test_state());
    let bearer = format!("Bearer {TOKEN}");
    let with_host = |host: &str| {
        Request::builder()
            .uri("/health")
            .header(header::HOST, host)
            .header(header::AUTHORIZATION, &bearer)
    };

    for host in [
        "evil.example",
        "evil.example:8080",
        "127.0.0.1.evil.example",
        "localhost:80x",
        "[::1",
        "bad host",
        "::1",
    ] {
        let response = health(&app, with_host(host)).await;
        assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST, "{host}");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "host_not_allowed", "{host}");
        assert!(!body.to_string().contains(TOKEN));
    }
    let missing = health(
        &app,
        Request::builder()
            .uri("/health")
            .header(header::AUTHORIZATION, &bearer),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::MISDIRECTED_REQUEST);
    let absolute = health(
        &app,
        with_host("127.0.0.1").uri("http://evil.example/health"),
    )
    .await;
    assert_eq!(absolute.status(), StatusCode::MISDIRECTED_REQUEST);
    let doubled = health(
        &app,
        with_host("127.0.0.1").header(header::HOST, "evil.example"),
    )
    .await;
    assert_eq!(doubled.status(), StatusCode::MISDIRECTED_REQUEST);

    for host in [
        "127.0.0.1",
        "localhost:8080",
        "LOCALHOST",
        "localhost.",
        "[::1]:8080",
        "192.0.2.10:8080",
        "astro.lan",
        "ASTRO.LAN:8080",
        "[2001:db8::5]",
        "[::7]:8080",
    ] {
        assert_eq!(
            health(&app, with_host(host)).await.status(),
            StatusCode::OK,
            "{host}"
        );
    }
}

#[tokio::test]
async fn a_loopback_bind_stays_open_as_today() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = fixture(outside.path(), "out.fits");
    let cfg = cfg_with(
        "127.0.0.1:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_SERVER_ROOT", root.path().to_str().unwrap()),
        ],
    );
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(cfg, &state);

    let bare = Request::builder().uri("/health");
    assert_eq!(health(&app, bare).await.status(), StatusCode::OK);
    let foreign = Request::builder()
        .uri("/health")
        .header(header::HOST, "evil.example");
    assert_eq!(health(&app, foreign).await.status(), StatusCode::OK);
    for uri in open_uris(&sid) {
        let response = send(
            &app,
            Request::builder()
                .method(Method::POST)
                .uri(&uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "path": outside_file }).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }
}

#[tokio::test]
async fn build_router_guards_a_non_loopback_bind_taken_from_the_state() {
    let config = ServerConfig {
        bind: "0.0.0.0:9".parse().unwrap(),
        ..ServerConfig::default()
    };
    let app = build_router(AppState::new(Arc::new(config)));
    let response = health(&app, req(Method::GET, "/health")).await;
    assert_ne!(response.status(), StatusCode::OK);
    let code = body_json(response).await["error"]["code"].clone();
    assert!(
        code == "guard_unconfigured" || code == "unauthorized",
        "{code}"
    );
}

#[tokio::test]
async fn open_routes_refuse_a_path_outside_the_roots_on_a_non_loopback_bind() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    let inside_file = fixture(root.path(), "in.fits");
    let outside_file = fixture(outside.path(), "out.fits");
    let escape = root
        .path()
        .join("sub")
        .join("..")
        .join("..")
        .join(outside.path().file_name().unwrap())
        .join("out.fits");
    let inside_via_dots = root.path().join("sub").join("..").join("in.fits");
    let missing_inside = root.path().join("absent.fits");
    let root_name = root
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(guarded_cfg(root.path()), &state);

    for uri in open_uris(&sid) {
        for path in [&outside_file, &escape] {
            let path = path.to_str().unwrap();
            let response = open(&app, &uri, path).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri} {path}");
            let body = body_json(response).await;
            assert_eq!(body["error"]["code"], "path_outside_root", "{uri} {path}");
            assert!(body["error"]["message"].as_str().unwrap().contains(path));
            assert!(body["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("ASTROBURST_SERVER_ROOT"));
        }
        let outside_body = body_json(open(&app, &uri, outside_file.to_str().unwrap()).await).await;
        assert!(
            !outside_body.to_string().contains(&root_name),
            "{outside_body}"
        );
        for path in [&inside_file, &inside_via_dots] {
            let response = open(&app, &uri, path.to_str().unwrap()).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri} {path:?}");
        }
        let response = open(&app, &uri, missing_inside.to_str().unwrap()).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn a_link_out_of_the_root_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fixture(outside.path(), "out.fits");
    let link = root.path().join("escape");
    link_dir(outside.path(), &link);
    let through_link = link.join("out.fits");
    assert!(through_link.exists());
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(guarded_cfg(root.path()), &state);

    for uri in open_uris(&sid) {
        let response = open(&app, &uri, through_link.to_str().unwrap()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(
            body_json(response).await["error"]["code"],
            "path_outside_root"
        );
    }
}

#[tokio::test]
async fn relative_drive_relative_verbatim_and_unc_forms_cannot_leave_the_root() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let inside_file = fixture(root.path(), "in.fits");
    let outside_file = fixture(outside.path(), "out.fits");
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(guarded_cfg(root.path()), &state);
    let mut refused = vec![
        "Cargo.toml".to_owned(),
        "../Cargo.toml".to_owned(),
        "src".to_owned(),
    ];
    let mut not_refused = vec![];
    if cfg!(windows) {
        let outside_text = outside_file.to_str().unwrap().to_owned();
        let drive = &outside_text[..1];
        let cwd = std::env::current_dir().unwrap();
        let cwd_drive = &cwd.to_str().unwrap()[..2];
        refused.push(format!("{cwd_drive}Cargo.toml"));
        refused.push(format!(r"\\?\{outside_text}"));
        refused.push(format!(r"\\localhost\{drive}$\{}", &outside_text[3..]));
        not_refused.push(format!(r"\\?\{}", inside_file.to_str().unwrap()));
    }

    for uri in open_uris(&sid) {
        for path in &refused {
            let response = open(&app, &uri, path).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri} {path}");
        }
        for path in &not_refused {
            let response = open(&app, &uri, path).await;
            assert_ne!(response.status(), StatusCode::FORBIDDEN, "{uri} {path}");
        }
    }
}

#[cfg(windows)]
#[test]
fn a_share_path_passes_the_lexical_check_only_when_a_root_lies_on_that_share() {
    let roots = [
        PathBuf::from(r"\\?\C:\data"),
        PathBuf::from(r"\\?\UNC\NAS\Astro\night"),
    ];
    for path in [
        r"C:\data\x.fits",
        r"\\?\C:\data\x.fits",
        r"C:x.fits",
        r"\data\x.fits",
        r"relative\x.fits",
        r"\\nas\astro\x.fits",
        "//NAS/astro/x.fits",
        r"\\?\UNC\nas\ASTRO\night\x.fits",
    ] {
        assert!(
            !names_a_device_or_foreign_share(&roots, Path::new(path)),
            "{path}"
        );
    }
    for path in [
        r"\\nas\other\x.fits",
        r"\\evil\astro\x.fits",
        r"\\?\UNC\evil\astro\x.fits",
        r"\\nas",
        r"\\",
        r"\/evil/astro/x.fits",
        r"\\.\pipe\x",
        r"\??\UNC\nas\astro\x.fits",
        r"\\?\GLOBALROOT\Device\Mup\nas\astro\x.fits",
    ] {
        assert!(
            names_a_device_or_foreign_share(&roots, Path::new(path)),
            "{path}"
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn a_device_or_share_path_is_refused_before_the_file_system_opens_it() {
    use std::time::Instant;
    use tokio::net::windows::named_pipe::ServerOptions;

    let root = tempfile::tempdir().unwrap();
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let loopback = science_app(loopback_cfg(root.path()), &state);
    let guarded = science_app(guarded_cfg(root.path()), &state);
    let open_uri = format!("/v2/sessions/{sid}/open");
    let pipe = format!(r"\\.\pipe\astroburst-guard-{}", uuid::Uuid::new_v4());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe)
        .unwrap();

    let listed = call(
        &loopback,
        Method::GET,
        &format!("/v2/fs?dir={}", encoded(&pipe)),
    )
    .await;
    assert_eq!(listed.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        open(&guarded, &open_uri, &pipe).await.status(),
        StatusCode::FORBIDDEN
    );
    let touched = tokio::time::timeout(Duration::from_millis(200), server.connect()).await;
    assert!(
        touched.is_err(),
        "the server opened {pipe} before refusing it"
    );

    for path in [
        r"\\192.0.2.1\s",
        "//192.0.2.1/s",
        r"\\192.0.2.1",
        r"\\?\UNC\192.0.2.1\s",
        r"\??\UNC\192.0.2.1\s",
        r"\\.\UNC\192.0.2.1\s",
        r"\\?\GLOBALROOT\Device\Mup\192.0.2.1\s",
    ] {
        let started = Instant::now();
        let listed = call(
            &loopback,
            Method::GET,
            &format!("/v2/fs?dir={}", encoded(path)),
        )
        .await;
        assert_eq!(listed.status(), StatusCode::FORBIDDEN, "{path}");
        let opened = open(&guarded, &open_uri, &format!(r"{path}\x.fits")).await;
        assert_eq!(opened.status(), StatusCode::FORBIDDEN, "{path}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{path} took {:?}",
            started.elapsed()
        );
    }
}

#[tokio::test]
async fn the_output_and_mast_download_folders_are_roots_too() {
    let root = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mast = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let in_out = fixture(out.path(), "o.fits");
    let in_mast = fixture(mast.path(), "m.fits");
    let far = fixture(elsewhere.path(), "far.fits");
    let cfg = cfg_with(
        "0.0.0.0:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_SERVER_ROOT", root.path().to_str().unwrap()),
            ("ASTROBURST_OUTPUT_ROOT", out.path().to_str().unwrap()),
            (
                "ASTROBURST_MAST_DOWNLOAD_DIR",
                mast.path().to_str().unwrap(),
            ),
        ],
    );
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(cfg, &state);
    let uri = format!("/v2/sessions/{sid}/open");

    assert_eq!(
        open(&app, &uri, in_out.to_str().unwrap()).await.status(),
        StatusCode::OK
    );
    let mast_status = if cfg!(feature = "mast") {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    assert_eq!(
        open(&app, &uri, in_mast.to_str().unwrap()).await.status(),
        mast_status
    );
    assert_eq!(
        open(&app, &uri, far.to_str().unwrap()).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn v1_job_routes_take_paths_only_under_the_roots_on_a_non_loopback_bind() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let inside = fixture(root.path(), "in.fits");
    let far = fixture(outside.path(), "far.fits");
    let (inside, far) = (inside.to_str().unwrap(), far.to_str().unwrap());
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let guarded = science_app(guarded_cfg(root.path()), &state);
    let job = |route: &str, body: &Value| {
        authorized(Method::POST, &format!("/sessions/{sid}/{route}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let light = json!([{ "label": "L", "paths": [inside] }]);
    let refused = [
        (
            "stacking/stack",
            json!({ "paths": [far, far], "align": false, "rejection": "none", "combine": "max", "normalization": "none" }),
        ),
        (
            "stacking/drizzle",
            json!({ "paths": [inside, far], "align": false }),
        ),
        (
            "pipeline/run",
            json!({ "channels": [{ "label": "L", "paths": [inside] }, { "label": "R", "paths": [far] }], "align": false }),
        ),
        (
            "pipeline/run",
            json!({ "channels": light, "bias_paths": [far], "align": false }),
        ),
        (
            "pipeline/run",
            json!({ "channels": light, "dark_paths": [far], "align": false }),
        ),
        (
            "pipeline/run",
            json!({ "channels": light, "flat_paths": [far], "align": false }),
        ),
        (
            "pipeline/run",
            json!({ "channels": light, "flat_dark_paths": [far], "align": false }),
        ),
    ];

    for &(route, ref body) in &refused {
        let response = send(&guarded, job(route, body)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{route} {body}");
        let error = body_json(response).await["error"].clone();
        assert_eq!(error["code"], "path_outside_root", "{route} {body}");
        assert!(error["message"].as_str().unwrap().contains(far), "{error}");
    }
    assert!(session.jobs.is_empty());
    assert_eq!(
        state.job_semaphore.available_permits(),
        state.config.jobs_max
    );

    let inside_stack = json!({ "paths": [inside, inside], "align": false });
    assert_eq!(
        send(&guarded, job("stacking/stack", &inside_stack))
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    let loopback = science_app(loopback_cfg(root.path()), &state);
    let far_stack = json!({ "paths": [far, far], "align": false });
    let open_as_today = post(
        &loopback,
        &format!("/sessions/{sid}/stacking/stack"),
        far_stack,
    )
    .await;
    assert_eq!(open_as_today.status(), StatusCode::ACCEPTED);
    wait_for_workers(&state).await;
}

#[tokio::test]
async fn an_unusable_server_root_refuses_every_path() {
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("missing");
    let file = fixture(parent.path(), "in.fits");
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(guarded_cfg(&missing), &state);
    for uri in open_uris(&sid) {
        let response = open(&app, &uri, file.to_str().unwrap()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }
}

#[tokio::test]
async fn every_ref_route_answers_the_same_message_without_an_active_ref() {
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let routes = [
        (Method::GET, "structure", ""),
        (Method::GET, "header", ""),
        (Method::GET, "wcs", ""),
        (
            Method::POST,
            "cutout",
            r#"{"region":{"type":"pixel","x":1,"y":1,"width":2,"height":2}}"#,
        ),
        (Method::POST, "wcs/pix2sky", r#"{"points":[]}"#),
        (Method::POST, "wcs/sky2pix", r#"{"points":[]}"#),
        (Method::POST, "wcs/grid", "{}"),
        (
            Method::POST,
            "wcs/separation",
            r#"{"type":"pixel","a":[0,0],"b":[1,1]}"#,
        ),
        (Method::POST, "bin", r#"{"factor":2}"#),
        (Method::POST, "pixel", r#"{"x":0,"y":0}"#),
        (Method::POST, "stats", "{}"),
        (Method::POST, "histogram", "{}"),
        (Method::POST, "render", "{}"),
    ];
    for (method, route, body) in routes {
        let mut request = req(method, &format!("/v2/sessions/{sid}/{route}"));
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let response = send(&app, request.body(Body::from(body)).unwrap()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{route}");
        assert_eq!(
            body_json(response).await["error"],
            json!({ "code": "bad_request", "message": "no active image in this session; open a file first" }),
            "{route}"
        );
    }
}

#[tokio::test]
async fn resolve_ref_without_an_explicit_or_active_ref_is_400_with_the_exact_message() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (status, body) = refusal(resolve_ref(&session, None).await.err().unwrap()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({ "success": false, "error": { "code": "bad_request", "message": "no active image in this session; open a file first" } })
    );
}

#[tokio::test]
async fn resolve_ref_of_an_unknown_ref_is_404_and_a_known_ref_resolves_by_name_or_as_active() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (status, body) = refusal(
        resolve_ref(&session, Some("nope".into()))
            .await
            .err()
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body["error"],
        json!({ "code": "not_found", "message": "image ref nope not found in session" })
    );

    derived(&session, "cut_0");
    *session.v2.active_ref.write().await = Some("cut_0".into());
    let named = resolve_ref(&session, Some("cut_0".into())).await.unwrap();
    let active = resolve_ref(&session, None).await.unwrap();
    assert_eq!(
        (named.name.as_str(), active.name.as_str()),
        ("cut_0", "cut_0")
    );
    assert_eq!(active.entry.arr().dim(), (2, 3));
    assert_eq!(active.wcs_source, "");
}

#[tokio::test]
async fn require_file_source_refuses_a_derived_ref_and_returns_the_plane_of_a_file_ref() {
    let root = tempfile::tempdir().unwrap();
    let file = fixture(root.path(), "in.fits");
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(loopback_cfg(root.path()), &state);
    let science = ScienceState::new(loopback_cfg(root.path()));

    derived(&session, "cut_0");
    let opened = post(
        &app,
        &format!("/v2/sessions/{sid}/open"),
        json!({ "path": file, "name": "img" }),
    )
    .await;
    assert_eq!(opened.status(), StatusCode::OK);
    let binned = post(
        &app,
        &format!("/v2/sessions/{sid}/bin"),
        json!({ "factor": 1, "ref": "img", "name": "bin_0" }),
    )
    .await;
    assert_eq!(binned.status(), StatusCode::OK);

    for name in ["cut_0", "bin_0"] {
        let resolved = resolve_ref(&session, Some(name.into())).await.unwrap();
        let (status, body) = refusal(
            require_file_source(&science, &resolved)
                .await
                .err()
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}");
        assert_eq!(body["error"]["code"], "derived_ref_unsupported", "{name}");
        assert!(
            body["error"]["message"].as_str().unwrap().contains(name),
            "{body}"
        );
    }
    let resolved = resolve_ref(&session, Some("img".into())).await.unwrap();
    let source = require_file_source(&science, &resolved).await.unwrap();
    assert_eq!(source.path, file.to_str().unwrap());
}

#[tokio::test]
async fn require_file_source_refuses_a_file_outside_the_roots_on_any_bind() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let far = fixture(outside.path(), "far.fits");
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(loopback_cfg(root.path()), &state);
    let opened = post(
        &app,
        &format!("/v2/sessions/{sid}/open"),
        json!({ "path": far, "name": "far" }),
    )
    .await;
    assert_eq!(opened.status(), StatusCode::OK);

    let resolved = resolve_ref(&session, Some("far".into())).await.unwrap();
    for science in [
        ScienceState::new(loopback_cfg(root.path())),
        ScienceState::new(guarded_cfg(root.path())),
    ] {
        let (status, body) = refusal(
            require_file_source(&science, &resolved)
                .await
                .err()
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "path_outside_root");
    }
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn the_lib_fits_fixtures_reach_the_server_tests() {
    use astroburst_lib::infra::fits::reader::test_fixtures::sci_err_dq_mef;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sci_err_dq.fits");
    sci_err_dq_mef(&path, 4, 3, vec![0; 12]);
    let state = test_state();
    let sid = state.create_session().unwrap().id.clone();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let opened = post(
        &app,
        &format!("/v2/sessions/{sid}/open"),
        json!({ "path": path }),
    )
    .await;
    assert_eq!(opened.status(), StatusCode::OK);
    let body = body_json(opened).await;
    assert_eq!(body["dims"], json!([4, 3]));
    assert_eq!(body["extname"], "SCI");
}

#[cfg(all(feature = "test-support", feature = "mast"))]
#[tokio::test]
async fn the_canned_mast_server_reaches_the_server_tests() {
    use astroburst_lib::infra::mast::testing::{serve_routes, Route};

    let server = serve_routes(vec![Route::get("/ping").json(r#"{"pong":true}"#)]).await;
    let answer = reqwest::get(format!("{}/ping", server.base))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(answer, r#"{"pong":true}"#);
    assert_eq!(server.count("/ping"), 1);
}

fn science_with(cfg: ScienceConfig, state: &AppState) -> (Arc<ScienceState>, Router) {
    let science = Arc::new(ScienceState::new(cfg));
    let app = build_router_with(state.clone(), Arc::clone(&science));
    (science, app)
}

async fn get(app: &Router, uri: &str) -> Value {
    let response = send(app, req(Method::GET, uri).body(Body::empty()).unwrap()).await;
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    body_json(response).await
}

async fn cancel_job(app: &Router, session: &Session, job: &Job) -> Response {
    send(
        app,
        req(
            Method::DELETE,
            &format!("/sessions/{}/jobs/{}", session.id, job.id),
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await
}

async fn wait_for_workers(state: &AppState) {
    let every_permit = state
        .job_semaphore
        .acquire_many(state.config.jobs_max as u32);
    drop(
        tokio::time::timeout(Duration::from_secs(10), every_permit)
            .await
            .expect("every job worker exits")
            .expect("the job pool is open"),
    );
}

fn job_of(session: &Session, accepted: &Value) -> Arc<Job> {
    let id = accepted["job_id"].as_str().expect("a job id");
    session
        .jobs
        .get(id)
        .map(|entry| Arc::clone(entry.value()))
        .expect("the job is registered in its session")
}

fn header_with(cards: &[(&str, &str)]) -> HduHeader {
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key, value.to_string());
    }
    header
}

fn product(name: &str, cards: &[(&str, &str)]) -> Product {
    Product {
        name: name.to_owned(),
        image: Array2::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32),
        header: header_with(cards),
    }
}

fn finished_job(session: &Session, completed_at: u64) -> String {
    let job = new_job("synth");
    job.set_done();
    *job.completed_at.lock().unwrap() = Some(completed_at);
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    job.id.clone()
}

fn outcome(result: Value) -> JobOutcome {
    JobOutcome {
        result,
        warnings: Vec::new(),
        products: Vec::new(),
        error: None,
        error_code: None,
        error_hint: None,
    }
}

#[tokio::test]
async fn cancel_reaches_a_running_stack_job_started_through_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<String> = (0..3)
        .map(|i| {
            let path = dir.path().join(format!("frame{i}.fits"));
            write_pixels_fits(&path, 8, 8, &[100.0 + i as f32; 64]);
            path.to_str().unwrap().to_owned()
        })
        .collect();
    let state = test_state();
    let session = state.create_session().unwrap();
    let (science, app) = science_with(loopback_cfg(dir.path()), &state);
    let (paused_tx, paused_rx) = tokio::sync::oneshot::channel::<()>();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();
    let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel::<bool>();

    let (status, Json(accepted)) = run_science_job(
        &state,
        &session,
        &science,
        "stack",
        6,
        move |_, progress| {
            let gate = Mutex::new(Some((paused_tx, resume_rx)));
            let job_progress = progress.clone();
            let paused = ProgressHandle::with_sink(
                Arc::new(move |payload: &ProgressPayload| {
                    if payload.stage != STAGE_LOAD_FRAME {
                        return;
                    }
                    if let Some((paused, resume)) = gate.lock().unwrap().take() {
                        let _ = paused.send(());
                        let _ = resume.recv();
                    }
                }),
                6,
            )
            .with_cancel_source(Arc::new(move || job_progress.is_cancelled()));
            let config = StackConfig {
                align: false,
                ..StackConfig::default()
            };
            let stacked = stack_from_paths_with_memory(&paths, &config, Some(&paused), None);
            let cancelled = matches!(
                stacked
                    .as_ref()
                    .err()
                    .and_then(|e| e.downcast_ref::<CoreError>()),
                Some(CoreError::Cancelled)
            );
            let _ = stopped_tx.send(cancelled);
            let stacked = stacked?;
            Ok(Finished {
                result: json!({ "frame_count": stacked.frame_count }),
                warnings: stacked.warnings,
                products: vec![Product {
                    name: "stacked".into(),
                    image: stacked.image,
                    header: HduHeader::empty(),
                }],
            })
        },
    )
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    let job = job_of(&session, &accepted);

    paused_rx
        .await
        .expect("the stack loads its first frame and reports it");
    let cancelled = cancel_job(&app, &session, &job).await;
    assert_eq!(cancelled.status(), StatusCode::OK);
    assert_eq!(body_json(cancelled).await["status"], "cancelled");
    resume_tx.send(()).unwrap();

    assert!(
        stopped_rx.await.unwrap(),
        "the stack went on after its job was cancelled"
    );
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Cancelled);
    assert!(job.pct.load(std::sync::atomic::Ordering::Acquire) < 100);
    assert!(!session.cache.contains("stacked"));
    assert!(session.v2.meta.is_empty());
    assert!(science.outcomes.get(&session.id, &job.id).is_none());
}

#[tokio::test]
async fn a_science_job_publishes_its_products_to_lees_routes_and_records_its_outcome() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let (science, app) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);
    let sky = [
        ("CTYPE1", "RA---TAN"),
        ("CTYPE2", "DEC--TAN"),
        ("CRVAL1", "83.8"),
        ("CRVAL2", "-5.4"),
        ("CRPIX1", "4.5"),
        ("CRPIX2", "4.5"),
        ("CD1_1", "-0.0001"),
        ("CD1_2", "0.0"),
        ("CD2_1", "0.0"),
        ("CD2_2", "0.0001"),
    ];

    let (_, Json(accepted)) = run_science_job(
        &state,
        &session,
        &science,
        "synth",
        2,
        move |_, progress| {
            progress.report(1, 2, "render");
            Ok(Finished {
                result: json!({ "stars": 5 }),
                warnings: vec!["few stars".to_owned()],
                products: vec![
                    product("plain_0", &[("EXPTIME", "300")]),
                    product("sky_0", &sky),
                ],
            })
        },
    )
    .unwrap();
    let job = job_of(&session, &accepted);
    let mut events = job.rx.lock().unwrap().take().unwrap();
    wait_for_workers(&state).await;

    assert_eq!(job.current_status(), JobStatus::Done);
    let recorded = science
        .outcomes
        .get(&sid, &job.id)
        .expect("the outcome is recorded");
    assert_eq!(recorded.result, json!({ "stars": 5 }));
    assert_eq!(recorded.warnings, ["few stars"]);
    assert_eq!(recorded.products, ["plain_0", "sky_0"]);
    assert_eq!(recorded.error, None);
    let reported: Vec<(u32, String)> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            SseEvent::Progress { pct, stage } => Some((pct, stage)),
            _ => None,
        })
        .collect();
    assert!(
        reported.contains(&(50, "render".to_owned())),
        "{reported:?}"
    );

    let images = get(&app, &format!("/v2/sessions/{sid}/images")).await;
    let listed: Vec<Value> = images["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            json!([
                i["image_ref"],
                i["width"],
                i["height"],
                i["wcs_present"],
                i["source"]
            ])
        })
        .collect();
    assert_eq!(
        listed,
        [
            json!(["plain_0", 8, 8, false, null]),
            json!(["sky_0", 8, 8, true, null])
        ]
    );
    let header = get(&app, &format!("/v2/sessions/{sid}/header?ref=plain_0")).await;
    assert_eq!(header["cards"]["EXPTIME"]["value"], "300");
}

#[tokio::test]
async fn a_failed_science_job_records_the_error_text_it_sends_on_the_stream() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (science, _) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);

    let (_, Json(accepted)) = run_science_job(&state, &session, &science, "synth", 1, |_, _| {
        Err(anyhow::anyhow!("no stars fit the field").context("synthetic field"))
    })
    .unwrap();
    let job = job_of(&session, &accepted);
    let mut events = job.rx.lock().unwrap().take().unwrap();
    wait_for_workers(&state).await;

    assert_eq!(job.current_status(), JobStatus::Error);
    let streamed = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            SseEvent::Error { message } => Some(message),
            _ => None,
        })
        .expect("an error event");
    assert_eq!(streamed, "synthetic field: no stars fit the field");
    let recorded = science
        .outcomes
        .get(&session.id, &job.id)
        .expect("the failure is recorded");
    assert_eq!(recorded.error.as_deref(), Some(streamed.as_str()));
    assert!(recorded.products.is_empty());
}

#[tokio::test]
async fn a_science_job_cancelled_before_its_body_returns_publishes_and_records_nothing() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (science, app) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

    let (_, Json(accepted)) =
        run_science_job(&state, &session, &science, "synth", 1, move |_, _| {
            let _ = release_rx.recv();
            Ok(Finished {
                result: json!({}),
                warnings: Vec::new(),
                products: vec![product("late_0", &[])],
            })
        })
        .unwrap();
    let job = job_of(&session, &accepted);
    assert_eq!(
        cancel_job(&app, &session, &job).await.status(),
        StatusCode::OK
    );
    let _ = release_tx.send(());
    wait_for_workers(&state).await;

    assert_eq!(job.current_status(), JobStatus::Cancelled);
    assert!(!session.cache.contains("late_0"));
    assert!(session.v2.meta.is_empty());
    assert!(science.outcomes.get(&session.id, &job.id).is_none());
}

#[tokio::test]
async fn outcomes_are_pruned_with_their_session_and_finished_jobs_are_capped_oldest_first() {
    let state = test_state();
    let (science, app) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);
    let kept = state.create_session().unwrap();
    let deleted = state.create_session().unwrap();
    let running = new_job("stack");
    kept.jobs.insert(running.id.clone(), Arc::clone(&running));

    let finished: Vec<String> = (0..FINISHED_JOBS_PER_SESSION + 2)
        .map(|i| {
            let jid = finished_job(&kept, 1_000 + i as u64);
            science
                .outcomes
                .record(&state, &kept.id, &jid, outcome(json!(i)));
            jid
        })
        .collect();
    let gone = finished_job(&deleted, 5);
    science
        .outcomes
        .record(&state, &deleted.id, &gone, outcome(json!("gone")));
    assert!(science.outcomes.get(&deleted.id, &gone).is_some());

    let removed = send(
        &app,
        req(Method::DELETE, &format!("/v2/sessions/{}", deleted.id))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    science.outcomes.prune(&state);

    assert!(science.outcomes.get(&deleted.id, &gone).is_none());
    for jid in &finished[..2] {
        assert!(!kept.jobs.contains_key(jid));
        assert!(science.outcomes.get(&kept.id, jid).is_none());
    }
    for (i, jid) in finished.iter().enumerate().skip(2) {
        assert_eq!(
            science
                .outcomes
                .get(&kept.id, jid)
                .map(|o| o.result.clone()),
            Some(json!(i))
        );
    }
    assert!(kept.jobs.contains_key(&running.id));
    assert_eq!(kept.jobs.len(), FINISHED_JOBS_PER_SESSION + 1);
}

#[test]
fn the_server_release_profile_unwinds_and_the_desktop_release_still_aborts() {
    let server = profile("release-server");
    assert!(
        server.contains(&("inherits".into(), "\"release\"".into())),
        "{server:?}"
    );
    assert!(
        server.contains(&("panic".into(), "\"unwind\"".into())),
        "{server:?}"
    );
    let desktop = profile("release");
    assert!(
        desktop.contains(&("panic".into(), "\"abort\"".into())),
        "{desktop:?}"
    );
}

const ROMAN_TREE: &str = "roman:\n  meta:\n    instrument: {name: WFI}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[0, 1], [0, 0]]\n    datatype: uint32\n";

async fn call(app: &Router, method: Method, uri: &str) -> Response {
    send(app, req(method, uri).body(Body::empty()).unwrap()).await
}

fn outcome_uri(session: &Session, job: &str) -> String {
    format!("/v2/sessions/{}/jobs/{job}/outcome", session.id)
}

fn bits(image: &Array2<f32>) -> Vec<u32> {
    image.iter().map(|v| v.to_bits()).collect()
}

fn encoded(text: impl AsRef<str>) -> String {
    text.as_ref()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn write_roman_asdf(path: &Path) {
    let preamble = "#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n";
    std::fs::write(path, format!("{preamble}{ROMAN_TREE}...\n")).unwrap();
}

fn listed_planes(path: &str) -> Value {
    let (planes, is_asdf) = list_planes(path).unwrap();
    planes
        .iter()
        .map(|info| {
            let name = info.extname.as_deref().unwrap_or("");
            let axes = [info.naxis4, info.naxis3, info.naxis2, info.naxis1];
            json!({
                "index": info.index,
                "extname": info.extname,
                "extver": info.extver,
                "naxis": info.naxis,
                "shape": axes[4 - info.naxis.clamp(0, 4) as usize..],
                "bitpix": info.bitpix,
                "has_data": info.has_data,
                "plane_ref": plane_ref(path, info, is_asdf).cache_key(),
                "is_dq": is_dq_name(name),
                "is_err": is_err_name(name),
            })
        })
        .collect()
}

fn summary(planes: &Value) -> Vec<Value> {
    planes
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            json!([
                p["extname"],
                p["shape"],
                p["bitpix"],
                p["is_dq"],
                p["is_err"],
                p["plane_ref"]
            ])
        })
        .collect()
}

async fn open_named(app: &Router, sid: &str, body: Value) {
    let name = body["name"].clone();
    let opened = post(app, &format!("/v2/sessions/{sid}/open"), body).await;
    assert_eq!(opened.status(), StatusCode::OK, "{name}");
}

async fn download(app: &Router, uri: &str, dir: &Path, name: &str) -> String {
    let response = call(app, Method::GET, uri).await;
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/fits");
    let length = response.headers()[header::CONTENT_LENGTH]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(bytes.len(), length);
    assert_eq!(bytes.len() % 2880, 0);
    let path = dir.join(name);
    std::fs::write(&path, &bytes).unwrap();
    path.to_str().unwrap().to_owned()
}

fn entry_names(listing: &Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect()
}

fn small_field() -> SynthConfig {
    let mut config = SynthConfig::default();
    config.field.width = 32;
    config.field.height = 24;
    config.field.n_stars = 7;
    config.field.seed = 4242;
    config.noise.seed = Some(7);
    config
}

fn assert_carries(header: &HduHeader, expected: &HduHeader, name: &str) {
    for (key, value) in &expected.cards {
        assert_eq!(header.get(key), Some(value.as_str()), "{name} {key}");
    }
}

#[tokio::test]
async fn the_outcome_route_answers_the_exact_outcome_of_a_finished_and_of_a_failed_science_job() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (science, app) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);

    let (_, Json(accepted)) = run_science_job(&state, &session, &science, "synth", 1, |_, _| {
        Ok(Finished {
            result: json!({ "stars": 5 }),
            warnings: vec!["few stars".to_owned()],
            products: vec![
                product("field_0", &[("EXPTIME", "300")]),
                product("field_1", &[]),
            ],
        })
    })
    .unwrap();
    let done = job_of(&session, &accepted);
    let (_, Json(accepted)) = run_science_job(&state, &session, &science, "synth", 1, |_, _| {
        Err(anyhow::anyhow!("no stars fit the field").context("synthetic field"))
    })
    .unwrap();
    let failed = job_of(&session, &accepted);
    wait_for_workers(&state).await;
    session.cache.remove("field_1");

    let response = call(&app, Method::GET, &outcome_uri(&session, &done.id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await,
        json!({
            "job": {
                "id": done.id,
                "action": "synth",
                "status": "done",
                "pct": 100,
                "started_at": done.started_at,
                "completed_at": done.completed_at(),
            },
            "outcome": {
                "result": { "stars": 5 },
                "warnings": ["few stars"],
                "products": [{ "ref": "field_0", "present": true }, { "ref": "field_1", "present": false }],
                "error": null,
            },
        })
    );

    let response = call(&app, Method::GET, &outcome_uri(&session, &failed.id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await,
        json!({
            "job": {
                "id": failed.id,
                "action": "synth",
                "status": "error",
                "pct": 0,
                "started_at": failed.started_at,
                "completed_at": failed.completed_at(),
            },
            "outcome": {
                "result": null,
                "warnings": [],
                "products": [],
                "error": "synthetic field: no stars fit the field",
            },
        })
    );
}

#[tokio::test]
async fn the_outcome_route_is_null_without_a_recorded_outcome_404_for_an_unknown_job_and_prunes_when_read(
) {
    let state = test_state();
    let session = state.create_session().unwrap();
    let (_, app) = science_with(cfg_with("127.0.0.1:8080", &[]), &state);
    let running = new_job("stack");
    session
        .jobs
        .insert(running.id.clone(), Arc::clone(&running));

    let response = call(&app, Method::GET, &outcome_uri(&session, &running.id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["job"]["status"], "running");
    assert_eq!(body["outcome"], Value::Null);

    let response = call(&app, Method::GET, &outcome_uri(&session, "nope")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(response).await,
        json!({ "success": false, "error": { "code": "not_found", "message": "job nope not found" } })
    );

    let finished: Vec<String> = (0..=FINISHED_JOBS_PER_SESSION)
        .map(|i| finished_job(&session, 1_000 + i as u64))
        .collect();
    let response = call(
        &app,
        Method::GET,
        &outcome_uri(&session, finished.last().unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["outcome"], Value::Null);
    assert_eq!(session.jobs.len(), FINISHED_JOBS_PER_SESSION + 1);
    assert!(!session.jobs.contains_key(&finished[0]));
    assert!(session.jobs.contains_key(&running.id));
    let response = call(&app, Method::GET, &outcome_uri(&session, &finished[0])).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_v2_job_routes_answer_as_the_v1_job_routes_with_the_same_handlers() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let job = new_job("stack");
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    job.progress(40, "loading");
    let v1 = format!("/sessions/{}/jobs/{}", session.id, job.id);
    let v2 = format!("/v2/sessions/{}/jobs/{}", session.id, job.id);

    assert_eq!(get(&app, &v2).await, get(&app, &v1).await);
    let missing = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{}/jobs/nope", session.id),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(missing).await["error"],
        json!({ "code": "not_found", "message": "job nope not found" })
    );

    let stream = call(&app, Method::GET, &format!("{v2}/stream")).await;
    assert_eq!(stream.status(), StatusCode::OK);
    assert!(stream.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let second = call(&app, Method::GET, &format!("{v1}/stream")).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let cancelled = call(&app, Method::DELETE, &v2).await;
    assert_eq!(cancelled.status(), StatusCode::OK);
    assert_eq!(body_json(cancelled).await["status"], "cancelled");
    assert_eq!(job.current_status(), JobStatus::Cancelled);
    drop(stream);
}

#[tokio::test]
async fn planes_lists_the_hdus_or_asdf_arrays_of_a_refs_source_as_list_planes_does() {
    let root = tempfile::tempdir().unwrap();
    let mef = root.path().join("jw_cal.fits");
    write_mef_with_dq(&mef, 4, 3);
    let roman = root.path().join("roman.asdf");
    write_roman_asdf(&roman);
    let outside = tempfile::tempdir().unwrap();
    let far = fixture(outside.path(), "far.fits");
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(loopback_cfg(root.path()), &state);
    open_named(&app, &sid, json!({ "path": mef, "hdu": 1, "name": "sci" })).await;
    open_named(&app, &sid, json!({ "path": roman, "name": "roman" })).await;
    open_named(&app, &sid, json!({ "path": far, "name": "far" })).await;
    derived(&session, "cut_0");

    let mef_text = mef.to_str().unwrap();
    let planes = get(&app, &format!("/v2/sessions/{sid}/planes?ref=sci")).await;
    assert_eq!(
        (
            planes["ref"].clone(),
            planes["format"].clone(),
            planes["count"].clone()
        ),
        (json!("sci"), json!("fits"), json!(4))
    );
    assert_eq!(planes["planes"], listed_planes(mef_text));
    assert_eq!(
        summary(&planes["planes"])[1..],
        [
            json!([
                "SCI",
                [3, 4],
                -32,
                false,
                false,
                format!("{mef_text}#hdu=1")
            ]),
            json!(["ERR", [3, 4], -32, false, true, format!("{mef_text}#hdu=2")]),
            json!(["DQ", [3, 4], 32, true, false, format!("{mef_text}#hdu=3")]),
        ]
    );

    let roman_text = roman.to_str().unwrap();
    let structure = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{sid}/structure?ref=roman"),
    )
    .await;
    assert_eq!(structure.status(), StatusCode::BAD_REQUEST);
    let planes = get(&app, &format!("/v2/sessions/{sid}/planes?image=roman")).await;
    assert_eq!(
        (planes["format"].clone(), planes["count"].clone()),
        (json!("asdf"), json!(3))
    );
    assert_eq!(planes["planes"], listed_planes(roman_text));
    assert_eq!(
        summary(&planes["planes"]),
        [
            json!([
                "roman",
                [2, 2],
                -32,
                false,
                false,
                format!("{roman_text}#array=roman")
            ]),
            json!([
                "roman.data",
                [2, 2],
                -32,
                false,
                false,
                format!("{roman_text}#array=roman.data")
            ]),
            json!([
                "roman.dq",
                [2, 2],
                32,
                true,
                false,
                format!("{roman_text}#array=roman.dq")
            ]),
        ]
    );

    let active_outside = call(&app, Method::GET, &format!("/v2/sessions/{sid}/planes")).await;
    assert_eq!(active_outside.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(active_outside).await["error"]["code"],
        "path_outside_root"
    );
    let derived_ref = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{sid}/planes?ref=cut_0"),
    )
    .await;
    assert_eq!(derived_ref.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(derived_ref).await["error"]["code"],
        "derived_ref_unsupported"
    );
    let unknown = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{sid}/planes?ref=nope"),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(unknown).await["error"]["message"],
        "image ref nope not found in session"
    );
}

#[tokio::test]
async fn deleting_a_ref_frees_it_from_the_cache_and_the_image_list_and_clears_it_when_active() {
    let root = tempfile::tempdir().unwrap();
    let file = fixture(root.path(), "in.fits");
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(loopback_cfg(root.path()), &state);
    derived(&session, "cut_0");
    open_named(&app, &sid, json!({ "path": file, "name": "img" })).await;

    let removed = call(
        &app,
        Method::DELETE,
        &format!("/v2/sessions/{sid}/images/cut_0"),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert!(!session.cache.contains("cut_0"));
    assert!(!session.v2.meta.contains_key("cut_0"));
    assert_eq!(session.v2.active_ref.read().await.as_deref(), Some("img"));

    let removed = call(
        &app,
        Method::DELETE,
        &format!("/v2/sessions/{sid}/images/img"),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(*session.v2.active_ref.read().await, None);
    assert!(!session.cache.contains("img"));
    let images = get(&app, &format!("/v2/sessions/{sid}/images")).await;
    assert_eq!(
        (images["count"].clone(), images["active_ref"].clone()),
        (json!(0), Value::Null)
    );
    let stats = post(
        &app,
        &format!("/v2/sessions/{sid}/stats"),
        json!({ "ref": "img" }),
    )
    .await;
    assert_eq!(stats.status(), StatusCode::NOT_FOUND);

    let again = call(
        &app,
        Method::DELETE,
        &format!("/v2/sessions/{sid}/images/img"),
    )
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(again).await["error"],
        json!({ "code": "not_found", "message": "image ref img not found in session" })
    );
}

#[tokio::test]
async fn a_ref_downloads_as_fits_that_reads_back_bit_identical_with_its_err_and_dq_planes() {
    let root = tempfile::tempdir().unwrap();
    let mef = root.path().join("jw_cal.fits");
    write_mef_with_dq(&mef, 5, 4);
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(loopback_cfg(root.path()), &state);
    open_named(&app, &sid, json!({ "path": mef, "hdu": 1, "name": "sci" })).await;
    open_named(&app, &sid, json!({ "path": mef, "hdu": 3, "name": "dq" })).await;
    let odd = Array2::from_shape_vec(
        (2, 3),
        vec![
            1.5f32,
            -0.0,
            f32::NAN,
            f32::MIN_POSITIVE,
            -3.25e7,
            f32::INFINITY,
        ],
    )
    .unwrap();
    session.cache.insert_synthetic_with_header(
        "odd",
        Arc::new(odd.clone()),
        ImageStats::default(),
        Some(header_with(&[("EXPTIME", "300")])),
    );
    let entry = session.cache.get("odd").unwrap();
    register_and_respond(&session, "odd".into(), None, None, &entry, Value::Null);

    let source = mef.to_str().unwrap();
    let plane = |path: &str, hdu: usize| load_plane(&ImageRef::hdu(path, hdu)).unwrap();
    let dq_bits = |path: &str, hdu: usize| {
        plane(path, hdu)
            .int_plane
            .expect("an integer DQ plane")
            .bits
    };
    let written = download(
        &app,
        &format!("/v2/sessions/{sid}/images/sci/fits"),
        out.path(),
        "sci.fits",
    )
    .await;
    let (hdus, _) = list_planes(&written).unwrap();
    let names: Vec<Option<&str>> = hdus.iter().map(|h| h.extname.as_deref()).collect();
    assert_eq!(names[1..], [Some("SCI"), Some("ERR"), Some("DQ")]);
    assert_eq!(
        bits(&plane(&written, 1).arr),
        bits(session.cache.get("sci").unwrap().arr())
    );
    assert_eq!(bits(&plane(&written, 1).arr), bits(&plane(source, 1).arr));
    assert_eq!(bits(&plane(&written, 2).arr), bits(&plane(source, 2).arr));
    assert_eq!(dq_bits(&written, 3), dq_bits(source, 3));

    let written = download(
        &app,
        &format!("/v2/sessions/{sid}/images/odd/fits"),
        out.path(),
        "odd.fits",
    )
    .await;
    assert_eq!(list_planes(&written).unwrap().0.len(), 1);
    let reread = load_plane(&ImageRef::parse(&written)).unwrap();
    assert_eq!(bits(&reread.arr), bits(&odd));
    assert_eq!(reread.header.get_f64("EXPTIME"), Some(300.0));

    let written = download(
        &app,
        &format!("/v2/sessions/{sid}/images/dq/fits"),
        out.path(),
        "dq.fits",
    )
    .await;
    let (hdus, _) = list_planes(&written).unwrap();
    let with_data: Vec<(usize, Option<&str>, i64)> = hdus
        .iter()
        .filter(|h| h.has_data)
        .map(|h| (h.index, h.extname.as_deref(), h.bitpix))
        .collect();
    assert_eq!(with_data, [(1, Some("DQ"), 32)]);
    let flags = dq_bits(&written, 1);
    assert_eq!(flags, dq_bits(source, 3));
    assert_eq!(flags[[DQ_HIGH_PIXEL.1, DQ_HIGH_PIXEL.0]], 2_147_483_649);

    let response = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{sid}/images/sci/fits"),
    )
    .await;
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"sci.fits\""
    );
    let missing = call(
        &app,
        Method::GET,
        &format!("/v2/sessions/{sid}/images/nope/fits"),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(missing).await["error"]["message"],
        "image ref nope not found in session"
    );
}

#[tokio::test]
async fn fs_lists_one_folder_under_the_roots_and_refuses_a_folder_outside_them() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    for name in [
        "a.fits",
        "b.FIT",
        "c.fits.gz",
        "d.fits.fz",
        "e.asdf",
        "notes.txt",
        ".hidden.fits",
    ] {
        std::fs::write(root.path().join(name), name.as_bytes()).unwrap();
    }
    std::fs::create_dir(root.path().join("night1")).unwrap();
    let canonical = std::fs::canonicalize(root.path()).unwrap();
    let root_name = root
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let state = test_state();
    let app = science_app(loopback_cfg(root.path()), &state);

    let listing = get(&app, "/v2/fs").await;
    assert_eq!(listing["dir"], canonical.to_str().unwrap());
    assert_eq!(
        (listing["count"].clone(), listing["truncated"].clone()),
        (json!(7), json!(false))
    );
    let entries: Vec<Value> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!([e["name"], e["kind"], e["bytes"]]))
        .collect();
    assert_eq!(
        entries,
        [
            json!(["a.fits", "fits", 6]),
            json!(["b.FIT", "fits", 5]),
            json!(["c.fits.gz", "fits", 9]),
            json!(["d.fits.fz", "fits", 9]),
            json!(["e.asdf", "asdf", 6]),
            json!(["night1", "dir", null]),
            json!(["notes.txt", "other", 9]),
        ]
    );
    assert!(listing["entries"][0]["modified_ms"].as_u64().unwrap() > 0);

    let filtered = get(
        &app,
        &format!(
            "/v2/fs?dir={}&glob={}",
            encoded(root.path().to_str().unwrap()),
            encoded("*.FITS")
        ),
    )
    .await;
    assert_eq!(entry_names(&filtered), ["a.fits", "night1"]);
    let night = get(
        &app,
        &format!(
            "/v2/fs?dir={}",
            encoded(root.path().join("night1").to_str().unwrap())
        ),
    )
    .await;
    assert_eq!(night["count"], 0);

    let refused = call(
        &app,
        Method::GET,
        &format!("/v2/fs?dir={}", encoded(outside.path().to_str().unwrap())),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let body = body_json(refused).await;
    assert_eq!(body["error"]["code"], "path_outside_root");
    assert!(!body.to_string().contains(&root_name), "{body}");
    let escape = root
        .path()
        .join("..")
        .join(outside.path().file_name().unwrap());
    let refused = call(
        &app,
        Method::GET,
        &format!("/v2/fs?dir={}", encoded(escape.to_str().unwrap())),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let missing = call(
        &app,
        Method::GET,
        &format!(
            "/v2/fs?dir={}",
            encoded(root.path().join("missing").to_str().unwrap())
        ),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let file = call(
        &app,
        Method::GET,
        &format!(
            "/v2/fs?dir={}",
            encoded(root.path().join("a.fits").to_str().unwrap())
        ),
    )
    .await;
    assert_eq!(file.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fs_caps_a_listing_at_5000_entries_and_says_so() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..5001 {
        std::fs::File::create(root.path().join(format!("f{i:05}.fits"))).unwrap();
    }
    let app = science_app(loopback_cfg(root.path()), &test_state());
    let listing = get(&app, "/v2/fs").await;
    assert_eq!(
        (listing["count"].clone(), listing["truncated"].clone()),
        (json!(5000), json!(true))
    );
    let names = entry_names(&listing);
    assert_eq!(
        (
            names.first().unwrap().as_str(),
            names.last().unwrap().as_str()
        ),
        ("f00000.fits", "f04999.fits")
    );
}

#[tokio::test]
async fn capabilities_lists_every_route_with_the_features_and_limits_of_this_build() {
    let state = test_state();
    let app = science_app(
        cfg_with("127.0.0.1:8080", &[("ASTROBURST_MAST_TOKEN", MAST_TOKEN)]),
        &state,
    );
    let caps = get(&app, "/v2/capabilities").await;

    let listed: Vec<String> = caps["routes"]
        .as_array()
        .expect("a route list")
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r["method"].as_str().unwrap(),
                r["path"].as_str().unwrap()
            )
        })
        .collect();
    let registered = registered_routes();
    assert_eq!(listed, registered);
    assert!(registered.contains(&"GET /v2/capabilities".to_owned()));
    assert_eq!(caps["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        caps["features"],
        json!({
            "mast": cfg!(feature = "mast"),
            "astrometry_net": cfg!(feature = "astrometry-net"),
            "vizier": cfg!(feature = "vizier"),
            "asdf_full": cfg!(feature = "asdf-full"),
        })
    );
    let config = ServerConfig::default();
    assert_eq!(
        caps["limits"],
        json!({
            "session_max": config.session_max,
            "session_ttl_seconds": config.session_ttl.as_secs(),
            "jobs_max": config.jobs_max,
            "cache_max_entries": config.cache_max_entries,
            "cache_max_bytes": config.cache_max_bytes,
        })
    );
    assert!(!caps.to_string().contains(MAST_TOKEN));
    let without = get(
        &science_app(cfg_with("127.0.0.1:8080", &[]), &state),
        "/v2/capabilities",
    )
    .await;
    if cfg!(feature = "mast") {
        assert_eq!(caps["mast_token"], json!({ "present": true }));
        assert_eq!(without["mast_token"], json!({ "present": false }));
    } else {
        assert!(caps.get("mast_token").is_none() && without.get("mast_token").is_none());
    }
}

#[tokio::test]
async fn a_synthetic_field_and_its_catalogue_equal_a_direct_generate_call_with_one_row_per_star() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let config = small_field();

    let accepted = post(
        &app,
        &format!("/v2/sessions/{sid}/synth"),
        json!({ "config": config, "name": "field", "save_ground_truth": true }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let job = job_of(&session, &body_json(accepted).await);
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Done);

    let (noisy, truth, stars) = pipeline::generate(&config).unwrap();
    let catalog = pipeline::catalog_csv(&stars, &config.noise);
    assert_eq!(stars.len(), config.field.n_stars);
    assert_eq!(catalog.lines().count(), stars.len() + 1);
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    assert_eq!(
        outcome["outcome"],
        json!({
            "result": { "width": 32, "height": 24, "star_count": stars.len(), "frames": 1, "catalog_csv": catalog, "manifest_csv": null },
            "warnings": [],
            "products": [{ "ref": "field", "present": true }, { "ref": "field_truth", "present": true }],
            "error": null,
        })
    );
    let published = session.cache.get("field").unwrap();
    assert_eq!(bits(published.arr()), bits(&noisy));
    assert_carries(
        published.header().unwrap(),
        &pipeline::synth_header(&config, 0, None),
        "field",
    );
    let truth_ref = session.cache.get("field_truth").unwrap();
    assert_eq!(bits(truth_ref.arr()), bits(&truth));
    assert_carries(
        truth_ref.header().unwrap(),
        &pipeline::ground_truth_header(&config),
        "field_truth",
    );
    let header = get(
        &app,
        &format!("/v2/sessions/{sid}/header?ref=field&keys=SYNSEED"),
    )
    .await;
    assert_eq!(header["cards"]["SYNSEED"]["value"], "4242");
}

#[tokio::test]
async fn a_synthetic_stack_publishes_one_ref_per_frame_equal_to_a_direct_generate_stack_call() {
    let state = test_state();
    let session = state.create_session().unwrap();
    let sid = session.id.clone();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let mut config = small_field();
    config.n_frames = 3;
    config.frame_variation = FrameVariation::default();

    let accepted = post(
        &app,
        &format!("/v2/sessions/{sid}/synth"),
        json!({ "config": config, "stack": true, "name": "run" }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let job = job_of(&session, &body_json(accepted).await);
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Done);

    let (frames, _, stars) = pipeline::generate_stack(&config).unwrap();
    let plan = pipeline::prepare_stack(&config).unwrap();
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    assert_eq!(
        outcome["outcome"]["result"],
        json!({
            "width": 32,
            "height": 24,
            "star_count": stars.len(),
            "frames": 3,
            "catalog_csv": pipeline::catalog_csv(&stars, &config.noise),
            "manifest_csv": plan.manifest_csv(),
        })
    );
    assert_eq!(
        outcome["outcome"]["products"],
        json!([
            { "ref": "run_0000", "present": true },
            { "ref": "run_0001", "present": true },
            { "ref": "run_0002", "present": true },
        ])
    );
    for (i, frame) in frames.iter().enumerate() {
        let name = format!("run_{i:04}");
        let published = session.cache.get(&name).unwrap();
        assert_eq!(bits(published.arr()), bits(frame), "{name}");
        assert_carries(published.header().unwrap(), &plan.header(i as u32), &name);
    }
    assert_ne!(bits(&frames[0]), bits(&frames[1]));
    assert!(!session.cache.contains("run_truth"));
}

#[tokio::test]
async fn synth_refuses_an_invalid_field_or_products_larger_than_the_session_cache_before_taking_a_job_slot(
) {
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &state);
    let uri = format!("/v2/sessions/{}/synth", session.id);

    let mut invalid = small_field();
    invalid.field.width = 0;
    let refused = post(&app, &uri, json!({ "config": invalid })).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let body = body_json(refused).await;
    assert_eq!(body["error"]["code"], "bad_request");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Field size 0x24"),
        "{body}"
    );

    let mut many = small_field();
    many.n_frames = 33;
    let refused = post(&app, &uri, json!({ "config": many, "stack": true })).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let body = body_json(refused).await;
    assert!(
        body["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("ASTROBURST_CACHE_MAX_ENTRIES"),
        "{body}"
    );
    assert!(session.jobs.is_empty());
    assert_eq!(
        state.job_semaphore.available_permits(),
        state.config.jobs_max
    );

    let frame_bytes = 32 * 24 * std::mem::size_of::<f32>();
    let small = AppState::new(Arc::new(ServerConfig {
        cache_max_bytes: 2 * frame_bytes,
        ..ServerConfig::default()
    }));
    let small_session = small.create_session().unwrap();
    let small_app = science_app(cfg_with("127.0.0.1:8080", &[]), &small);
    let small_uri = format!("/v2/sessions/{}/synth", small_session.id);
    let mut three = small_field();
    three.n_frames = 3;
    let refused = post(
        &small_app,
        &small_uri,
        json!({ "config": three, "stack": true }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let body = body_json(refused).await;
    assert!(
        body["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("ASTROBURST_CACHE_MAX_BYTES"),
        "{body}"
    );
    assert!(small_session.jobs.is_empty());

    let fits = post(
        &small_app,
        &small_uri,
        json!({ "config": small_field(), "save_ground_truth": true }),
    )
    .await;
    assert_eq!(fits.status(), StatusCode::ACCEPTED);
    wait_for_workers(&small).await;
    assert!(
        small_session.cache.contains("synth_0") && small_session.cache.contains("synth_0_truth")
    );
}

fn lightstack_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack")
}

fn d1_recipe() -> Value {
    let text = std::fs::read_to_string(lightstack_fixtures().join("d1/recipe_d1.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn cubeprep_cfg(bind: &str, out: &Path) -> ScienceConfig {
    let data = lightstack_fixtures().join("d1");
    cfg_with(
        bind,
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_SERVER_ROOT", data.to_str().unwrap()),
            ("ASTROBURST_OUTPUT_ROOT", out.to_str().unwrap()),
        ],
    )
}

fn cubeprep_uri(session: &Session) -> String {
    format!("/v2/sessions/{}/cubeprep/jobs", session.id)
}

async fn post_authorized(app: &Router, uri: &str, body: Value) -> Response {
    send(
        app,
        authorized(Method::POST, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

async fn accepted_cubeprep(app: &Router, session: &Session, recipe: Value) -> (Arc<Job>, PathBuf) {
    let response = post_authorized(app, &cubeprep_uri(session), recipe).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    assert_eq!(accepted["status"], "running");
    let output_dir = PathBuf::from(accepted["output_dir"].as_str().expect("an output folder"));
    (job_of(session, &accepted), output_dir)
}

#[tokio::test]
async fn a_cubeprep_job_runs_the_d1_recipe_from_the_data_root_and_lists_its_files_in_the_outcome() {
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("127.0.0.1:8080", out.path()), &state);

    let (job, output_dir) = accepted_cubeprep(&app, &session, d1_recipe()).await;
    assert_eq!(output_dir, out.path().join(format!("cubeprep-{}", job.id)));
    wait_for_workers(&state).await;
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    assert_eq!(job.current_status(), JobStatus::Done, "{outcome}");

    let recorded = &outcome["outcome"];
    let result = &recorded["result"];
    assert_eq!(result["provenance"]["schema_version"], 1);
    assert_eq!(result["provenance"]["integrity"], "sha256_before_after");
    assert_eq!(result["provenance"]["dependencies"].as_array().unwrap().len(), 12);
    assert!(result["files"].as_array().unwrap().iter().all(|file| file["sha256"].as_str().is_some_and(|hash| hash.len() == 64)));
    assert_eq!(result["output_dir"], json!(output_dir.to_str().unwrap()));
    let cube = PathBuf::from(result["cube"].as_str().unwrap());
    assert!(
        cube.starts_with(output_dir.join("08_match")) && cube.is_file(),
        "{result}"
    );
    let folders: Vec<&str> = result["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["directory"].as_str().unwrap())
        .collect();
    assert_eq!(
        folders,
        [
            "01_crop",
            "02_units",
            "03_align",
            "04_cube",
            "05_edit",
            "06_psfs",
            "07_kernels",
            "08_match"
        ]
    );
    let files: Vec<&str> = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(
        files.contains(&"recipe.json") && files.contains(&"07_kernels/kernel_f150w_to_f444w.fits"),
        "{files:?}"
    );
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(output_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(&written, result);
    assert!(!recorded["warnings"].as_array().unwrap().is_empty());
    assert_eq!(recorded["warnings"], result["warnings"]);
    assert_eq!(recorded["products"], json!([]));
    assert_eq!(recorded["error"], Value::Null);
    assert!(recorded.get("error_code").is_none() && recorded.get("error_hint").is_none());

    let saved = std::fs::read(output_dir.join("recipe.json")).unwrap();
    let response = send(
        &app,
        authorized(Method::POST, &cubeprep_uri(&session))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(saved.clone()))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    assert_eq!(accepted["warnings"], json!([]), "{accepted}");
    let repeated = job_of(&session, &accepted);
    wait_for_workers(&state).await;
    let again = get(&app, &outcome_uri(&session, &repeated.id)).await;
    assert_eq!(repeated.current_status(), JobStatus::Done, "{again}");
    let rerun = &again["outcome"]["result"];
    let saved_sha256: String = Sha256::digest(&saved)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(result["provenance"]["recipe_sha256"], json!(saved_sha256));
    assert_eq!(rerun["provenance"]["recipe_sha256"], json!(saved_sha256));
    assert_eq!(
        rerun["provenance"]["dependencies"],
        result["provenance"]["dependencies"]
    );
    let directories = |run: &Value| -> Vec<Value> {
        run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["directory"].clone())
            .collect()
    };
    assert_eq!(directories(rerun), directories(result));
    assert_eq!(
        rerun["files"].as_array().unwrap().len(),
        result["files"].as_array().unwrap().len()
    );
    let cube_sha256 = |run: &Value| -> Value {
        let cube = Path::new(run["cube"].as_str().unwrap());
        let folder = Path::new(run["output_dir"].as_str().unwrap());
        run["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| folder.join(file["path"].as_str().unwrap()) == cube)
            .expect("the cube is listed among the files")["sha256"]
            .clone()
    };
    assert_eq!(cube_sha256(rerun), cube_sha256(result));
}

#[tokio::test]
async fn cubeprep_refuses_a_bad_recipe_with_its_code_and_hint_before_taking_a_job_slot() {
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("127.0.0.1:8080", out.path()), &state);
    let changed = |change: &dyn Fn(&mut Value)| {
        let mut recipe = d1_recipe();
        change(&mut recipe);
        recipe
    };
    let cases = [
        (
            changed(&|r| r["steps"][1] = json!({ "step": "stretch" })),
            StatusCode::BAD_REQUEST,
            "unknown_step",
            "known steps are units, crop, align, cube, edit, psfs, kernels, match, export",
        ),
        (
            changed(&|r| r["steps"][2]["methd"] = json!("bilinear")),
            StatusCode::BAD_REQUEST,
            "unknown_parameter",
            "accepted keys: area_rule, border_crop, method, reference",
        ),
        (
            changed(&|r| r["recipe_version"] = json!(2)),
            StatusCode::BAD_REQUEST,
            "recipe_version_unsupported",
            "this AstroBurst reads recipe_version 1",
        ),
        (
            changed(&|r| r["inputs"][1]["filter"] = json!("f814w")),
            StatusCode::BAD_REQUEST,
            "filter_duplicate",
            "each input holds one filter; a cube takes one plane per filter",
        ),
        (
            changed(&|r| r["inputs"][0]["path"] = json!("missing.fits")),
            StatusCode::NOT_FOUND,
            "file_not_found",
            "recipe paths are relative to the data folder unless absolute",
        ),
    ];
    for (recipe, status, code, hint) in cases {
        let response = post(&app, &cubeprep_uri(&session), recipe).await;
        assert_eq!(response.status(), status, "{code}");
        let body = body_json(response).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["error"]["code"], code, "{body}");
        assert_eq!(body["error"]["hint"], hint, "{body}");
    }
    let invalid = post(
        &app,
        &cubeprep_uri(&session),
        changed(&|r| r["steps"][0]["size_arcsec"] = json!(0)),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(invalid).await["error"],
        json!({ "code": "recipe_invalid", "message": "steps[0] (crop): size_arcsec must be positive" })
    );
    assert!(session.jobs.is_empty());
    assert_eq!(
        state.job_semaphore.available_permits(),
        state.config.jobs_max
    );
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_cubeprep_recipe_that_cuts_before_the_match_is_accepted_with_the_validation_advisory() {
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("127.0.0.1:8080", out.path()), &state);
    let mut recipe = d1_recipe();
    recipe["steps"].as_array_mut().unwrap().insert(
        5,
        json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 }),
    );
    let advisory = json!(
        "09_match: 06_edit cuts the cube before the match; pixels within half a kernel width of the cut edges are matched against zeros; place the cut after match to use the cut-away data"
    );

    let response = post_authorized(&app, &cubeprep_uri(&session), recipe).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    assert_eq!(accepted["warnings"], json!([advisory]), "{accepted}");
    let job = job_of(&session, &accepted);
    wait_for_workers(&state).await;
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    assert_eq!(job.current_status(), JobStatus::Done, "{outcome}");
    assert!(
        outcome["outcome"]["warnings"]
            .as_array()
            .unwrap()
            .contains(&advisory),
        "{outcome}"
    );
}

#[tokio::test]
async fn a_cubeprep_job_without_a_kernel_for_a_plane_fails_with_kernel_missing_and_keeps_its_earlier_steps(
) {
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("127.0.0.1:8080", out.path()), &state);
    let mut recipe = d1_recipe();
    recipe["steps"][5]["files"]
        .as_array_mut()
        .unwrap()
        .remove(1);

    let (job, output_dir) = accepted_cubeprep(&app, &session, recipe).await;
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Error);
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    let recorded = &outcome["outcome"];
    assert_eq!(recorded["result"], Value::Null);
    assert_eq!(recorded["error_code"], "kernel_missing", "{recorded}");
    let message = recorded["error"].as_str().unwrap();
    assert!(
        message.starts_with("08_match: ") && message.contains("F150W"),
        "{message}"
    );
    assert!(
        recorded["error_hint"]
            .as_str()
            .unwrap()
            .contains("allow_unmatched"),
        "{recorded}"
    );
    assert!(output_dir.join("07_kernels").is_dir());
    assert!(!output_dir.join("result.json").exists());
}

#[tokio::test]
async fn a_failed_cubeprep_job_keeps_the_advisories_of_its_202_in_the_failure_marker() {
    let out = tempfile::tempdir().unwrap();
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("127.0.0.1:8080", out.path()), &state);
    let mut recipe = d1_recipe();
    recipe["steps"][5]["files"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    recipe["steps"].as_array_mut().unwrap().insert(
        5,
        json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 }),
    );

    let response = post_authorized(&app, &cubeprep_uri(&session), recipe).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    let advisories = accepted["warnings"].as_array().unwrap().clone();
    assert_eq!(advisories.len(), 1, "{accepted}");
    let job = job_of(&session, &accepted);
    let output_dir = PathBuf::from(accepted["output_dir"].as_str().unwrap());
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Error);
    let marker: Value =
        serde_json::from_str(&std::fs::read_to_string(output_dir.join("failure.json")).unwrap())
            .unwrap();
    assert_eq!(
        (marker["code"].as_str(), marker["stage"].as_str()),
        (Some("kernel_missing"), Some("09_match")),
        "{marker}"
    );
    let warnings = marker["warnings"].as_array().unwrap();
    assert_eq!(warnings.first(), advisories.first(), "{marker}");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().unwrap().starts_with("08_kernels: ")),
        "{marker}"
    );
}

#[tokio::test]
async fn a_guarded_cubeprep_refuses_a_path_outside_the_roots_and_resolves_relative_ones_under_the_root(
) {
    let out = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let foreign = fixture(outside.path(), "outside.fits");
    let state = test_state();
    let session = state.create_session().unwrap();
    let app = science_app(cubeprep_cfg("0.0.0.0:8080", out.path()), &state);

    for path in [
        json!(foreign),
        json!("../d1_stars/d1_stars_JWST_F444W.fits"),
    ] {
        let mut recipe = d1_recipe();
        recipe["inputs"][0]["path"] = path;
        let response = post_authorized(&app, &cubeprep_uri(&session), recipe).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            body_json(response).await["error"]["code"],
            "path_outside_root"
        );
    }
    assert!(session.jobs.is_empty());

    let (job, _) = accepted_cubeprep(&app, &session, d1_recipe()).await;
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Done);
}

#[tokio::test]
async fn a_guarded_cubeprep_refuses_an_empirical_psf_image_that_links_outside_the_roots() {
    let root = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let stars = lightstack_fixtures().join("d1_stars");
    let image = stars.join("d1_stars_JWST_F444W.fits");
    let data_dir = root.path().join("stars");
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::copy(&image, root.path().join("input.fits")).unwrap();
    std::fs::copy(&image, outside.path().join("elsewhere.fits")).unwrap();
    std::fs::copy(
        stars.join("stars_f444w.reg"),
        data_dir.join("stars_f444w.reg"),
    )
    .unwrap();
    let linked = data_dir.join("x_JWST_F444W.fits");
    link_file(&outside.path().join("elsewhere.fits"), &linked);
    let state = test_state();
    let session = state.create_session().unwrap();
    let cfg = cfg_with(
        "0.0.0.0:8080",
        &[
            ("ASTROBURST_TOKEN", TOKEN),
            ("ASTROBURST_SERVER_ROOT", root.path().to_str().unwrap()),
            ("ASTROBURST_OUTPUT_ROOT", out.path().to_str().unwrap()),
        ],
    );
    let app = science_app(cfg, &state);
    let text = std::fs::read_to_string(stars.join("recipe_d1_stars.json")).unwrap();
    let mut recipe: Value = serde_json::from_str(&text).unwrap();
    recipe["inputs"][0]["path"] = json!("input.fits");
    recipe["steps"][0]["data_dir"] = json!("stars");

    let response = post_authorized(&app, &cubeprep_uri(&session), recipe.clone()).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["error"]["code"],
        "path_outside_root"
    );
    assert!(session.jobs.is_empty());

    std::fs::remove_file(&linked).unwrap();
    std::fs::copy(&image, &linked).unwrap();
    let (job, output_dir) = accepted_cubeprep(&app, &session, recipe).await;
    wait_for_workers(&state).await;
    assert_eq!(job.current_status(), JobStatus::Done);
    assert!(output_dir.join("01_psfs").is_dir());
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[tokio::test]
async fn a_cubeprep_job_writes_under_the_default_output_root_of_the_server_root() {
    let root = tempfile::tempdir().unwrap();
    copy_tree(&lightstack_fixtures().join("d1"), root.path());
    let state = test_state();
    let session = state.create_session().unwrap();
    let cfg = cfg_with(
        "127.0.0.1:8080",
        &[("ASTROBURST_SERVER_ROOT", root.path().to_str().unwrap())],
    );
    let app = science_app(cfg, &state);
    let (job, output_dir) = accepted_cubeprep(&app, &session, d1_recipe()).await;
    wait_for_workers(&state).await;
    let outcome = get(&app, &outcome_uri(&session, &job.id)).await;
    assert_eq!(
        job.current_status(),
        JobStatus::Done,
        "{}",
        outcome["outcome"]["error"]
    );
    let out = std::fs::canonicalize(root.path().join("out")).unwrap();
    assert_eq!(
        std::fs::canonicalize(&output_dir).unwrap().parent(),
        Some(out.as_path())
    );
    assert!(output_dir.join("08_match").is_dir());
}

#[tokio::test]
async fn a_cubeprep_job_reads_its_inputs_under_a_server_root_whose_name_ends_in_a_period() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(parent.path()).unwrap();
    let data = canonical.join("d1.");
    copy_tree(&lightstack_fixtures().join("d1"), &data);
    copy_tree(&lightstack_fixtures().join("d1"), &canonical.join("d1"));

    for bind in ["0.0.0.0:8080", "127.0.0.1:8080"] {
        let out = tempfile::tempdir().unwrap();
        let state = test_state();
        let session = state.create_session().unwrap();
        let cfg = cfg_with(
            bind,
            &[
                ("ASTROBURST_TOKEN", TOKEN),
                ("ASTROBURST_SERVER_ROOT", data.to_str().unwrap()),
                ("ASTROBURST_OUTPUT_ROOT", out.path().to_str().unwrap()),
            ],
        );
        let app = science_app(cfg, &state);
        let (job, _) = accepted_cubeprep(&app, &session, d1_recipe()).await;
        wait_for_workers(&state).await;
        let response = send(
            &app,
            authorized(Method::GET, &outcome_uri(&session, &job.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{bind}");
        let outcome = body_json(response).await;
        assert_eq!(job.current_status(), JobStatus::Done, "{bind}: {outcome}");
        let dependencies = outcome["outcome"]["result"]["provenance"]["dependencies"]
            .as_array()
            .unwrap();
        assert_eq!(dependencies.len(), 12, "{bind}");
        assert!(
            dependencies
                .iter()
                .all(|dependency| Path::new(dependency["path"].as_str().unwrap()).starts_with(&data)),
            "{bind}: {dependencies:?}"
        );
    }
}

#[tokio::test]
async fn cubeprep_filters_lists_the_datasets_and_filters_a_recipe_can_name() {
    let app = science_app(cfg_with("127.0.0.1:8080", &[]), &test_state());
    let listed = get(&app, "/v2/cubeprep/filters").await;
    let registry = filter_registry();
    assert_eq!(
        listed,
        json!({ "datasets": registry.datasets(), "filters": registry.entries() })
    );
    for dataset in ["HST", "JWST"] {
        assert!(listed["datasets"]
            .as_array()
            .unwrap()
            .contains(&json!(dataset)));
    }
    let f444w = listed["filters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["dataset"] == "JWST" && entry["filter"] == "F444W")
        .expect("JWST F444W is listed");
    assert!(
        f444w["pivot_angstrom"].as_f64().unwrap() > 40_000.0,
        "{f444w}"
    );
}
