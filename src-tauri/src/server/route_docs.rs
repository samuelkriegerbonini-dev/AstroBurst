use std::sync::Arc;

use astroburst_lib::core::stacking::drizzle::drizzle_stack;
use astroburst_lib::types::stacking::DrizzleConfig;
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use ndarray::Array2;
use tower::ServiceExt;

use crate::config::ServerConfig;
use crate::job::new_job;
use crate::router::build_router;
use crate::session::Session;
use crate::state::AppState;

const ROUTER_SOURCE: &str = include_str!("router.rs");
const ROUTE_REFERENCE: &str = include_str!("../../../docs/server-api.md");

const ROUTING_METHODS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];
const UNREADABLE_REGISTRATIONS: [&str; 5] = [
    ".nest(",
    ".nest_service(",
    ".merge(",
    ".route_service(",
    ".fallback_service(",
];
const ROUTE_COUNT_PHRASE: &str = " method and path pairs";

const AXUM_DEFAULT_BODY_LIMIT: usize = 2_097_152;
const DRIZZLE_SCALE_RANGE: (f64, f64) = (1.0, 4.0);
const DRIZZLE_PIXFRAC_RANGE: (f64, f64) = (0.1, 1.0);
const DRIZZLE_SIZE_TOLERANCE_PERCENT: usize = 5;

fn calls<'a>(source: &'a str, opener: &str) -> Vec<&'a str> {
    source
        .split(opener)
        .skip(1)
        .map(|rest| {
            let mut depth = 1usize;
            let end = rest
                .char_indices()
                .find_map(|(i, c)| {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    (depth == 0).then_some(i)
                })
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .collect()
}

fn quoted_path(call: &str) -> Option<&str> {
    let start = call.find('"')? + 1;
    let len = call[start..].find('"')?;
    Some(&call[start..start + len])
}

fn called_methods(call: &str) -> Vec<String> {
    call.match_indices('(')
        .filter_map(|(open, _)| {
            let before = &call[..open];
            let start = before
                .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_')
                .len();
            let ident = &before[start..];
            ROUTING_METHODS
                .contains(&ident)
                .then(|| ident.to_ascii_uppercase())
        })
        .collect()
}

fn indented_at_most_three(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() <= 3).then_some(rest)
}

fn fence(line: &str) -> Option<(char, usize, bool)> {
    let rest = indented_at_most_three(line)?;
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let after = rest.trim_start_matches(marker);
    let len = rest.len() - after.len();
    (len >= 3).then(|| (marker, len, after.trim().is_empty()))
}

fn outside_fences(markdown: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut open: Option<(char, usize)> = None;
    markdown.lines().enumerate().filter(move |(_, line)| {
        match (open, fence(line)) {
            (None, Some((marker, len, _))) => open = Some((marker, len)),
            (Some((marker, len)), Some((other, other_len, true)))
                if other == marker && other_len >= len =>
            {
                open = None
            }
            (None, None) => return true,
            _ => {}
        }
        false
    })
}

fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let rest = indented_at_most_three(line)?;
    let text = rest.trim_start_matches('#');
    let level = rest.len() - text.len();
    ((1..=6).contains(&level) && (text.is_empty() || text.starts_with([' ', '\t'])))
        .then(|| (level, text.trim()))
}

fn headings(markdown: &str) -> Vec<(usize, usize, &str)> {
    outside_fences(markdown)
        .filter_map(|(index, line)| atx_heading(line).map(|(level, text)| (index, level, text)))
        .collect()
}

fn index_rows(markdown: &str) -> Vec<String> {
    outside_fences(markdown)
        .filter(|(_, line)| line.trim_start().starts_with('|'))
        .filter_map(|(_, line)| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            cells.windows(2).find_map(|cell| {
                let path = cell[1].split('`').nth(1)?;
                route_pair(&format!("{} {path}", cell[0]))
            })
        })
        .collect()
}

