// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use crate::config::ServerConfig;
use crate::job::{new_job, spawn_waiting_worker};
use crate::router::build_router;
use crate::session::Session;
use crate::state::AppState;

fn cfg() -> Arc<ServerConfig> {
    Arc::new(ServerConfig::default())
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_returns_ok() {
    let app = build_router(AppState::new(cfg()));
    let resp = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
    assert!(json["sessions_active"].is_number());
    assert!(json["sessions_total"].is_number());
}

#[tokio::test]
async fn request_id_echoed() {
    let app = build_router(AppState::new(cfg()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("x-request-id", "echo-me-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("echo-me-123")
    );
}

#[tokio::test]
async fn unknown_session_returns_404() {
    let app = build_router(AppState::new(cfg()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/sessions/no-such-sid/jobs/any-jid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fits_header_unknown_slot_returns_404() {
    let state = AppState::new(cfg());
    state
        .sessions
        .insert("sid-h".into(), Session::new("sid-h".into(), &ServerConfig::default()));

    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/sessions/sid-h/fits/header")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"slot":"ghost"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_job_returns_running_status() {
    let state = AppState::new(cfg());
    let session = Session::new("sid-g".into(), &ServerConfig::default());
    let job = new_job("test-action");
    let jid = job.id.clone();
    session.jobs.insert(jid.clone(), Arc::clone(&job));
    state.sessions.insert("sid-g".into(), session);

    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/sid-g/jobs/{jid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "running");
    assert_eq!(json["action"], "test-action");
}

async fn wait_for_free_permit(state: &AppState) {
    let permit = tokio::time::timeout(std::time::Duration::from_secs(5), state.job_semaphore.acquire())
        .await
        .expect("the worker must give its queue slot back when it exits")
        .expect("semaphore open");
    drop(permit);
}

#[tokio::test]
async fn cancel_job_sets_cancelled() {
    let config = ServerConfig { jobs_max: 1, ..ServerConfig::default() };
    let state = AppState::new(Arc::new(config.clone()));
    let session = Session::new("sid-c".into(), &config);
    let job = new_job("to-cancel");
    let mut events = job.rx.lock().unwrap().take().unwrap();
    let jid = job.id.clone();
    session.jobs.insert(jid.clone(), Arc::clone(&job));
    state.sessions.insert("sid-c".into(), Arc::clone(&session));
    let release = spawn_waiting_worker(&job, Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap()).await;
    assert_eq!(state.job_semaphore.available_permits(), 0);

    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/sessions/sid-c/jobs/{jid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "cancelled");
    assert!(job.cancel.is_cancelled());
    assert!(matches!(events.try_recv(), Ok(crate::job::SseEvent::Cancelled)));
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    ));

    assert_eq!(state.job_semaphore.available_permits(), 0, "the cancelled worker still runs and keeps its queue slot");
    assert!(session.has_active_jobs(), "the TTL cleaner must not evict a session whose worker still runs");
    let resp = post_json(
        build_router(state.clone()),
        "/sessions/sid-c/stacking/stack",
        r#"{"paths":["/nonexistent.fits"],"align":false}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS, "a retry must wait for the cancelled worker");

    drop(release);
    wait_for_free_permit(&state).await;
    assert!(!session.has_active_jobs());
    assert_eq!(job.current_status(), crate::job::JobStatus::Cancelled);
}

#[tokio::test]
async fn deleting_a_v2_session_cancels_its_running_jobs() {
    let state = AppState::new(cfg());
    let session = Session::new("sid-del-jobs".into(), &ServerConfig::default());
    let job = new_job("stack");
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    state.sessions.insert("sid-del-jobs".into(), session);

    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/v2/sessions/sid-del-jobs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(job.current_status(), crate::job::JobStatus::Cancelled);
    assert!(job.cancel.is_cancelled());
}

#[tokio::test]
async fn sse_stream_ends_with_a_cancelled_event() {
    let state = AppState::new(cfg());
    let session = Session::new("sid-sse-c".into(), &ServerConfig::default());
    let job = new_job("stack");
    let jid = job.id.clone();
    session.jobs.insert(jid.clone(), Arc::clone(&job));
    state.sessions.insert("sid-sse-c".into(), session);

    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(format!("/sessions/sid-sse-c/jobs/{jid}/stream")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    job.progress(30, "loading");
    job.set_cancelled();

    let bytes = tokio::time::timeout(std::time::Duration::from_secs(5), axum::body::to_bytes(resp.into_body(), usize::MAX))
        .await
        .expect("the stream must close after the terminal event")
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("event: progress"), "{text}");
    assert!(text.trim_end().ends_with(r#"data: {"type":"cancelled"}"#), "{text}");
}

#[tokio::test]
async fn sse_second_subscriber_returns_409() {
    let state = AppState::new(cfg());
    let session = Session::new("sid-s".into(), &ServerConfig::default());
    let job = new_job("sse-test");
    let _ = job.rx.lock().unwrap().take();
    let jid = job.id.clone();
    session.jobs.insert(jid.clone(), Arc::clone(&job));
    state.sessions.insert("sid-s".into(), session);

    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/sid-s/jobs/{jid}/stream"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn semaphore_full_returns_429() {
    let state = AppState::new(cfg());
    state
        .sessions
        .insert("sid-t".into(), Session::new("sid-t".into(), &ServerConfig::default()));

    let _p1 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();
    let _p2 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();
    let _p3 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();
    let _p4 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/sessions/sid-t/stacking/stack")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"paths":["/nonexistent.fits"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn ttl_skips_session_with_running_job() {
    let c = ServerConfig::default();
    let session = Session::new("sid-ttl".into(), &c);
    assert!(!session.has_active_jobs(), "fresh session: no active jobs");

    let job = new_job("bg-work");
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    assert!(session.has_active_jobs(), "running job: session is active");

    assert!(job.begin_commit());
    assert!(session.has_active_jobs(), "job storing its result: session is still active");

    job.set_done();
    assert!(!session.has_active_jobs(), "done job: session no longer active");
}

#[tokio::test]
async fn ttl_keeps_a_session_while_its_cancelled_worker_still_runs() {
    let state = AppState::new(Arc::new(ServerConfig { jobs_max: 1, ..ServerConfig::default() }));
    let session = Session::new("sid-ttl-c".into(), &ServerConfig::default());
    let job = new_job("stack");
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    let release = spawn_waiting_worker(&job, Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap()).await;

    assert!(job.set_cancelled());
    assert!(!job.is_running());
    assert!(session.has_active_jobs(), "cancelled but the worker is still combining frames");

    drop(release);
    wait_for_free_permit(&state).await;
    assert!(!session.has_active_jobs(), "the worker has exited");
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn from_env_with(vars: &[(&str, &str)]) -> ServerConfig {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for (k, v) in vars {
        std::env::set_var(k, v);
    }
    let parsed = ServerConfig::from_env();
    for (k, _) in vars {
        std::env::remove_var(k);
    }
    parsed
}

#[test]
fn config_env_var_parsed_and_invalid_falls_back() {
    let d = ServerConfig::default();
    assert_eq!(d.session_max, 8);
    assert_eq!(d.jobs_max, 4);
    assert_eq!(d.cache_max_entries, 32);
    assert_eq!(d.cache_max_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(d.session_ttl.as_secs(), 900);
    assert_eq!(d.cleanup_interval.as_secs(), 60);
    assert_eq!(d.log_level, "info");
    assert_eq!(d.bind.to_string(), "127.0.0.1:8080");

    let parsed = from_env_with(&[
        ("ASTROBURST_SESSION_MAX", "3"),
        ("ASTROBURST_SESSION_TTL", "120"),
        ("ASTROBURST_JOBS_MAX", "not-a-number"),
        ("ASTROBURST_BIND", "127.0.0.1:9"),
    ]);
    assert_eq!(parsed.session_max, 3);
    assert_eq!(parsed.session_ttl.as_secs(), 120);
    assert_eq!(parsed.jobs_max, d.jobs_max);
    assert_eq!(parsed.bind.to_string(), "127.0.0.1:9");
    assert_eq!(parsed.cache_max_entries, d.cache_max_entries);
}

#[test]
fn config_zero_cleanup_interval_is_clamped_so_the_ttl_cleaner_cannot_panic() {
    let parsed = from_env_with(&[("ASTROBURST_CLEANUP_INTERVAL", "0")]);
    assert_eq!(parsed.cleanup_interval.as_secs(), 1);
    let parsed = from_env_with(&[("ASTROBURST_CLEANUP_INTERVAL", "5")]);
    assert_eq!(parsed.cleanup_interval.as_secs(), 5);
}

pub(crate) mod v2_fixtures {

    use std::io::Write;

    const BLOCK: usize = 2880;

    fn card(key: &str, value: &str) -> Vec<u8> {
        let text = format!("{key:<8}= {value}");
        let mut bytes = text.into_bytes();
        bytes.resize(80, b' ');
        bytes
    }

    fn header_block(cards: &[(&str, String)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in cards {
            out.extend_from_slice(&card(k, v));
        }
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        out.extend_from_slice(&end);
        while out.len() % BLOCK != 0 {
            out.push(b' ');
        }
        out
    }

    fn data_block(pixels: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in pixels {
            out.extend_from_slice(&p.to_be_bytes());
        }
        while out.len() % BLOCK != 0 {
            out.push(0);
        }
        out
    }

    fn ramp(w: usize, h: usize) -> Vec<f32> {
        (0..w * h).map(|i| i as f32).collect()
    }

    fn wcs_cards() -> Vec<(&'static str, String)> {
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "4.0".into()),
            ("CRPIX2", "4.0".into()),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", "-1.0E-4".into()),
            ("CD1_2", "0.0".into()),
            ("CD2_1", "0.0".into()),
            ("CD2_2", "1.0E-4".into()),
        ]
    }

    pub fn write_wcs_fits(path: &std::path::Path, w: usize, h: usize) {
        let mut cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
        ];
        cards.extend(wcs_cards());

        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(&ramp(w, h)));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_rotated_wcs_fits(path: &std::path::Path, w: usize, h: usize) {
        let cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "4.0".into()),
            ("CRPIX2", "4.0".into()),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", "-1.0E-4".into()),
            ("CD1_2", "2.0E-5".into()),
            ("CD2_1", "-3.0E-5".into()),
            ("CD2_2", "1.0E-4".into()),
        ];
        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(&ramp(w, h)));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_no_wcs_fits(path: &std::path::Path, w: usize, h: usize) {
        let cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
        ];
        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(&ramp(w, h)));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_bunit_fits(path: &std::path::Path, w: usize, h: usize, bunit: &str) {
        let mut cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("BUNIT", format!("'{bunit}'")),
        ];
        cards.extend(wcs_cards());
        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(&ramp(w, h)));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_exposure_fits(path: &std::path::Path, w: usize, h: usize, exptime: f64) {
        let cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("EXPTIME", format!("{exptime:.1}")),
        ];
        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(&vec![100.0; w * h]));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_pixels_fits(path: &std::path::Path, w: usize, h: usize, pixels: &[f32]) {
        assert_eq!(pixels.len(), w * h, "pixel count must equal w*h");
        let cards: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
        ];
        let mut buf = header_block(&cards);
        buf.extend_from_slice(&data_block(pixels));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn write_mef_fits(
        path: &std::path::Path,
        (w0, h0): (usize, usize),
        (w1, h1): (usize, usize),
    ) {
        let mut primary: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w0.to_string()),
            ("NAXIS2", h0.to_string()),
        ];
        primary.extend(wcs_cards());

        let ext: Vec<(&str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w1.to_string()),
            ("NAXIS2", h1.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'WEIGHT  '".into()),
        ];

        let mut buf = header_block(&primary);
        buf.extend_from_slice(&data_block(&ramp(w0, h0)));
        buf.extend_from_slice(&header_block(&ext));
        buf.extend_from_slice(&data_block(&ramp(w1, h1)));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    fn data_block_i32(pixels: &[i32]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in pixels {
            out.extend_from_slice(&p.to_be_bytes());
        }
        while out.len() % BLOCK != 0 {
            out.push(0);
        }
        out
    }

    fn image_ext(name: &str, bitpix: &str, w: usize, h: usize, extra: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", bitpix.into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", format!("'{name:<8}'")),
            ("EXTVER", "1".into()),
        ];
        cards.extend(extra.iter().cloned());
        cards
    }

    pub const DQ_FLAGGED_PIXEL: (usize, usize) = (1, 0);
    pub const DQ_HIGH_PIXEL: (usize, usize) = (2, 1);

    pub fn write_mef_with_dq(path: &std::path::Path, w: usize, h: usize) {
        let mut primary: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
        ];
        primary.extend(wcs_cards());

        let sci = image_ext("SCI", "-32", w, h, &[("BUNIT", "'MJy/sr'".into())]);
        let err = image_ext("ERR", "-32", w, h, &[("BUNIT", "'MJy/sr'".into())]);
        let dq = image_ext("DQ", "32", w, h, &[("BZERO", "2147483648".into()), ("BSCALE", "1".into())]);

        let mut dq_pixels = vec![-2147483648i32; w * h];
        dq_pixels[DQ_FLAGGED_PIXEL.1 * w + DQ_FLAGGED_PIXEL.0] = 3 - 2147483647 - 1;
        dq_pixels[DQ_HIGH_PIXEL.1 * w + DQ_HIGH_PIXEL.0] = 1;

        let mut buf = header_block(&primary);
        buf.extend_from_slice(&header_block(&sci));
        buf.extend_from_slice(&data_block(&ramp(w, h)));
        buf.extend_from_slice(&header_block(&err));
        buf.extend_from_slice(&data_block(&ramp(w, h).iter().map(|v| v * 0.5).collect::<Vec<f32>>()));
        buf.extend_from_slice(&header_block(&dq));
        buf.extend_from_slice(&data_block_i32(&dq_pixels));
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }
}

async fn post_json(
    app: axum::Router,
    uri: &str,
    body: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap(),
    )
    .await
    .unwrap()
}

fn seed_session(state: &AppState, id: &str) {
    state
        .sessions
        .insert(id.into(), Session::new(id.into(), &ServerConfig::default()));
}

#[tokio::test]
async fn v2_create_session_returns_id() {
    let app = build_router(AppState::new(cfg()));
    let resp = post_json(app, "/v2/sessions", "").await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert!(json["session_id"].is_string());
}

#[tokio::test]
async fn v2_open_returns_dims_stats_wcs_and_lists_ref() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("wcs.fits");
    v2_fixtures::write_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-open");

    let body = format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap());
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-open/open", &body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ref"], "img_0");
    assert_eq!(json["active_ref"], "img_0");
    assert_eq!(json["dims"], serde_json::json!([8, 8]));
    assert_eq!(json["wcs_present"], true);
    assert!(json["stats"]["median"].is_number());
    assert!(json["header"]["CTYPE1"].is_string());

    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-open/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["active_ref"], "img_0");
    assert_eq!(json["count"], 1);
    assert_eq!(json["images"][0]["image_ref"], "img_0");
    assert_eq!(json["images"][0]["wcs_present"], true);
}

#[tokio::test]
async fn v2_second_open_adds_ref_without_evicting_first() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.fits");
    let b = dir.path().join("b.fits");
    v2_fixtures::write_wcs_fits(&a, 8, 8);
    v2_fixtures::write_wcs_fits(&b, 6, 6);

    let state = AppState::new(cfg());
    seed_session(&state, "s-two");

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-two/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(a.to_str().unwrap()).unwrap()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-two/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(b.to_str().unwrap()).unwrap()),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["ref"], "img_1");
    assert_eq!(json["active_ref"], "img_1");
    assert_eq!(json["dims"], serde_json::json!([6, 6]));

    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-two/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    assert_eq!(json["active_ref"], "img_1");
    assert_eq!(json["images"][0]["width"], 8);
    assert_eq!(json["images"][1]["width"], 6);

    let session = state.sessions.get("s-two").unwrap().clone();
    assert_eq!(session.cache.get("img_0").unwrap().arr().dim(), (8, 8));
    assert_eq!(session.cache.get("img_1").unwrap().arr().dim(), (6, 6));
}

#[tokio::test]
async fn v2_hdu_switch_creates_new_ref_and_keeps_original() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("mef.fits");
    v2_fixtures::write_mef_fits(&fits, (8, 8), (16, 4));

    let state = AppState::new(cfg());
    seed_session(&state, "s-hdu");

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-hdu/open",
        &format!(r#"{{"path":{},"hdu":0}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["ref"], "img_0");
    assert_eq!(json["dims"], serde_json::json!([8, 8]));

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-hdu/hdu", r#"{"hdu":1}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ref"], "img_1");
    assert_eq!(json["active_ref"], "img_1");
    assert_eq!(json["dims"], serde_json::json!([16, 4]));

    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-hdu/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    let img0 = json["images"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["image_ref"] == "img_0")
        .unwrap();
    assert_eq!(img0["width"], 8);
    assert_eq!(img0["height"], 8);
}