fn stated_route_count(markdown: &str) -> Option<usize> {
    outside_fences(markdown).find_map(|(_, line)| {
        let before = &line[..line.find(ROUTE_COUNT_PHRASE)?];
        before
            .rsplit(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    })
}

fn section(markdown: &str, title: &str) -> String {
    let all = headings(markdown);
    let (start, level) = all
        .iter()
        .find(|(_, _, text)| *text == title)
        .map(|(index, level, _)| (*index, *level))
        .unwrap_or_else(|| panic!("docs/server-api.md has no heading `{title}`"));
    let end = all
        .iter()
        .find(|(index, other, _)| *index > start && *other <= level)
        .map_or(usize::MAX, |(index, _, _)| *index);
    markdown
        .lines()
        .enumerate()
        .filter(|(index, _)| (start + 1..end).contains(index))
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn route_pair(text: &str) -> Option<String> {
    let (method, path) = text.split_once(' ')?;
    let known = ROUTING_METHODS
        .iter()
        .any(|name| name.to_ascii_uppercase() == method);
    (known && path.starts_with('/') && !path.contains(char::is_whitespace))
        .then(|| format!("{method} {path}"))
}

fn route_reference_problems(router: &str, reference: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut registered = Vec::new();
    for call in calls(router, ".route(") {
        let methods = called_methods(call);
        match quoted_path(call) {
            Some(path) if !methods.is_empty() => {
                registered.extend(methods.into_iter().map(|method| format!("{method} {path}")))
            }
            _ => problems.push(format!("unreadable registration .route({})", call.trim())),
        }
    }
    for opener in UNREADABLE_REGISTRATIONS {
        problems.extend(
            calls(router, opener)
                .into_iter()
                .map(|call| format!("unreadable registration {opener}{})", call.trim())),
        );
    }
    let titled: Vec<String> = headings(reference)
        .into_iter()
        .filter_map(|(_, _, text)| route_pair(text))
        .collect();
    let indexed = index_rows(reference);
    for (kind, documented) in [("heading", &titled), ("index row", &indexed)] {
        problems.extend(
            registered
                .iter()
                .filter(|pair| !documented.contains(pair))
                .map(|pair| format!("no {kind}: {pair}")),
        );
        problems.extend(
            documented
                .iter()
                .filter(|pair| !registered.contains(pair))
                .map(|pair| format!("{kind} for an unregistered route: {pair}")),
        );
    }
    match stated_route_count(reference) {
        Some(count) if count == registered.len() => {}
        Some(count) => problems.push(format!(
            "the route index says {count}{ROUTE_COUNT_PHRASE}, the router registers {}",
            registered.len()
        )),
        None => problems.push(format!(
            "the route index states no `N{ROUTE_COUNT_PHRASE}` count"
        )),
    }
    problems
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn router_with_session(sid: &str) -> (Router, Arc<Session>) {
    let state = AppState::new(Arc::new(ServerConfig::default()));
    let session = Session::new(sid.into(), &ServerConfig::default());
    state.sessions.insert(sid.into(), Arc::clone(&session));
    (build_router(state), session)
}

async fn send(app: &Router, method: Method, uri: &str, body: String) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if !body.is_empty() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    app.clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

fn content_type(response: &Response) -> &str {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

fn padded_points_body(len: usize) -> String {
    let (open, close) = (r#"{"points":[],"pad":""#, r#""}"#);
    format!(
        "{open}{}{close}",
        "x".repeat(len - open.len() - close.len())
    )
}

fn textured_frame(rows: usize, cols: usize, seed: usize) -> Array2<f32> {
    Array2::from_shape_fn((rows, cols), |(r, c)| {
        ((r * 31 + c * 17 + seed * 7) % 23) as f32
    })
}

fn drizzled(
    frames: &[Array2<f32>],
    scale: f64,
    pixfrac: f64,
) -> Option<((usize, usize), Vec<u32>)> {
    let config = DrizzleConfig {
        scale,
        pixfrac,
        align: false,
        ..DrizzleConfig::default()
    };
    drizzle_stack(frames, &config).ok().map(|result| {
        let bits = result.image.iter().map(|v| v.to_bits()).collect();
        (result.output_dims, bits)
    })
}

#[test]
fn the_route_reference_matches_the_router() {
    let problems = route_reference_problems(ROUTER_SOURCE, ROUTE_REFERENCE);
    assert!(
        problems.is_empty(),
        "docs/server-api.md is out of step with src-tauri/src/server/router.rs ({} problem(s)):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn every_method_of_a_registration_is_read_and_each_missing_heading_is_named() {
    let router = r#"Router::new()
        .route("/a", get(health))
        .route(
            "/b/:id",
            routing::get(b::show).delete(b::remove),
        )
        .route("/c", routing::post(c::create))"#;
    let reference = "# Title\n\n4 method and path pairs.\n\n| 1 | GET | [`/a`](#get-a) |\n| 2 | GET | `/b/:id` |\n| 3 | DELETE | `/b/:id` |\n| 4 | POST | `/c` |\n\n#### GET /a\n\n```\n# DELETE /b/:id\n```\n\n### POST /c\n";
    assert_eq!(
        route_reference_problems(router, reference),
        ["no heading: GET /b/:id", "no heading: DELETE /b/:id"]
    );
}

#[test]
fn stale_headings_stale_index_rows_and_a_wrong_count_are_named() {
    let router = r#".route("/a", get(a)).route("/c", routing::post(c))"#;
    let reference = "3 method and path pairs.\n\n| 1 | GET | `/a` |\n| 2 | POST | `/c` |\n| 3 | GET | `/gone` |\n\n### GET /a\n\n### POST /c?x=\n\n### DELETE /gone\n";
    assert_eq!(
        route_reference_problems(router, reference),
        [
            "no heading: POST /c",
            "heading for an unregistered route: POST /c?x=",
            "heading for an unregistered route: DELETE /gone",
            "index row for an unregistered route: GET /gone",
            "the route index says 3 method and path pairs, the router registers 2",
        ]
    );
}

#[test]
fn nested_merged_and_service_registrations_are_reported_as_unreadable() {
    let router = r#"Router::new()
        .route("/a", get(a))
        .nest("/v2/science", science::router())
        .merge(other())
        .route_service("/files", ServeDir::new("x"))
        .fallback_service(fallback())"#;
    let reference = "1 method and path pairs.\n\n| 1 | GET | `/a` |\n\n### GET /a\n";
    assert_eq!(
        route_reference_problems(router, reference),
        [
            r#"unreadable registration .nest("/v2/science", science::router())"#,
            "unreadable registration .merge(other())",
            r#"unreadable registration .route_service("/files", ServeDir::new("x"))"#,
            "unreadable registration .fallback_service(fallback())",
        ]
    );
}

#[test]
fn only_lines_markdown_renders_as_headings_document_a_route() {
    let router = r#".route("/a", get(a)).route("/b", get(b))"#;
    let reference = "2 method and path pairs.\n\n| 1 | GET | `/a` |\n| 2 | GET | `/b` |\n\n   ### GET /b\n\n    # GET /a\n\n#GET /a\n\n~~~\n### GET /a\n~~~\n\n```text\n~~~\n### GET /a\n```\n";
    assert_eq!(
        route_reference_problems(router, reference),
        ["no heading: GET /a"]
    );
}

#[test]
fn the_drizzle_reference_states_the_scale_pixfrac_and_frame_sizes_the_engine_runs_with() {
    let frames = [textured_frame(12, 16, 0), textured_frame(12, 16, 1)];
    let run = |scale, pixfrac| drizzled(&frames, scale, pixfrac).expect("two equal frames drizzle");
    let (min_scale, max_scale) = DRIZZLE_SCALE_RANGE;
    let (min_pixfrac, max_pixfrac) = DRIZZLE_PIXFRAC_RANGE;
    assert_eq!(
        run(max_scale, 0.7).0,
        (12 * max_scale as usize, 16 * max_scale as usize)
    );
    assert!(
        run(max_scale * 2.0, 0.7) == run(max_scale, 0.7),
        "scale above {max_scale} is not clamped"
    );
    assert!(
        run(min_scale / 2.0, 0.7) == run(min_scale, 0.7),
        "scale below {min_scale} is not clamped"
    );
    let uneven_scale = 1.3;
    assert!(
        run(uneven_scale, min_pixfrac / 10.0) == run(uneven_scale, min_pixfrac),
        "pixfrac below {min_pixfrac} is not clamped"
    );
    assert!(
        run(uneven_scale, max_pixfrac * 3.0) == run(uneven_scale, max_pixfrac),
        "pixfrac above {max_pixfrac} is not clamped"
    );
    assert!(
        run(uneven_scale, min_pixfrac) != run(uneven_scale, max_pixfrac),
        "pixfrac has no effect on these frames"
    );

    let smallest = (30, 40);
    let tolerance = smallest.1 * DRIZZLE_SIZE_TOLERANCE_PERCENT / 100;
    let within = [
        textured_frame(30, 40, 0),
        textured_frame(30 + tolerance, 40 + tolerance, 1),
    ];
    assert_eq!(
        drizzled(&within, 1.0, 1.0).map(|(dims, _)| dims),
        Some(smallest)
    );
    let beyond = [
        textured_frame(30, 40, 0),
        textured_frame(30, 40 + tolerance + 1, 1),
    ];
    assert!(
        drizzled(&beyond, 1.0, 1.0).is_none(),
        "a frame {} px wider is accepted",
        tolerance + 1
    );

    let drizzle = section(ROUTE_REFERENCE, "POST /sessions/:sid/stacking/drizzle");
    for claim in [
        format!("`scale` clamped to {min_scale} to {max_scale}"),
        format!("`pixfrac` clamped to {min_pixfrac} to {max_pixfrac}"),
        format!("{DRIZZLE_SIZE_TOLERANCE_PERCENT} %"),
    ] {
        assert!(
            drizzle.contains(&claim),
            "the drizzle section of docs/server-api.md does not state {claim:?}"
        );
    }
}

#[tokio::test]
async fn the_error_reference_states_the_request_body_limit() {
    let (app, _session) = router_with_session("sid-body-limit");
    let uri = "/v2/sessions/sid-body-limit/wcs/pix2sky";

    let at_limit = send(
        &app,
        Method::POST,
        uri,
        padded_points_body(AXUM_DEFAULT_BODY_LIMIT),
    )
    .await;
    assert_eq!(at_limit.status(), StatusCode::BAD_REQUEST);
    assert_eq!(content_type(&at_limit), "application/json");

    let over_limit = send(
        &app,
        Method::POST,
        uri,
        padded_points_body(AXUM_DEFAULT_BODY_LIMIT + 1),
    )
    .await;
    assert_eq!(over_limit.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        content_type(&over_limit).starts_with("text/plain"),
        "{}",
        content_type(&over_limit)
    );

    let limit = format!("{} bytes", thousands(AXUM_DEFAULT_BODY_LIMIT));
    let errors = section(ROUTE_REFERENCE, "7. Errors");
    assert!(
        errors.contains("`413`") && errors.contains(&limit),
        "the errors section of docs/server-api.md does not list the 413 for a body over {limit}"
    );
    let coordinates = section(ROUTE_REFERENCE, "11.4 Coordinates");
    assert!(
        coordinates.contains("2 MiB"),
        "the coordinates section of docs/server-api.md does not say that a `points` list must fit in 2 MiB"
    );
}

#[tokio::test]
async fn the_stream_reference_warns_that_head_takes_the_one_subscription() {
    let (app, session) = router_with_session("sid-stream-head");
    let job = new_job("stack");
    session.jobs.insert(job.id.clone(), Arc::clone(&job));
    let uri = format!("/sessions/sid-stream-head/jobs/{}/stream", job.id);

    let head = send(&app, Method::HEAD, &uri, String::new()).await;
    assert_eq!(head.status(), StatusCode::OK);
    let get = send(&app, Method::GET, &uri, String::new()).await;
    assert_eq!(get.status(), StatusCode::CONFLICT);

    let stream = section(ROUTE_REFERENCE, "GET /sessions/:sid/jobs/:jid/stream");
    assert!(
        stream.contains("`HEAD`"),
        "the stream section of docs/server-api.md does not warn that HEAD takes the one subscription"
    );
}