async fn open_dq_mef(id: &str, hdu: usize) -> (AppState, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("dq_mef.fits");
    v2_fixtures::write_mef_with_dq(&fits, 4, 4);
    let path = fits.to_str().unwrap().to_string();

    let state = AppState::new(cfg());
    seed_session(&state, id);
    let resp = post_json(
        build_router(state.clone()),
        &format!("/v2/sessions/{id}/open"),
        &format!(r#"{{"path":{},"hdu":{hdu}}}"#, serde_json::to_string(&path).unwrap()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    (state, dir, path)
}

#[tokio::test]
async fn v2_open_reports_plane_ref_and_is_dq() {
    let (state, _dir, path) = open_dq_mef("s-plane", 1).await;
    let resp = get_uri(build_router(state.clone()), "/v2/sessions/s-plane/images").await;
    let json = body_json(resp).await;
    let img = &json["images"][0];
    assert_eq!(img["plane_ref"], format!("{path}#hdu=1"));
    assert_eq!(img["is_dq"], false);
    assert!(img["array"].is_null());
    assert_eq!(img["hdu"], 1);

    let resp = post_json(build_router(state), "/v2/sessions/s-plane/hdu", r#"{"hdu":3}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["is_dq"], true);
    assert_eq!(json["plane_ref"], format!("{path}#hdu=3"));
    assert_eq!(json["extname"], "DQ");
}

#[tokio::test]
async fn v2_hdu_switch_with_array_on_fits_is_bad_request() {
    let (state, _dir, _path) = open_dq_mef("s-arr", 1).await;
    let resp = post_json(build_router(state), "/v2/sessions/s-arr/hdu", r#"{"array":"dq"}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
    assert!(json["error"]["message"].as_str().unwrap().contains("no ASDF arrays"));
}

#[tokio::test]
async fn v2_open_with_both_hdu_and_array_is_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("both.fits");
    v2_fixtures::write_mef_with_dq(&fits, 4, 4);
    let state = AppState::new(cfg());
    seed_session(&state, "s-both");
    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-both/open",
        &format!(r#"{{"path":{},"hdu":1,"array":"dq"}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["message"], "provide exactly one of hdu or array");

    let resp = post_json(build_router(state), "/v2/sessions/s-both/hdu", r#"{}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v2_pixel_reports_dq_and_err_companions() {
    let (state, _dir, _path) = open_dq_mef("s-dqpx", 1).await;
    let (x, y) = v2_fixtures::DQ_FLAGGED_PIXEL;
    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-dqpx/pixel",
        &format!(r#"{{"x":{x},"y":{y},"box":1}}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dq"]["text"], "3: DO_NOT_USE | SATURATED");
    assert_eq!(json["dq"]["bits"], 3);
    assert_eq!(json["dq"]["table"], "unknown");
    assert_eq!(json["dq"]["names"], serde_json::json!(["DO_NOT_USE", "SATURATED"]));
    assert!((json["err"]["value"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert_eq!(json["err"]["unit"], "MJy/sr");
    assert_eq!(json["unit"], "MJy/sr");

    let resp = post_json(build_router(state), "/v2/sessions/s-dqpx/pixel", r#"{"x":0,"y":0,"box":1}"#).await;
    let json = body_json(resp).await;
    assert_eq!(json["dq"]["text"], "0: GOOD");
    assert_eq!(json["err"]["value"], 0.0);
}

#[tokio::test]
async fn v2_pixel_on_dq_ref_reports_high_bits() {
    let (state, _dir, _path) = open_dq_mef("s-dqself", 3).await;
    let (x, y) = v2_fixtures::DQ_HIGH_PIXEL;
    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-dqself/pixel",
        &format!(r#"{{"x":{x},"y":{y},"box":1}}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dq"]["bits"], 2147483649u64);
    assert_eq!(json["dq"]["value"], 2147483649u64);
    assert_eq!(json["dq"]["text"], "2147483649: DO_NOT_USE | REFERENCE_PIXEL");
    assert!((json["value"].as_f64().unwrap() - 2147483649.0).abs() < 300.0);
}

#[tokio::test]
async fn v2_delete_session_then_404() {
    let state = AppState::new(cfg());
    seed_session(&state, "s-del");

    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/v2/sessions/s-del")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-del")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn v2_keepalive_ok_and_status_reports_active_ref() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("k.fits");
    v2_fixtures::write_wcs_fits(&fits, 4, 4);

    let state = AppState::new(cfg());
    seed_session(&state, "s-ka");

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-ka/keepalive", "").await;
    assert_eq!(resp.status(), StatusCode::OK);

    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-ka/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-ka")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["active_ref"], "img_0");
    assert_eq!(json["image_count"], 1);
}

#[tokio::test]
async fn v2_list_and_status_drop_refs_evicted_by_cache() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..3)
        .map(|i| {
            let p = dir.path().join(format!("e{i}.fits"));
            v2_fixtures::write_wcs_fits(&p, 4, 4);
            p
        })
        .collect();

    let cfg = ServerConfig {
        cache_max_entries: 2,
        ..ServerConfig::default()
    };
    let state = AppState::new(Arc::new(cfg.clone()));
    state
        .sessions
        .insert("s-evict".into(), Session::new("s-evict".into(), &cfg));

    for p in &paths {
        let resp = post_json(
            build_router(state.clone()),
            "/v2/sessions/s-evict/open",
            &format!(r#"{{"path":{}}}"#, serde_json::to_string(p.to_str().unwrap()).unwrap()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-evict/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    assert_eq!(json["active_ref"], "img_2");
    let refs: Vec<&str> = json["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["image_ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs, vec!["img_1", "img_2"]);

    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v2/sessions/s-evict")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["image_count"], 2);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-evict/stats",
        r#"{"ref":"img_0"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

async fn seed_wcs_session(id: &str) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("wcs.fits");
    v2_fixtures::write_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, id);
    post_json(
        build_router(state.clone()),
        &format!("/v2/sessions/{id}/open"),
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;
    (state, dir)
}

#[tokio::test]
async fn v2_pix2sky_converts_batch_and_reports_on_image() {
    let (state, _dir) = seed_wcs_session("s-p2s").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-p2s/wcs/pix2sky",
        r#"{"points":[[3,3],[100,100]]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);

    let r0 = &json["results"][0];
    assert!((r0["ra"].as_f64().unwrap() - 150.0).abs() < 1e-6);
    assert!((r0["dec"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    assert_eq!(r0["on_image"], true);

    let r1 = &json["results"][1];
    assert_eq!(r1["on_image"], false);
    assert_eq!(json["frame"], "icrs");
}

#[tokio::test]
async fn v2_pix2sky_galactic_frame_converts_from_icrs() {
    use astroburst_lib::core::astrometry::frames::icrs_to_galactic;

    let (state, _dir) = seed_wcs_session("s-p2s-gal").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-p2s-gal/wcs/pix2sky",
        r#"{"points":[[3,3],[5,1]]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let icrs = body_json(resp).await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-p2s-gal/wcs/pix2sky",
        r#"{"points":[[3,3],[5,1]],"frame":"galactic"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let gal = body_json(resp).await;
    assert_eq!(gal["frame"], "galactic");
    assert_eq!(gal["count"], 2);
    for i in 0..2 {
        let (l, b) = icrs_to_galactic(
            icrs["results"][i]["ra"].as_f64().unwrap(),
            icrs["results"][i]["dec"].as_f64().unwrap(),
        );
        assert!((gal["results"][i]["ra"].as_f64().unwrap() - l).abs() < 1e-9, "point {i}");
        assert!((gal["results"][i]["dec"].as_f64().unwrap() - b).abs() < 1e-9, "point {i}");
        assert_eq!(gal["results"][i]["on_image"], true);
    }
    assert!((gal["results"][0]["ra"].as_f64().unwrap() - icrs["results"][0]["ra"].as_f64().unwrap()).abs() > 1.0);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-p2s-gal/wcs/pix2sky",
        r#"{"points":[[3,3]],"frame":"supergalactic"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn v2_sky2pix_round_trips_within_tolerance() {
    let (state, _dir) = seed_wcs_session("s-s2p").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-s2p/wcs/sky2pix",
        r#"{"points":[[150.0,2.0]]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let r0 = &json["results"][0];
    assert!((r0["x"].as_f64().unwrap() - 3.0).abs() < 1e-6);
    assert!((r0["y"].as_f64().unwrap() - 3.0).abs() < 1e-6);
    assert_eq!(r0["on_image"], true);
}

#[tokio::test]
async fn v2_separation_sky_matches_reference() {
    let (state, _dir) = seed_wcs_session("s-sep-sky").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-sep-sky/wcs/separation",
        r#"{"type":"sky","a":[150.0,2.0],"b":[150.0,3.0]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!((json["separation_deg"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert!((json["separation_arcsec"].as_f64().unwrap() - 3600.0).abs() < 1e-4);
}

#[tokio::test]
async fn v2_separation_pixel_uses_wcs() {
    let (state, _dir) = seed_wcs_session("s-sep-pix").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-sep-pix/wcs/separation",
        r#"{"type":"pixel","a":[3,3],"b":[3,4]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!((json["separation_arcsec"].as_f64().unwrap() - 0.36).abs() < 1e-3);
    assert_eq!(json["ref"], "img_0");
    assert!(json["a_sky"].is_array());
}

#[tokio::test]
async fn v2_wcs_without_header_returns_wcs_required() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("nowcs.fits");
    v2_fixtures::write_no_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-nowcs");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-nowcs/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-nowcs/wcs/pix2sky",
        r#"{"points":[[1,1]]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "wcs_required");
}

async fn get_uri(app: axum::Router, uri: &str) -> axum::response::Response {
    app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn v2_structure_lists_every_hdu() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("mef.fits");
    v2_fixtures::write_mef_fits(&fits, (8, 8), (16, 4));

    let state = AppState::new(cfg());
    seed_session(&state, "s-struct");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-struct/open",
        &format!(r#"{{"path":{},"hdu":0}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = get_uri(build_router(state), "/v2/sessions/s-struct/structure").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);

    let h0 = &json["hdus"][0];
    assert_eq!(h0["index"], 0);
    assert_eq!(h0["bitpix"], -32);
    assert_eq!(h0["dtype"], "float32");
    assert_eq!(h0["has_data"], true);
    assert_eq!(h0["shape"], serde_json::json!([8, 8]));

    let h1 = &json["hdus"][1];
    assert_eq!(h1["index"], 1);
    assert_eq!(h1["extname"], "WEIGHT");
    assert_eq!(h1["shape"], serde_json::json!([4, 16]));
}

#[tokio::test]
async fn v2_header_full_subset_and_glob() {
    let (state, _dir) = seed_wcs_session("s-hdr").await;

    let resp = get_uri(build_router(state.clone()), "/v2/sessions/s-hdr/header").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["cards"]["CTYPE1"]["value"].is_string());
    assert!(json["cards"]["NAXIS1"]["value"].is_string());

    let resp = get_uri(
        build_router(state.clone()),
        "/v2/sessions/s-hdr/header?keys=CTYPE1,CRVAL1",
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    assert_eq!(json["cards"]["CTYPE1"]["value"], "RA---TAN");
    assert_eq!(json["cards"]["CRVAL1"]["value"], "150.0");
    assert!(json["cards"].get("CD1_1").is_none());

    let resp = get_uri(build_router(state), "/v2/sessions/s-hdr/header?keys=CD*_*").await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 4);
    for k in ["CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
        assert!(json["cards"][k]["value"].is_string(), "missing {k}");
    }
    assert!(json["cards"].get("CTYPE1").is_none());
}

#[tokio::test]
async fn v2_wcs_summary_reports_projection_scale_and_orientation() {
    let (state, _dir) = seed_wcs_session("s-wcs-sum").await;

    let resp = get_uri(build_router(state), "/v2/sessions/s-wcs-sum/wcs").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["present"], true);
    assert_eq!(json["projection"], "TAN");
    assert_eq!(json["crval"], serde_json::json!([150.0, 2.0]));
    assert_eq!(json["crpix"], serde_json::json!([4.0, 4.0]));

    assert!((json["pixel_scale_x_arcsec"].as_f64().unwrap() - 0.36).abs() < 1e-9);
    assert!((json["pixel_scale_y_arcsec"].as_f64().unwrap() - 0.36).abs() < 1e-9);
    assert!(json["rotation_deg"].as_f64().unwrap().abs() < 1e-9);
    assert_eq!(json["flipped"], false);
    assert_eq!(json["parity"], "normal");
    assert_eq!(json["sip_present"], false);
}

#[tokio::test]
async fn v2_wcs_summary_no_wcs_returns_present_false() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("nowcs.fits");
    v2_fixtures::write_no_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-wcs-none");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-wcs-none/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = get_uri(build_router(state), "/v2/sessions/s-wcs-none/wcs").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["present"], false);
}

#[tokio::test]
async fn v2_cutout_pixel_fully_on_image_crops_and_shifts_wcs() {
    let (state, _dir) = seed_wcs_session("s-cut-px").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-cut-px/cutout",
        r#"{"region":{"type":"pixel","x":2,"y":2,"width":4,"height":4}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert_eq!(json["dims"], serde_json::json!([4, 4]));
    assert!((json["fraction_on_image"].as_f64().unwrap() - 1.0).abs() < 1e-12);
    assert_eq!(json["wcs_present"], true);
    assert!((json["stats"]["min"].as_f64().unwrap() - 18.0).abs() < 1e-6);
    assert!((json["stats"]["max"].as_f64().unwrap() - 45.0).abs() < 1e-6);
    assert_eq!(json["stats"]["valid_count"], 16);

    let cut_ref = json["ref"].as_str().unwrap().to_owned();

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-cut-px/wcs/pix2sky",
        &format!(r#"{{"points":[[1,1]],"ref":"{cut_ref}"}}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let r0 = &json["results"][0];
    assert!((r0["ra"].as_f64().unwrap() - 150.0).abs() < 1e-6);
    assert!((r0["dec"].as_f64().unwrap() - 2.0).abs() < 1e-6);
}

#[tokio::test]
async fn v2_cutout_sky_region_resolves_against_wcs() {
    let (state, _dir) = seed_wcs_session("s-cut-sky").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-cut-sky/cutout",
        r#"{"region":{"type":"sky","ra":150.0,"dec":2.0,"size_arcmin":0.024}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dims"], serde_json::json!([4, 4]));
    assert!((json["fraction_on_image"].as_f64().unwrap() - 1.0).abs() < 1e-12);
    assert_eq!(json["wcs_present"], true);
}

#[tokio::test]
async fn v2_cutout_partial_overlap_nan_fills_and_reports_fraction() {
    let (state, _dir) = seed_wcs_session("s-cut-part").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-cut-part/cutout",
        r#"{"region":{"type":"pixel","x":6,"y":6,"width":4,"height":4}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dims"], serde_json::json!([4, 4]));
    let frac = json["fraction_on_image"].as_f64().unwrap();
    assert!(frac < 1.0);
    assert!((frac - 0.25).abs() < 1e-12);
    assert_eq!(json["stats"]["valid_count"], 4);
    assert_eq!(json["ltv1"], serde_json::json!(-6.0));
    assert_eq!(json["ltv2"], serde_json::json!(-6.0));
}

#[tokio::test]
async fn v2_cutout_sky_without_wcs_errors_wcs_required() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("nowcs.fits");
    v2_fixtures::write_no_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-cut-nowcs");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-cut-nowcs/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-cut-nowcs/cutout",
        r#"{"region":{"type":"sky","ra":150.0,"dec":2.0,"size_arcmin":1.0}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "wcs_required");
}

fn seed_synthetic_image(
    state: &AppState,
    sid: &str,
    image_ref: &str,
    arr: ndarray::Array2<f32>,
) -> Arc<Session> {
    seed_session(state, sid);
    let session = state.sessions.get(sid).unwrap().clone();
    let stats = astroburst_lib::core::imaging::stats::compute_image_stats(&arr);
    session
        .cache
        .insert_synthetic(image_ref, Arc::new(arr), stats);
    *session
        .v2
        .active_ref
        .try_write()
        .expect("no lock contention while seeding") = Some(image_ref.to_string());
    session
}

#[tokio::test]
async fn v2_bin_mean_matches_hand_computed_block_average_and_ignores_nan() {
    let arr = ndarray::Array2::from_shape_vec(
        (4, 4),
        vec![
            1.0, 2.0, 10.0, 20.0,
            3.0, 4.0, 30.0, 40.0,
            100.0, 200.0, f32::NAN, 9.0,
            300.0, 400.0, 9.0, 9.0,
        ],
    )
    .unwrap();

    let state = AppState::new(cfg());
    let session = seed_synthetic_image(&state, "s-bin", "img_0", arr);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-bin/bin",
        r#"{"factor":2,"method":"mean","ref":"img_0"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ref"], "bin_0");
    assert_eq!(json["active_ref"], "bin_0");
    assert_eq!(json["from_ref"], "img_0");
    assert_eq!(json["dims"], serde_json::json!([2, 2]));
    assert_eq!(json["method"], "mean");

    let out = session.cache.get("bin_0").unwrap();
    let a = out.arr();
    assert_eq!(a.dim(), (2, 2));
    assert!((a[[0, 0]] - 2.5).abs() < 1e-4);
    assert!((a[[0, 1]] - 25.0).abs() < 1e-4);
    assert!((a[[1, 0]] - 250.0).abs() < 1e-4);
    assert!(a[[1, 1]].is_finite());
    assert!((a[[1, 1]] - 9.0).abs() < 1e-4);
}

#[tokio::test]
async fn v2_bin_sum_method_returns_bad_request() {
    let arr = ndarray::Array2::from_elem((4, 4), 1.0f32);
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-bin-sum", "img_0", arr);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-bin-sum/bin",
        r#"{"factor":2,"method":"sum","ref":"img_0"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn v2_pixel_value_and_box_stats_and_sky() {
    let (state, _dir) = seed_wcs_session("s-px").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px/pixel",
        r#"{"x":3,"y":3,"box":5}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert!((json["value"].as_f64().unwrap() - 27.0).abs() < 1e-6);
    let nb = &json["neighborhood"];
    assert!((nb["min"].as_f64().unwrap() - 9.0).abs() < 1e-6);
    assert!((nb["max"].as_f64().unwrap() - 45.0).abs() < 1e-6);
    assert!((nb["mean"].as_f64().unwrap() - 27.0).abs() < 1e-6);
    assert!((nb["median"].as_f64().unwrap() - 27.0).abs() < 1e-6);
    assert_eq!(nb["n_pixels"], 25);
    assert_eq!(nb["n_nan"], 0);
    assert!(json["unit"].is_null(), "fixture without BUNIT must report null unit");
    assert_eq!(json["ref"], "img_0");
    assert_eq!(json["box"], 5);

    assert!((json["sky"]["ra"].as_f64().unwrap() - 150.0).abs() < 1e-6);
    assert!((json["sky"]["dec"].as_f64().unwrap() - 2.0).abs() < 1e-6);
}

#[tokio::test]
async fn v2_pixel_reports_bunit_and_median() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("bunit.fits");
    v2_fixtures::write_bunit_fits(&fits, 8, 8, "MJy/sr  ");

    let state = AppState::new(cfg());
    seed_session(&state, "s-px-unit");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-px-unit/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px-unit/pixel",
        r#"{"x":3.7,"y":3.2,"box":3}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["unit"], "MJy/sr");
    assert_eq!(json["x"], 4);
    assert_eq!(json["y"], 3);
    assert!((json["value"].as_f64().unwrap() - 28.0).abs() < 1e-6);
    let nb = &json["neighborhood"];
    assert_eq!(nb["n_pixels"], 9);
    assert!((nb["median"].as_f64().unwrap() - 28.0).abs() < 1e-6);
    assert!((nb["min"].as_f64().unwrap() - 19.0).abs() < 1e-6);
    assert!((nb["max"].as_f64().unwrap() - 37.0).abs() < 1e-6);
    assert!(json["sky"]["ra"].is_number());
}

#[tokio::test]
async fn v2_pixel_even_box_is_bad_request() {
    let (state, _dir) = seed_wcs_session("s-px-even").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px-even/pixel",
        r#"{"x":3,"y":3,"box":4}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn v2_pixel_box_clipped_at_image_edge() {
    let (state, _dir) = seed_wcs_session("s-px-edge").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px-edge/pixel",
        r#"{"x":0,"y":0,"box":5}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!((json["value"].as_f64().unwrap() - 0.0).abs() < 1e-6);
    assert_eq!(json["neighborhood"]["n_pixels"], 9);
    assert!((json["neighborhood"]["max"].as_f64().unwrap() - 18.0).abs() < 1e-6);
}

#[tokio::test]
async fn v2_pixel_without_wcs_omits_sky() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("nowcs.fits");
    v2_fixtures::write_no_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-px-nowcs");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-px-nowcs/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px-nowcs/pixel",
        r#"{"x":2,"y":1,"box":3}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!((json["value"].as_f64().unwrap() - 10.0).abs() < 1e-6);
    assert!(json["sky"].is_null());
}

#[tokio::test]
async fn v2_pixel_out_of_bounds_errors_not_panics() {
    let (state, _dir) = seed_wcs_session("s-px-oob").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-px-oob/pixel",
        r#"{"x":100,"y":100,"box":5}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "pixel_out_of_bounds");
}

async fn seed_stats_session(id: &str) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("stats.fits");

    let (w, h) = (10usize, 10usize);
    let mut pixels = vec![1.0f32; w * h];
    let region_vals: [f32; 16] = [
        100.0, 101.0, 102.0, 103.0,
        104.0, 105.0, 106.0, 107.0,
        108.0, 109.0, 110.0, 111.0,
        112.0, 8000.0, 9000.0, f32::NAN,
    ];
    for y in 2..6 {
        for x in 2..6 {
            pixels[y * w + x] = region_vals[(y - 2) * 4 + (x - 2)];
        }
    }
    v2_fixtures::write_pixels_fits(&fits, w, h, &pixels);

    let state = AppState::new(cfg());
    seed_session(&state, id);
    post_json(
        build_router(state.clone()),
        &format!("/v2/sessions/{id}/open"),
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;
    (state, dir)
}

#[tokio::test]
async fn v2_stats_region_base_sigma_clip_and_percentiles() {
    let (state, _dir) = seed_stats_session("s-stats").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats/stats",
        r#"{
            "region": {"type":"pixel","x":2,"y":2,"width":4,"height":4},
            "sigma_clip": {"sigma": 3.0, "maxiters": 5},
            "percentiles": [16, 50, 84]
        }"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert_eq!(json["valid_count"], 15);
    assert_eq!(json["n_nan"], 1);
    assert_eq!(json["min"], 100.0);
    assert_eq!(json["max"], 9000.0);
    assert!((json["mean"].as_f64().unwrap() - 1225.2).abs() < 0.5);
    assert_eq!(json["median"], 107.0);
    assert_eq!(json["mad"], 4.0);
    assert!((json["sigma"].as_f64().unwrap() - 4.0 * 1.4826).abs() < 1e-3);

    assert_eq!(json["region"]["x"], 2);
    assert_eq!(json["region"]["y"], 2);
    assert_eq!(json["region"]["width"], 4);
    assert_eq!(json["region"]["height"], 4);
    assert_eq!(json["region"]["clipped"], false);

    let c = &json["clipped"];
    assert_eq!(c["n_rejected"], 2);
    assert!((c["mean"].as_f64().unwrap() - 106.0).abs() < 1e-6);
    assert!((c["median"].as_f64().unwrap() - 106.0).abs() < 1e-6);
    assert!((c["std"].as_f64().unwrap() - 3.0 * 1.4826).abs() < 1e-3);

    let p = &json["percentiles"];
    assert_eq!(p[0]["percentile"], 16.0);
    assert_eq!(p[0]["value"], 102.0);
    assert_eq!(p[1]["value"], 107.0);
    assert_eq!(p[2]["value"], 112.0);
}

#[tokio::test]
async fn v2_stats_without_sigma_clip_omits_clipped_block() {
    let (state, _dir) = seed_stats_session("s-stats-noclip").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-noclip/stats",
        r#"{"region":{"type":"pixel","x":2,"y":2,"width":4,"height":4}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.get("clipped").is_none() || json["clipped"].is_null());
    assert!(json.get("percentiles").is_none() || json["percentiles"].is_null());
}

#[tokio::test]
async fn v2_stats_region_out_of_bounds_then_clips() {
    let (state, _dir) = seed_stats_session("s-stats-oob").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-oob/stats",
        r#"{"region":{"type":"pixel","x":5,"y":0,"width":8,"height":4}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "region_out_of_bounds");

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-oob/stats",
        r#"{"region":{"type":"pixel","x":5,"y":0,"width":8,"height":4,"clip":true}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["region"]["clipped"], true);
    assert_eq!(json["region"]["width"], 5);
}

#[tokio::test]
async fn v2_stats_full_frame_when_no_region() {
    let (state, _dir) = seed_stats_session("s-stats-full").await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-full/stats",
        r#"{}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["valid_count"], 99);
    assert_eq!(json["n_nan"], 1);
    assert_eq!(json["region"]["width"], 10);
    assert_eq!(json["region"]["height"], 10);
}

fn hist_ramp_4x4() -> ndarray::Array2<f32> {
    ndarray::Array2::from_shape_vec((4, 4), (1..=16).map(|i| i as f32).collect()).unwrap()
}

#[tokio::test]
async fn v2_histogram_default_auto_range_matches_build_histogram() {
    let arr = hist_ramp_4x4();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist", "img_0", arr.clone());

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-hist/histogram",
        r#"{"bins":8,"ref":"img_0"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    let expected = astroburst_lib::core::imaging::stats::build_histogram(arr.as_slice().unwrap(), 8, 1.0, 16.0);
    let got: Vec<u64> = json["bins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let exp: Vec<u64> = expected.bins.iter().map(|&c| c as u64).collect();
    assert_eq!(got, exp);

    let edges: Vec<f64> = json["bin_edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(edges.len(), expected.bin_edges.len());
    for (g, e) in edges.iter().zip(expected.bin_edges.iter()) {
        assert!((g - e).abs() < 1e-9, "edge {g} vs {e}");
    }

    assert_eq!(json["range_source"], "auto");
    assert_eq!(json["log_counts"], false);
    assert_eq!(json["min"], 1.0);
    assert_eq!(json["max"], 16.0);
}

#[tokio::test]
async fn v2_histogram_auto_range_excludes_outlier() {
    let mut vals: Vec<f32> = (0..1023).map(|i| 100.0 + i as f32 * 0.1).collect();
    vals.push(1.0e6);
    let arr = ndarray::Array2::from_shape_vec((32, 32), vals).unwrap();
    let raw_max = arr.iter().copied().fold(f32::MIN, f32::max);
    assert_eq!(raw_max, 1.0e6);

    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist-out", "img_0", arr.clone());

    let resp = post_json(
        build_router(state),
        r#"/v2/sessions/s-hist-out/histogram"#,
        r#"{"bins":10,"range":null}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert_eq!(json["range_source"], "auto");
    let hi = json["max"].as_f64().unwrap();
    assert!(hi < 300.0, "auto-range max should exclude the 1e6 outlier, got {hi}");
    assert!(hi > 200.0, "auto-range max should still cover the band top, got {hi}");
    let lo = json["min"].as_f64().unwrap();
    assert!(lo >= 100.0 && lo < 110.0, "auto-range min ~band bottom, got {lo}");

    let in_range = arr.iter().filter(|&&v| (v as f64) >= lo && (v as f64) <= hi).count() as u64;
    let counted: u64 = json["bins"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).sum();
    assert_eq!(counted, in_range, "bins must hold exactly the in-range pixels");
    assert!(in_range < 1024);
}

#[tokio::test]
async fn v2_histogram_explicit_range_and_log_counts() {
    let arr = hist_ramp_4x4();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist-log", "img_0", arr.clone());

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-hist-log/histogram",
        r#"{"bins":8,"range":[1,16],"log_counts":true}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert_eq!(json["range_source"], "explicit");
    assert_eq!(json["log_counts"], true);
    assert_eq!(json["min"], 1.0);
    assert_eq!(json["max"], 16.0);

    let expected = astroburst_lib::core::imaging::stats::build_histogram(
        arr.as_slice().unwrap(),
        8,
        1.0,
        16.0,
    );
    let got: Vec<f64> = json["bins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(got.len(), expected.bins.len());
    for (g, &c) in got.iter().zip(expected.bins.iter()) {
        assert!((g - (c as f64 + 1.0).ln()).abs() < 1e-9, "log count {g} vs {c}");
    }
}

#[tokio::test]
async fn v2_histogram_region_out_of_bounds_then_clips() {
    let arr = hist_ramp_4x4();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist-oob", "img_0", arr);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-hist-oob/histogram",
        r#"{"bins":4,"region":{"type":"pixel","x":2,"y":0,"width":4,"height":2}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "region_out_of_bounds");

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-hist-oob/histogram",
        r#"{"bins":4,"region":{"type":"pixel","x":2,"y":0,"width":4,"height":2,"clip":true}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["region"]["clipped"], true);
    assert_eq!(json["region"]["width"], 2);
}

#[tokio::test]
async fn v2_histogram_render_png_is_rejected_not_silent() {
    let arr = hist_ramp_4x4();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist-png", "img_0", arr);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-hist-png/histogram",
        r#"{"bins":8,"render_png":true}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "not_implemented");
}

fn resolved_header(resp: &axum::response::Response) -> serde_json::Value {
    let raw = resp
        .headers()
        .get("x-render-resolved")
        .expect("x-render-resolved header present")
        .to_str()
        .unwrap();
    serde_json::from_str(raw).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

fn decode_rgb(bytes: &[u8]) -> image::RgbImage {
    image::load_from_memory(bytes).expect("valid PNG").to_rgb8()
}

fn render_ramp_8x8() -> ndarray::Array2<f32> {
    ndarray::Array2::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32)
}

#[tokio::test]
async fn v2_render_default_full_frame_is_valid_png() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-r", "img_0", render_ramp_8x8());

    let resp = post_json(build_router(state), "/v2/sessions/s-r/render", "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "image/png"
    );
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["scale_algorithm"], "zscale");
    assert_eq!(hdr["stretch"], "linear");
    assert_eq!(hdr["colormap"], "gray");
    assert_eq!(hdr["binning_applied"], 1);
    assert_eq!(hdr["region"], serde_json::json!({"x":0,"y":0,"w":8,"h":8}));

    let bytes = body_bytes(resp).await;
    let img = decode_rgb(&bytes);
    assert_eq!(img.dimensions(), (8, 8));
}

#[tokio::test]
async fn v2_render_manual_linear_gray_maps_endpoints_exactly() {
    let arr = ndarray::Array2::from_shape_vec((2, 2), vec![1.0f32, 2.0, 3.0, 4.0]).unwrap();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rm", "img_0", arr);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rm/render",
        r#"{"scale":{"algorithm":"manual","vmin":1,"vmax":4,"stretch":"linear"},"colormap":"gray"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["vmin"], 1.0);
    assert_eq!(hdr["vmax"], 4.0);
    assert!((hdr["clipped_fraction"]["below_vmin"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    assert!((hdr["clipped_fraction"]["above_vmax"].as_f64().unwrap() - 0.25).abs() < 1e-9);

    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0]);
    assert_eq!(img.get_pixel(1, 1).0, [255, 255, 255]);
}

#[tokio::test]
async fn v2_render_scale_algorithms_and_stretches_differ() {
    let mut with_outliers = render_ramp_8x8();
    with_outliers[[0, 0]] = -1000.0;
    with_outliers[[7, 7]] = 5000.0;
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rd", "img_0", with_outliers);

    let mut outputs = Vec::new();
    let mut limits = std::collections::HashMap::new();
    for alg in ["zscale", "minmax", "percentile", "manual"] {
        let body = format!(
            r#"{{"scale":{{"algorithm":"{alg}","stretch":"linear","vmin":0,"vmax":40,"percentile":[10,90]}}}}"#
        );
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rd/render", &body).await;
        assert_eq!(resp.status(), StatusCode::OK, "alg {alg}");
        let hdr = resolved_header(&resp);
        limits.insert(alg, (hdr["vmin"].as_f64().unwrap(), hdr["vmax"].as_f64().unwrap()));
        outputs.push(body_bytes(resp).await);
    }
    let distinct: std::collections::HashSet<_> = outputs.iter().collect();
    assert_eq!(distinct.len(), 4, "every scale algorithm must produce its own rendering");

    assert_eq!(limits["minmax"], (-1000.0, 5000.0));
    assert_eq!(limits["manual"], (0.0, 40.0));
    let (zlo, zhi) = limits["zscale"];
    assert!(zlo > -1000.0 && zhi < 5000.0, "zscale must clip the outliers, got {zlo}..{zhi}");
    let (plo, phi) = limits["percentile"];
    assert!((1.0..=12.0).contains(&plo) && (51.0..=63.0).contains(&phi), "percentile 10..90 gave {plo}..{phi}");
    assert_ne!(limits["zscale"], limits["percentile"]);

    let mut stretched = Vec::new();
    for st in ["linear", "log", "sqrt", "asinh", "power"] {
        let body = format!(
            r#"{{"scale":{{"algorithm":"manual","vmin":0,"vmax":63,"stretch":"{st}"}}}}"#
        );
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rd/render", &body).await;
        assert_eq!(resp.status(), StatusCode::OK, "stretch {st}");
        stretched.push(body_bytes(resp).await);
    }
    let distinct: std::collections::HashSet<_> = stretched.iter().collect();
    assert_eq!(distinct.len(), 5, "all five stretches should differ on a ramp");
}

#[tokio::test]
async fn v2_render_viridis_is_genuinely_colored() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rv", "img_0", render_ramp_8x8());

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rv/render",
        r#"{"scale":{"algorithm":"minmax","stretch":"linear"},"colormap":"viridis"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["colormap"], "viridis");

    let img = decode_rgb(&body_bytes(resp).await);
    let colored = img.pixels().any(|p| p.0[0] != p.0[1] || p.0[1] != p.0[2]);
    assert!(colored, "viridis output should contain colored pixels");
}

#[tokio::test]
async fn v2_render_region_out_of_bounds_is_silently_clamped() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-roob", "img_0", render_ramp_8x8());

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-roob/render",
        r#"{"region":{"type":"pixel","x":6,"y":6,"width":10,"height":10}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "clamped region must not error");
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["region"], serde_json::json!({"x":6,"y":6,"w":2,"h":2}));
    assert_eq!(hdr["region_clipped"], true);
    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.dimensions(), (2, 2));
}

#[tokio::test]
async fn v2_render_max_dim_binning() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rb", "img_0", render_ramp_8x8());

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-rb/render",
        r#"{"max_dim":4}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["binning_applied"], 2);
    assert_eq!(decode_rgb(&body_bytes(resp).await).dimensions(), (4, 4));

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rb/render",
        r#"{"max_dim":16}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["binning_applied"], 1);
    assert_eq!(decode_rgb(&body_bytes(resp).await).dimensions(), (8, 8));
}

#[tokio::test]
async fn v2_render_crosshair_pixel_and_sky_locations() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("wcs.fits");
    v2_fixtures::write_wcs_fits(&fits, 8, 8);
    let state = AppState::new(cfg());
    seed_session(&state, "s-rx");
    let body = format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap());
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rx/open", &body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-rx/render",
        r#"{"overlays":[{"type":"crosshair","x":2,"y":5}]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.get_pixel(2, 5).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(2, 0).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(0, 5).0, [255, 0, 0]);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rx/render",
        r#"{"overlays":[{"type":"crosshair","ra":150.0,"dec":2.0}]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.get_pixel(3, 3).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(3, 0).0, [255, 0, 0]);
}

#[tokio::test]
async fn v2_render_scalebar_without_wcs_is_silently_omitted() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rsb", "img_0", render_ramp_8x8());

    let plain = post_json(build_router(state.clone()), "/v2/sessions/s-rsb/render", "{}").await;
    assert_eq!(plain.status(), StatusCode::OK);
    let plain_bytes = body_bytes(plain).await;

    let with_bar = post_json(
        build_router(state),
        "/v2/sessions/s-rsb/render",
        r#"{"overlays":[{"type":"scalebar","length_arcsec":30}]}"#,
    )
    .await;
    assert_eq!(with_bar.status(), StatusCode::OK, "must not error");
    let bar_bytes = body_bytes(with_bar).await;
    assert_eq!(plain_bytes, bar_bytes);
}

#[tokio::test]
async fn v2_render_is_stateless_no_new_ref_or_active_change() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("wcs.fits");
    v2_fixtures::write_wcs_fits(&fits, 8, 8);
    let state = AppState::new(cfg());
    seed_session(&state, "s-rs");
    let body = format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap());
    post_json(build_router(state.clone()), "/v2/sessions/s-rs/open", &body).await;

    let before = body_json(get_uri(build_router(state.clone()), "/v2/sessions/s-rs/images").await).await;

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rs/render", "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let after = body_json(get_uri(build_router(state), "/v2/sessions/s-rs/images").await).await;
    assert_eq!(before["active_ref"], after["active_ref"]);
    assert_eq!(before["count"], after["count"]);
    assert_eq!(before["images"], after["images"]);
}

#[tokio::test]
async fn v2_render_unknown_colormap_and_stretch_are_bad_request() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-re", "img_0", render_ramp_8x8());

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-re/render",
        r#"{"colormap":"bone"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-re/render",
        r#"{"scale":{"stretch":"histeq"}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-re/render",
        r#"{"scale":{"algorithm":"bogus"}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v2_render_every_colormap_is_accepted_and_inferno_is_colored() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-rc", "img_0", render_ramp_8x8());

    for name in ["gray", "grey", "viridis", "inferno", "magma", "plasma", "cividis", "heat", "cool", "rainbow"] {
        let body = format!(r#"{{"scale":{{"algorithm":"minmax","stretch":"linear"}},"colormap":"{name}"}}"#);
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rc/render", &body).await;
        assert_eq!(resp.status(), StatusCode::OK, "colormap {name}");
    }

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rc/render",
        r#"{"scale":{"algorithm":"minmax","stretch":"linear"},"colormap":"inferno"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["colormap"], "inferno");
    let img = decode_rgb(&body_bytes(resp).await);
    for (x, y) in [(0u32, 0u32), (7, 0), (0, 7), (7, 7)] {
        let p = img.get_pixel(x, y).0;
        assert!(p[0] != p[1] || p[1] != p[2], "corner ({x},{y}) should not be gray: {p:?}");
    }
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 4]);
    assert_eq!(img.get_pixel(7, 7).0, [252, 255, 164]);
}

#[tokio::test]
async fn v2_render_user_and_manual_both_resolve_and_echo_user() {
    let arr = ndarray::Array2::from_shape_vec((2, 2), vec![1.0f32, 2.0, 3.0, 4.0]).unwrap();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-ru", "img_0", arr);

    let mut outputs = Vec::new();
    for alg in ["user", "manual"] {
        let body = format!(
            r#"{{"scale":{{"algorithm":"{alg}","vmin":1,"vmax":4,"stretch":"linear"}},"colormap":"gray"}}"#
        );
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-ru/render", &body).await;
        assert_eq!(resp.status(), StatusCode::OK, "alg {alg}");
        let hdr = resolved_header(&resp);
        assert_eq!(hdr["scale_algorithm"], "user", "alg {alg}");
        assert_eq!(hdr["vmin"], 1.0);
        assert_eq!(hdr["vmax"], 4.0);
        outputs.push(body_bytes(resp).await);
    }
    assert_eq!(outputs[0], outputs[1]);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-ru/render",
        r#"{"scale":{"algorithm":"user","stretch":"linear"}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = resolved_header(&resp);
    assert_eq!(hdr["scale_algorithm"], "user");
    assert_eq!(hdr["vmin"], 1.0);
    assert_eq!(hdr["vmax"], 4.0);
}

#[tokio::test]
async fn v2_render_invert_cmap_flips_gray_endpoints() {
    let arr = ndarray::Array2::from_shape_vec((2, 2), vec![1.0f32, 2.0, 3.0, 4.0]).unwrap();
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-ri", "img_0", arr);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-ri/render",
        r#"{"scale":{"algorithm":"user","vmin":1,"vmax":4,"stretch":"linear"},"colormap":"gray","invert_cmap":true}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255]);
    assert_eq!(img.get_pixel(1, 1).0, [0, 0, 0]);
    assert_eq!(img.get_pixel(1, 0).0, [170, 170, 170]);
}

fn stats_fixture_array() -> ndarray::Array2<f32> {
    let (w, h) = (10usize, 10usize);
    let mut pixels = vec![1.0f32; w * h];
    let region_vals: [f32; 16] = [
        100.0, 101.0, 102.0, 103.0,
        104.0, 105.0, 106.0, 107.0,
        108.0, 109.0, 110.0, 111.0,
        112.0, 8000.0, 9000.0, f32::NAN,
    ];
    for y in 2..6 {
        for x in 2..6 {
            pixels[y * w + x] = region_vals[(y - 2) * 4 + (x - 2)];
        }
    }
    ndarray::Array2::from_shape_vec((h, w), pixels).unwrap()
}

#[tokio::test]
async fn v2_stats_shape_region_matches_core_masked_values() {
    let (state, _dir) = seed_stats_session("s-stats-shape").await;
    let shape = astroburst_lib::core::imaging::region::RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 };
    let expected = shape.masked_values(&stats_fixture_array(), None);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-shape/stats",
        r#"{"region":{"type":"shape","shape":"circle","x":4.5,"y":4.5,"r":2}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["valid_count"], expected.values.len() as u64);
    assert_eq!(json["n_nan"], expected.n_nan);
    assert_eq!(json["region"]["shape"], "circle");
    assert_eq!(json["region"]["bounds"]["x0"], 2);
    assert_eq!(json["region"]["bounds"]["x1"], 7);
    assert_eq!(json["region"]["clipped"], false);
    let sum: f64 = expected.values.iter().map(|&v| v as f64).sum();
    assert!((json["sum"].as_f64().unwrap() - sum).abs() < 1e-6);
    assert!((json["area"].as_f64().unwrap() - std::f64::consts::PI * 4.0).abs() < 1e-9);
}

#[tokio::test]
async fn v2_histogram_shape_region_bins_sum_to_valid_count() {
    let (state, _dir) = seed_stats_session("s-hist-shape").await;
    let shape = astroburst_lib::core::imaging::region::RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 };
    let expected = shape.masked_values(&stats_fixture_array(), None);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-hist-shape/histogram",
        r#"{"region":{"type":"shape","shape":"circle","x":4.5,"y":4.5,"r":2},"bins":8}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let total: u64 = json["bins"].as_array().unwrap().iter().map(|b| b.as_u64().unwrap()).sum();
    assert_eq!(total, expected.values.len() as u64);
    assert_eq!(json["region"]["shape"], "circle");
    assert!(json["region"]["bounds"].is_object());
}

#[tokio::test]
async fn v2_stats_sky_shape_without_wcs_errors_wcs_required() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("nowcs-shape.fits");
    v2_fixtures::write_no_wcs_fits(&fits, 8, 8);

    let state = AppState::new(cfg());
    seed_session(&state, "s-stats-nowcs");
    post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-nowcs/open",
        &format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap()),
    )
    .await;

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-nowcs/stats",
        r#"{"region":{"type":"shape","shape":"circle","x":150.0,"y":2.0,"r":3,"system":"fk5"}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "wcs_required");
}

#[tokio::test]
async fn v2_stats_shape_out_of_bounds_then_clips() {
    let (state, _dir) = seed_stats_session("s-stats-shape-oob").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-shape-oob/stats",
        r#"{"region":{"type":"shape","shape":"circle","x":8,"y":4,"r":3}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "region_out_of_bounds");
    assert!(json["error"]["hint"].as_str().unwrap().contains("clip=true"));

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-shape-oob/stats",
        r#"{"region":{"type":"shape","shape":"circle","x":8,"y":4,"r":3,"clip":true}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["region"]["clipped"], true);
    let expected = astroburst_lib::core::imaging::region::RegionShape::Circle { x: 8.0, y: 4.0, r: 3.0 }
        .masked_values(&stats_fixture_array(), None);
    assert_eq!(json["valid_count"], expected.values.len() as u64);
}

#[tokio::test]
async fn v2_cutout_shape_with_mask_outside_nan_fills_outside_the_shape() {
    let (state, _dir) = seed_wcs_session("s-cut-shape").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-cut-shape/cutout",
        r#"{"region":{"type":"shape","shape":"circle","x":4,"y":4,"r":2},"mask_outside":true}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dims"], serde_json::json!([5, 5]));
    assert_eq!(json["stats"]["valid_count"], 13);
    assert_eq!(json["region"]["width"], 5);
    assert_eq!(json["region"]["height"], 5);
    assert_eq!(json["region"]["x"], 2);
    assert_eq!(json["region"]["shape"]["shape"], "circle");
    assert_eq!(json["region"]["shape"]["r"], 2.0);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-cut-shape/cutout",
        r#"{"region":{"type":"shape","shape":"circle","x":4,"y":4,"r":2},"ref":"img_0"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["stats"]["valid_count"], 25);
}

#[tokio::test]
async fn stack_accepts_named_rejection_and_combine_and_rejects_unknown_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..5 {
        let p = dir.path().join(format!("frame{i}.fits"));
        let pixels: Vec<f32> = (0..64)
            .map(|k| if i == 2 && k == 10 { 60000.0 } else { 100.0 + (k % 4) as f32 })
            .collect();
        v2_fixtures::write_pixels_fits(&p, 8, 8, &pixels);
        paths.push(p.to_str().unwrap().to_string());
    }

    let state = AppState::new(cfg());
    seed_session(&state, "s-stack-rej");

    let body = serde_json::json!({
        "paths": paths,
        "align": false,
        "rejection": "winsorized_sigma_clip",
        "combine": "median",
        "normalization": "none",
        "result_slot": "rej"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-stack-rej/stacking/stack", &body).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = body_json(resp).await;
    let jid = json["job_id"].as_str().unwrap().to_string();

    let mut status = String::new();
    for _ in 0..400 {
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/s-stack-rej/jobs/{jid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        status = body_json(resp).await["status"].as_str().unwrap().to_string();
        if status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(status, "done");

    let session = state.sessions.get("s-stack-rej").unwrap();
    let entry = session.cache.get("rej").expect("stacked slot");
    assert!((entry.arr()[[1, 2]] - 102.0).abs() < 1e-3, "cosmic ray leaked: {}", entry.arr()[[1, 2]]);
    assert!(entry.arr().iter().all(|v| *v < 200.0));
    drop(session);

    for bad in [
        serde_json::json!({ "paths": paths, "rejection": "bogus" }),
        serde_json::json!({ "paths": paths, "combine": "mode" }),
        serde_json::json!({ "paths": paths, "normalization": "robust" }),
        serde_json::json!({ "paths": paths, "rejection_normalization": "scale" }),
    ] {
        let resp = post_json(build_router(state.clone()), "/sessions/s-stack-rej/stacking/stack", &bad.to_string()).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{bad}");
    }

    let bad_pipeline = serde_json::json!({
        "channels": [{ "label": "R", "paths": paths }],
        "rejection": "bogus"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-stack-rej/pipeline/run", &bad_pipeline).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bad_pipeline = serde_json::json!({
        "channels": [{ "label": "R", "paths": paths }],
        "combine": "mode"
    })
    .to_string();
    let resp = post_json(build_router(state), "/sessions/s-stack-rej/pipeline/run", &bad_pipeline).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

fn gaussian_noise_array(rows: usize, cols: usize, sigma: f64, seed: u64) -> ndarray::Array2<f32> {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    let mut rng = StdRng::seed_from_u64(seed);
    ndarray::Array2::from_shape_fn((rows, cols), |_| {
        let u1: f64 = rng.gen::<f64>().max(1e-30);
        let u2: f64 = rng.gen::<f64>();
        (sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    })
}

#[tokio::test]
async fn v2_stats_background_subtracted_frame_keeps_negative_pixels_and_adds_exact_keys() {
    let arr = ndarray::Array2::from_shape_vec(
        (2, 4),
        vec![-3.0f32, -2.0, -1.0, 0.0, 1.0, 2.0, 30.0, f32::NAN],
    )
    .unwrap();
    let core = astroburst_lib::core::imaging::stats::compute_image_stats(&arr);
    assert_eq!(core.valid_count, 6);
    assert_eq!(core.median, 0.0);
    assert_eq!(core.min, -3.0);

    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-stats-neg", "img_0", arr);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-stats-neg/stats", r#"{}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["valid_count"], 7);
    assert_eq!(json["median"], 0.0);
    assert_eq!(json["min"], -3.0);
    assert_eq!(json["nan_count"], 1);
    assert!((json["count_fraction"].as_f64().unwrap() - 7.0 / 8.0).abs() < 1e-12);
    let avg_dev = (3.0 + 2.0 + 1.0 + 0.0 + 1.0 + 2.0 + 30.0) / 7.0;
    assert!((json["avg_dev"].as_f64().unwrap() - avg_dev).abs() < 1e-9);
    assert_eq!(json["mad"], 2.0);
    let bwmv = json["bwmv_sqrt"].as_f64().unwrap();
    assert!(bwmv > 0.0 && bwmv < json["std_dev"].as_f64().unwrap(), "bwmv {bwmv}");
    assert!(json.get("noise").is_none() || json["noise"].is_null());

    let resp = post_json(build_router(state), "/v2/sessions/s-stats-neg/stats", r#"{"noise": true}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["median"], 0.0);
    assert_eq!(json["noise"]["method"], "k-sigma-mrs");
    assert!(json["noise"]["sigma"].is_number());
    assert!(json["noise"]["fraction"].is_number());
}

#[tokio::test]
async fn v2_stats_noise_evaluation_follows_the_requested_region() {
    let mut arr = gaussian_noise_array(128, 128, 2.0, 77);
    let quiet = gaussian_noise_array(128, 128, 0.5, 78);
    for y in 0..64 {
        for x in 0..128 {
            arr[[y, x]] = quiet[[y, x]];
        }
    }
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-stats-noise", "img_0", arr);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-noise/stats",
        r#"{"noise": true, "region": {"type":"pixel","x":8,"y":72,"width":100,"height":48}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let loud = json["noise"]["sigma"].as_f64().unwrap();
    assert!((loud - 2.0).abs() / 2.0 < 0.1, "pixel region sigma {loud}");

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-noise/stats",
        r#"{"noise": true, "region": {"type":"shape","shape":"box","x":64,"y":30,"width":100,"height":40}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let quiet_sigma = json["noise"]["sigma"].as_f64().unwrap();
    assert!((quiet_sigma - 0.5).abs() / 0.5 < 0.1, "shape region sigma {quiet_sigma}");
    assert_eq!(json["region"]["shape"], "box");
}

async fn wait_for_job(state: &AppState, sid: &str, jid: &str) -> String {
    let mut status = String::new();
    for _ in 0..400 {
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/{sid}/jobs/{jid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        status = body_json(resp).await["status"].as_str().unwrap().to_string();
        if status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    status
}

#[tokio::test]
async fn pipeline_run_with_cosmetic_repairs_master_dark_hot_pixels_and_reports_dq_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let hot = 3usize * 8 + 2;
    let mut dark_paths = Vec::new();
    for i in 0..3 {
        let p = dir.path().join(format!("dark{i}.fits"));
        let pixels: Vec<f32> = (0..64)
            .map(|k| if k == hot { 5000.0 } else { 10.0 + ((k * 7 + i) % 5) as f32 })
            .collect();
        v2_fixtures::write_pixels_fits(&p, 8, 8, &pixels);
        dark_paths.push(p.to_str().unwrap().to_string());
    }
    let mut light_paths = Vec::new();
    for i in 0..4 {
        let p = dir.path().join(format!("light{i}.fits"));
        let pixels: Vec<f32> = (0..64)
            .map(|k| if k == hot { 9000.0 } else { 110.0 + ((k * 7 + i) % 5) as f32 })
            .collect();
        v2_fixtures::write_pixels_fits(&p, 8, 8, &pixels);
        light_paths.push(p.to_str().unwrap().to_string());
    }

    let state = AppState::new(cfg());
    seed_session(&state, "s-pipe-cos");

    let body = serde_json::json!({
        "channels": [{ "label": "R", "paths": light_paths }],
        "dark_paths": dark_paths,
        "align": false,
        "normalize": false,
        "rejection": "none",
        "result_prefix": "cos_",
        "cosmetic": { "use_master_dark": true, "dark_hot_sigma": 5 }
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-cos/pipeline/run", &body).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = body_json(resp).await;
    assert_eq!(json["warnings"], serde_json::json!([]));
    let jid = json["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-pipe-cos", &jid).await, "done");

    let session = state.sessions.get("s-pipe-cos").unwrap();
    let entry = session.cache.get("cos_R").expect("stacked slot");
    let repaired = entry.arr()[[3, 2]];
    assert!(repaired < 110.0, "hot pixel survived cosmetic correction: {repaired}");
    assert!(repaired > 90.0, "hot pixel over-corrected: {repaired}");
    assert!((entry.arr()[[0, 0]] - 100.0).abs() < 5.0, "background off: {}", entry.arr()[[0, 0]]);
    drop(session);

    let bad = serde_json::json!({
        "channels": [{ "label": "R", "paths": light_paths }],
        "cosmetic": { "dark_hot_sigma": -1 }
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-cos/pipeline/run", &bad).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let mef = dir.path().join("light_dq.fits");
    v2_fixtures::write_mef_with_dq(&mef, 8, 8);
    let with_dq = serde_json::json!({
        "channels": [{ "label": "L", "paths": [mef.to_str().unwrap()] }],
        "align": false,
        "normalize": false,
        "result_prefix": "dq_",
        "cosmetic": { "use_master_dark": true, "dark_hot_sigma": 5 }
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-cos/pipeline/run", &with_dq).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = body_json(resp).await;
    let warnings = json["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("DQ"), "{warnings:?}");
    let jid = json["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-pipe-cos", &jid).await, "done");

    let without_cosmetic = serde_json::json!({
        "channels": [{ "label": "L", "paths": [mef.to_str().unwrap()] }],
        "align": false,
        "result_prefix": "plain_"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-cos/pipeline/run", &without_cosmetic).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = body_json(resp).await;
    assert_eq!(json["warnings"], serde_json::json!([]));
    let jid = json["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-pipe-cos", &jid).await, "done");
}

#[tokio::test]
async fn v2_wcs_grid_handler_returns_lines_labels_and_steps() {
    use crate::error::AppError;
    use crate::extractors::SessionExtractor;
    use crate::v2::wcs::{grid, GridParams};

    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("grid.fits");
    v2_fixtures::write_wcs_fits(&fits, 64, 64);
    let state = AppState::new(cfg());
    seed_session(&state, "s-grid");
    let body = format!(r#"{{"path":{}}}"#, serde_json::to_string(fits.to_str().unwrap()).unwrap());
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-grid/open", &body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let session = state.sessions.get("s-grid").map(|e| Arc::clone(e.value())).unwrap();
    let call = |params: GridParams| grid(SessionExtractor(Arc::clone(&session)), axum::Json(params));

    let json = call(GridParams { image: None, frame: None, density: None }).await.unwrap().0;
    assert_eq!(json["ref"], "img_0");
    assert_eq!(json["frame"], "icrs");
    assert!((json["lat_step_deg"].as_f64().unwrap() * 3600.0 - 5.0).abs() < 1e-9, "{}", json["lat_step_deg"]);
    assert!((json["lon_step_deg"].as_f64().unwrap() * 240.0 - 1.0).abs() < 1e-9, "{}", json["lon_step_deg"]);
    assert_eq!(json["notes"], serde_json::json!([]));
    let lines = json["lines"].as_array().unwrap();
    let lat_lines = lines.iter().filter(|l| l["kind"] == "lat").count();
    let lon_lines = lines.iter().filter(|l| l["kind"] == "lon").count();
    assert!(lat_lines >= 3, "{lat_lines} latitude lines");
    assert!(lon_lines >= 1, "{lon_lines} longitude lines");
    for line in lines {
        assert!(line["label"].is_string());
        let points = line["points"].as_array().unwrap();
        assert!(points.len() >= 2);
        for p in points {
            let x = p[0].as_f64().unwrap();
            let y = p[1].as_f64().unwrap();
            assert!((-1.5..=64.5).contains(&x) && (-1.5..=64.5).contains(&y), "point ({x},{y}) outside the image");
        }
    }
    let labels = json["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l["edge"] == "left" && l["kind"] == "lat"), "{labels:?}");
    assert!(labels.iter().any(|l| l["edge"] == "bottom" && l["kind"] == "lon"), "{labels:?}");
    assert!(labels.iter().all(|l| l["text"].is_string() && l["x"].is_number() && l["y"].is_number()));

    let galactic = call(GridParams { image: Some("img_0".into()), frame: Some("galactic".into()), density: Some(5) })
        .await
        .unwrap()
        .0;
    assert_eq!(galactic["frame"], "galactic");
    assert!(galactic["lines"].as_array().unwrap().len() > lines.len());
    let lon_labels: Vec<&str> = galactic["labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["kind"] == "lon")
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert!(!lon_labels.is_empty());
    assert!(lon_labels.iter().all(|t| t.ends_with('°') && !t.contains('h')), "{lon_labels:?}");

    let err = call(GridParams { image: None, frame: Some("supergalactic".into()), density: None }).await.err().unwrap();
    assert!(matches!(err, AppError::BadRequestWithHint { code: "bad_request", .. }), "{err:?}");
    let err = call(GridParams { image: None, frame: None, density: Some(9) }).await.err().unwrap();
    assert!(matches!(err, AppError::BadRequestWithHint { code: "bad_request", .. }), "{err:?}");
    let err = call(GridParams { image: Some("missing".into()), frame: None, density: None }).await.err().unwrap();
    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn drizzle_rejects_a_cosmic_ray_and_validates_the_rejection_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..5 {
        let p = dir.path().join(format!("drz{i}.fits"));
        let pixels: Vec<f32> = (0..64)
            .map(|k| if i == 2 && k == 10 { 60000.0 } else { 100.0 + (k % 4) as f32 })
            .collect();
        v2_fixtures::write_pixels_fits(&p, 8, 8, &pixels);
        paths.push(p.to_str().unwrap().to_string());
    }

    let state = AppState::new(cfg());
    seed_session(&state, "s-drz-rej");

    let body = serde_json::json!({
        "paths": paths,
        "align": false,
        "scale": 1.0,
        "pixfrac": 1.0,
        "rejection": "sigma_clip",
        "result_slot": "drz"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-drz-rej/stacking/drizzle", &body).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = body_json(resp).await;
    let jid = json["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-drz-rej", &jid).await, "done");

    let session = state.sessions.get("s-drz-rej").unwrap();
    let entry = session.cache.get("drz").expect("drizzled slot");
    assert_eq!(entry.arr().dim(), (8, 8));
    assert!((entry.arr()[[1, 2]] - 102.0).abs() < 1e-2, "cosmic ray leaked: {}", entry.arr()[[1, 2]]);
    drop(session);

    let leaking = serde_json::json!({
        "paths": paths,
        "align": false,
        "scale": 1.0,
        "pixfrac": 1.0,
        "rejection": "none",
        "result_slot": "drz_none"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-drz-rej/stacking/drizzle", &leaking).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let jid = body_json(resp).await["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-drz-rej", &jid).await, "done");
    let session = state.sessions.get("s-drz-rej").unwrap();
    let entry = session.cache.get("drz_none").expect("drizzled slot without rejection");
    assert!(entry.arr()[[1, 2]] > 1000.0, "rejection off still clipped: {}", entry.arr()[[1, 2]]);
    drop(session);

    let bad = serde_json::json!({ "paths": paths, "rejection": "bogus" }).to_string();
    let resp = post_json(build_router(state), "/sessions/s-drz-rej/stacking/drizzle", &bad).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pipeline_run_with_dark_optimize_recovers_the_scaled_dark() {
    let dir = tempfile::tempdir().unwrap();
    let pattern = |k: usize| ((k * 37) % 101) as f32;
    let mut bias_paths = Vec::new();
    let mut dark_paths = Vec::new();
    let mut light_paths = Vec::new();
    for i in 0..3 {
        let b = dir.path().join(format!("bias{i}.fits"));
        v2_fixtures::write_pixels_fits(&b, 8, 8, &vec![100.0f32; 64]);
        bias_paths.push(b.to_str().unwrap().to_string());

        let d = dir.path().join(format!("dark{i}.fits"));
        let dark: Vec<f32> = (0..64).map(|k| 100.0 + pattern(k)).collect();
        v2_fixtures::write_pixels_fits(&d, 8, 8, &dark);
        dark_paths.push(d.to_str().unwrap().to_string());

        let l = dir.path().join(format!("light{i}.fits"));
        let light: Vec<f32> = (0..64).map(|k| 100.0 + 0.7 * pattern(k) + 200.0).collect();
        v2_fixtures::write_pixels_fits(&l, 8, 8, &light);
        light_paths.push(l.to_str().unwrap().to_string());
    }

    let state = AppState::new(cfg());
    seed_session(&state, "s-pipe-dopt");

    let run = |prefix: &str, optimize: bool| {
        serde_json::json!({
            "channels": [{ "label": "L", "paths": light_paths }],
            "bias_paths": bias_paths,
            "dark_paths": dark_paths,
            "align": false,
            "normalize": false,
            "rejection": "none",
            "result_prefix": prefix,
            "dark_optimize": optimize
        })
        .to_string()
    };

    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-dopt/pipeline/run", &run("opt_", true)).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let jid = body_json(resp).await["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-pipe-dopt", &jid).await, "done");

    let resp = post_json(build_router(state.clone()), "/sessions/s-pipe-dopt/pipeline/run", &run("unit_", false)).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let jid = body_json(resp).await["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-pipe-dopt", &jid).await, "done");

    let session = state.sessions.get("s-pipe-dopt").unwrap();
    let optimized = session.cache.get("opt_L").expect("optimized slot");
    for (pos, &v) in optimized.arr().indexed_iter() {
        assert!((v - 200.0).abs() < 0.5, "pixel {:?} = {} after dark optimization", pos, v);
    }
    let unit = session.cache.get("unit_L").expect("unit-scale slot");
    let k = 1usize;
    assert!((unit.arr()[[0, k]] - (200.0 - 0.3 * pattern(k))).abs() < 0.5, "unit scale pixel {}", unit.arr()[[0, k]]);
}

#[tokio::test]
async fn v2_stats_counts_inf_pixels_as_nan_on_both_region_paths() {
    let mut arr = ndarray::Array2::from_elem((8, 8), 1.0f32);
    arr[[2, 2]] = f32::NAN;
    arr[[5, 5]] = f32::INFINITY;
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-stats-inf", "img_0", arr);

    let requests = [
        r#"{}"#,
        r#"{"region":{"type":"pixel","x":0,"y":0,"width":8,"height":8}}"#,
        r#"{"region":{"type":"shape","shape":"box","x":3.5,"y":3.5,"width":8,"height":8,"angle":0,"clip":true}}"#,
    ];
    for body in requests {
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-stats-inf/stats", body).await;
        assert_eq!(resp.status(), StatusCode::OK, "{body}");
        let json = body_json(resp).await;
        assert_eq!(json["valid_count"], 62, "{body}");
        assert_eq!(json["n_nan"], 2, "{body}");
        assert_eq!(json["nan_count"], 2, "{body}");
        assert!((json["count_fraction"].as_f64().unwrap() - 62.0 / 64.0).abs() < 1e-12, "{body}");
    }
}

#[tokio::test]
async fn v2_stats_reports_a_note_instead_of_zero_sigma_for_a_tiny_noise_region() {
    let (state, _dir) = seed_stats_session("s-stats-tiny-noise").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-tiny-noise/stats",
        r#"{"noise": true, "region": {"type":"shape","shape":"circle","x":4.5,"y":4.5,"r":2}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["noise"].is_null(), "{}", json["noise"]);
    assert_eq!(json["noise_note"], "region too small for noise evaluation (11 finite pixels, 64 needed)");

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-stats-tiny-noise/stats",
        r#"{"noise": true, "region": {"type":"pixel","x":2,"y":2,"width":4,"height":4}}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["noise"].is_null(), "{}", json["noise"]);
    assert_eq!(json["noise_note"], "region too small for noise evaluation (15 finite pixels, 64 needed)");

    let resp = post_json(build_router(state), "/v2/sessions/s-stats-tiny-noise/stats", r#"{"noise": true}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["noise"]["method"], "k-sigma-mrs");
    assert!(json["noise_note"].is_null());
}

fn path_body(p: &std::path::Path, extra: &str) -> String {
    format!(r#"{{"path":{}{extra}}}"#, serde_json::to_string(p.to_str().unwrap()).unwrap())
}

#[tokio::test]
async fn v1_fits_open_loads_the_given_path_into_a_reused_slot() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.fits");
    let b = dir.path().join("b.fits");
    v2_fixtures::write_wcs_fits(&a, 8, 8);
    v2_fixtures::write_wcs_fits(&b, 6, 6);
    let state = AppState::new(cfg());
    seed_session(&state, "s-v1-open");

    let resp = post_json(build_router(state.clone()), "/sessions/s-v1-open/fits/open", &path_body(&a, r#","slot":"main""#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["dims"], serde_json::json!([8, 8]));

    let resp = post_json(build_router(state.clone()), "/sessions/s-v1-open/fits/open", &path_body(&b, r#","slot":"main""#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["dims"], serde_json::json!([6, 6]));
    assert_eq!(json["stats"]["max"], 35.0);
    let session = state.sessions.get("s-v1-open").unwrap().clone();
    assert_eq!(session.cache.get("main").unwrap().arr().dim(), (6, 6));

    let resp = post_json(build_router(state.clone()), "/sessions/s-v1-open/fits/open", &path_body(&a, "")).await;
    assert_eq!(body_json(resp).await["dims"], serde_json::json!([8, 8]));
    v2_fixtures::write_wcs_fits(&a, 4, 4);
    let resp = post_json(build_router(state.clone()), "/sessions/s-v1-open/fits/open", &path_body(&a, "")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["dims"], serde_json::json!([4, 4]));
}

#[tokio::test]
async fn load_errors_caused_by_the_request_are_client_errors_not_500() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("one.fits");
    v2_fixtures::write_wcs_fits(&fits, 8, 8);
    let missing = dir.path().join("missing.fits");
    let state = AppState::new(cfg());
    seed_session(&state, "s-errs");

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-errs/open", &path_body(&missing, "")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"]["code"], "not_found");

    let literal = dir.path().join("one.fits#hdu=2");
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-errs/open", &path_body(&literal, "")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-errs/open", &path_body(&fits, r#","hdu":5"#)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "bad_request");

    let resp = post_json(build_router(state.clone()), "/sessions/s-errs/fits/open", &path_body(&missing, "")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = post_json(build_router(state.clone()), "/sessions/s-errs/fits/open", &path_body(&fits, r#","slot":"v""#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    for body in [
        r#"{"slot":"v","x":0,"y":0,"w":0,"h":4}"#,
        r#"{"slot":"v","x":0,"y":0,"w":4,"h":0}"#,
        r#"{"slot":"v","x":1,"y":0,"w":18446744073709551615,"h":4}"#,
        r#"{"slot":"v","x":0,"y":1,"w":4,"h":18446744073709551615}"#,
    ] {
        let resp = post_json(build_router(state.clone()), "/sessions/s-errs/image/viewport", body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let resp = post_json(build_router(state), "/sessions/s-errs/image/viewport", r#"{"slot":"v","x":6,"y":6,"w":4,"h":4}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(decode_rgb(&body_bytes(resp).await).dimensions(), (2, 2));
}

#[tokio::test]
async fn full_job_queue_reports_the_configured_limit() {
    let config = ServerConfig { jobs_max: 2, ..ServerConfig::default() };
    let state = AppState::new(Arc::new(config));
    seed_session(&state, "s-429");
    let _p1 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();
    let _p2 = Arc::clone(&state.job_semaphore).try_acquire_owned().unwrap();

    let resp = post_json(build_router(state), "/sessions/s-429/stacking/stack", r#"{"paths":["/nonexistent.fits"]}"#).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let msg = body_json(resp).await["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("max 2"), "{msg}");
}

#[tokio::test]
async fn v2_bin_turns_an_all_nan_block_into_nan_and_stats_report_it() {
    let arr = ndarray::Array2::from_shape_vec(
        (4, 4),
        vec![
            1.0, 2.0, f32::NAN, f32::NAN,
            3.0, 4.0, f32::NAN, f32::NAN,
            100.0, 200.0, f32::NAN, 9.0,
            300.0, 400.0, 9.0, 9.0,
        ],
    )
    .unwrap();
    let state = AppState::new(cfg());
    let session = seed_synthetic_image(&state, "s-bin-nan", "img_0", arr);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-bin-nan/bin", r#"{"factor":2,"ref":"img_0"}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["stats"]["valid_count"], 3);

    let out = session.cache.get("bin_0").unwrap();
    let a = out.arr();
    assert!((a[[0, 0]] - 2.5).abs() < 1e-4);
    assert!(a[[0, 1]].is_nan(), "all-NaN block must stay NaN, got {}", a[[0, 1]]);
    assert!((a[[1, 1]] - 9.0).abs() < 1e-4);

    let resp = post_json(build_router(state), "/v2/sessions/s-bin-nan/stats", r#"{"ref":"bin_0"}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["valid_count"], 3);
    assert_eq!(json["n_nan"], 1);
    assert_eq!(json["min"], 2.5);
}

#[tokio::test]
async fn v2_render_max_dim_draws_an_all_nan_block_as_nan_not_as_zero() {
    let mut arr = ndarray::Array2::from_elem((8, 8), 10.0f32);
    for y in 0..4 {
        for x in 0..4 {
            arr[[y, x]] = f32::NAN;
        }
    }
    arr[[7, 7]] = -10.0;
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-r-nan", "img_0", arr);

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-r-nan/render",
        r#"{"max_dim":2,"scale":{"algorithm":"manual","vmin":-10,"vmax":10},"colormap":"gray"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resolved_header(&resp)["binning_applied"], 4);
    let img = decode_rgb(&body_bytes(resp).await);
    assert_eq!(img.dimensions(), (2, 2));
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0]);
    assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255]);
}

#[tokio::test]
async fn open_bin_and_cutout_stats_use_the_same_rule_as_v2_stats() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("bgsub.fits");
    v2_fixtures::write_pixels_fits(&fits, 4, 2, &[-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 30.0, f32::NAN]);
    let state = AppState::new(cfg());
    seed_session(&state, "s-rule");

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rule/open", &path_body(&fits, "")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let opened = body_json(resp).await["stats"].clone();

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rule/stats", r#"{"ref":"img_0"}"#).await;
    let exact = body_json(resp).await;
    for key in ["min", "max", "median", "mad", "mean", "valid_count"] {
        assert_eq!(opened[key], exact[key], "open stats {key}");
    }
    assert_eq!(opened["valid_count"], 7);
    assert_eq!(opened["min"], -3.0);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rule/bin", r#"{"factor":1,"ref":"img_0"}"#).await;
    assert_eq!(body_json(resp).await["stats"]["valid_count"], 7);

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-rule/cutout",
        r#"{"ref":"img_0","region":{"type":"pixel","x":0,"y":0,"width":4,"height":2}}"#,
    )
    .await;
    assert_eq!(body_json(resp).await["stats"]["valid_count"], 7);

    let resp = post_json(build_router(state), "/sessions/s-rule/fits/open", &path_body(&fits, "")).await;
    let v1 = body_json(resp).await;
    assert_eq!(v1["stats"]["valid_count"], 7);
    assert_eq!(v1["stats"]["min"], -3.0);
}

#[tokio::test]
async fn v2_histogram_excludes_pixels_outside_the_range_and_rejects_an_inverted_range() {
    let state = AppState::new(cfg());
    seed_synthetic_image(&state, "s-hist-range", "img_0", hist_ramp_4x4());

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-hist-range/histogram", r#"{"bins":4,"range":[0,8]}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["bins"], serde_json::json!([1, 2, 2, 3]));

    for range in ["[10,0]", "[5,5]"] {
        let resp = post_json(
            build_router(state.clone()),
            "/v2/sessions/s-hist-range/histogram",
            &format!(r#"{{"bins":4,"range":{range}}}"#),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{range}");
    }
}

#[tokio::test]
async fn v2_stats_rejects_unbounded_sigma_clip_and_percentile_requests() {
    let (state, _dir) = seed_stats_session("s-stats-bounds").await;
    let many: Vec<f64> = (0..101).map(|i| i as f64 * 0.5).collect();
    let many = serde_json::json!({ "percentiles": many }).to_string();
    for body in [
        r#"{"sigma_clip":{"maxiters":0}}"#,
        r#"{"sigma_clip":{"maxiters":101}}"#,
        r#"{"sigma_clip":{"sigma":0}}"#,
        r#"{"sigma_clip":{"sigma":-3}}"#,
        r#"{"percentiles":[150]}"#,
        r#"{"percentiles":[-1]}"#,
        many.as_str(),
    ] {
        let resp = post_json(build_router(state.clone()), "/v2/sessions/s-stats-bounds/stats", body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-stats-bounds/stats",
        r#"{"sigma_clip":{"sigma":3,"maxiters":100},"percentiles":[0,100]}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn stack_rejects_minmax_counts_that_overflow_or_reject_every_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..3 {
        let p = dir.path().join(format!("mm{i}.fits"));
        v2_fixtures::write_pixels_fits(&p, 4, 4, &[100.0 + i as f32; 16]);
        paths.push(p.to_str().unwrap().to_string());
    }
    let state = AppState::new(cfg());
    seed_session(&state, "s-minmax");

    for (low, high) in [(u64::MAX, 1u64), (2, 1), (3, 0)] {
        let body = serde_json::json!({
            "paths": paths, "align": false, "rejection": "minmax", "minmax_low": low, "minmax_high": high
        })
        .to_string();
        let resp = post_json(build_router(state.clone()), "/sessions/s-minmax/stacking/stack", &body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "low {low} high {high}");
    }
    assert_eq!(state.job_semaphore.available_permits(), 4);

    let body = serde_json::json!({
        "paths": paths, "align": false, "rejection": "minmax", "minmax_low": 1, "minmax_high": 1, "result_slot": "mm"
    })
    .to_string();
    let resp = post_json(build_router(state.clone()), "/sessions/s-minmax/stacking/stack", &body).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let jid = body_json(resp).await["job_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_job(&state, "s-minmax", &jid).await, "done");
}

#[tokio::test]
async fn auto_generated_refs_never_replace_a_client_named_ref() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.fits");
    let b = dir.path().join("b.fits");
    let c = dir.path().join("c.fits");
    v2_fixtures::write_wcs_fits(&a, 8, 8);
    v2_fixtures::write_wcs_fits(&b, 6, 6);
    v2_fixtures::write_wcs_fits(&c, 4, 4);
    let state = AppState::new(cfg());
    seed_session(&state, "s-names");

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-names/open", &path_body(&a, r#","name":"img_1""#)).await;
    assert_eq!(body_json(resp).await["ref"], "img_1");
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-names/open", &path_body(&b, "")).await;
    assert_eq!(body_json(resp).await["ref"], "img_0");
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-names/open", &path_body(&c, "")).await;
    let third = body_json(resp).await["ref"].as_str().unwrap().to_string();
    assert_ne!(third, "img_1");
    assert_ne!(third, "img_0");

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-names/bin", r#"{"factor":2,"ref":"img_1","name":"bin_0"}"#).await;
    assert_eq!(body_json(resp).await["ref"], "bin_0");
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-names/bin", r#"{"factor":2,"ref":"img_0"}"#).await;
    assert_ne!(body_json(resp).await["ref"], "bin_0");

    let session = state.sessions.get("s-names").unwrap().clone();
    assert_eq!(session.cache.get("img_1").unwrap().arr().dim(), (8, 8));
    assert_eq!(session.cache.get("bin_0").unwrap().arr().dim(), (4, 4));
    let meta = session.v2.meta.get("img_1").unwrap().clone();
    assert_eq!(meta.source.as_deref(), a.to_str());
}

#[tokio::test]
async fn pixel_coordinates_follow_the_integer_pixel_centre_convention() {
    let (state, _dir) = seed_wcs_session("s-centre").await;

    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-centre/wcs/pix2sky",
        r#"{"points":[[-0.3,3],[7.7,3],[7.3,3],[3,-0.6]]}"#,
    )
    .await;
    let json = body_json(resp).await;
    let on: Vec<bool> = json["results"].as_array().unwrap().iter().map(|r| r["on_image"].as_bool().unwrap()).collect();
    assert_eq!(on, vec![true, false, true, false]);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-centre/pixel", r#"{"x":2.7,"y":0.2}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["x"], 3);
    assert_eq!(json["y"], 0);
    assert_eq!(json["value"], 3.0);

    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-centre/pixel", r#"{"x":-0.4,"y":0}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["value"], 0.0);

    let resp = post_json(build_router(state), "/v2/sessions/s-centre/pixel", r#"{"x":7.6,"y":0}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "pixel_out_of_bounds");
    assert!(json["error"]["message"].as_str().unwrap().contains("7.6"), "{}", json["error"]["message"]);
}

#[test]
fn concurrent_session_creates_never_exceed_session_max() {
    for _ in 0..50 {
        let config = ServerConfig { session_max: 4, ..ServerConfig::default() };
        let state = AppState::new(Arc::new(config));
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let state = state.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    state.create_session().is_some()
                })
            })
            .collect();
        let created = handles.into_iter().map(|h| h.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(state.sessions.len(), 4);
        assert_eq!(created, 4);
    }
}

#[tokio::test]
async fn v2_sky2pix_inverts_pix2sky_off_the_reference_pixel_for_a_rotated_cd() {
    let dir = tempfile::tempdir().unwrap();
    let fits = dir.path().join("rot.fits");
    v2_fixtures::write_rotated_wcs_fits(&fits, 16, 16);
    let state = AppState::new(cfg());
    seed_session(&state, "s-rot");
    let resp = post_json(build_router(state.clone()), "/v2/sessions/s-rot/open", &path_body(&fits, "")).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let points = [[7.0, 0.0], [0.0, 7.0], [12.5, 9.25], [3.0, 3.0]];
    let resp = post_json(
        build_router(state.clone()),
        "/v2/sessions/s-rot/wcs/pix2sky",
        &serde_json::json!({ "points": points }).to_string(),
    )
    .await;
    let sky = body_json(resp).await;
    let radec: Vec<[f64; 2]> = sky["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| [r["ra"].as_f64().unwrap(), r["dec"].as_f64().unwrap()])
        .collect();

    let resp = post_json(
        build_router(state),
        "/v2/sessions/s-rot/wcs/sky2pix",
        &serde_json::json!({ "points": radec }).to_string(),
    )
    .await;
    let back = body_json(resp).await;
    for (i, p) in points.iter().enumerate() {
        let x = back["results"][i]["x"].as_f64().unwrap();
        let y = back["results"][i]["y"].as_f64().unwrap();
        assert!((x - p[0]).abs() < 1e-6 && (y - p[1]).abs() < 1e-6, "point {p:?} came back as ({x}, {y})");
    }
}
