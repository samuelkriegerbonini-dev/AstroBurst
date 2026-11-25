use std::net::{IpAddr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::async_trait;
use axum::extract::{FromRequest, Request, State};
use axum::http::{header, HeaderMap, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;

use astroburst_lib::types::image_ref::{BandPlaneKey, ImageRef};

use super::codes;
use super::config::ScienceConfig;
use super::error::ScienceError;
use crate::science::state::ScienceState;
use crate::error::AppError;

const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

pub(crate) async fn guard(
    State(science): State<Arc<ScienceState>>,
    mut req: Request,
    next: Next,
) -> Response {
    if let Err(refusal) = admit(&science.cfg, req.headers(), req.uri()) {
        return refusal.into_response();
    }
    req.extensions_mut().insert(science);
    next.run(req).await
}

fn admit(cfg: &ScienceConfig, headers: &HeaderMap, uri: &Uri) -> Result<(), ScienceError> {
    if !cfg.guarded() {
        check_host(cfg, headers, uri)?;
        return check_origin(cfg, headers);
    }
    let token = cfg.token.as_ref().ok_or_else(|| {
        ScienceError::unavailable(
            codes::GUARD_UNCONFIGURED,
            "set ASTROBURST_TOKEN to serve on a non-loopback bind",
            None,
        )
    })?;
    check_host(cfg, headers, uri)?;
    check_origin(cfg, headers)?;
    match bearer(headers) {
        Some(presented) if token.matches(presented) => Ok(()),
        _ => Err(ScienceError::unauthorized(
            codes::UNAUTHORIZED,
            "this server requires an Authorization: Bearer header with its token",
            Some("send the token the server reads from ASTROBURST_TOKEN".into()),
        )),
    }
}

fn check_host(cfg: &ScienceConfig, headers: &HeaderMap, uri: &Uri) -> Result<(), ScienceError> {
    let named: Vec<Option<(String, Option<String>)>> = headers
        .get_all(header::HOST)
        .iter()
        .map(|value| value.to_str().ok().and_then(host_and_port))
        .chain(uri.host().map(|host| {
            host_name(host).map(|name| (name, uri.port_u16().map(|port| port.to_string())))
        }))
        .collect();
    let allowed = (!cfg.guarded() || !named.is_empty())
        && named.iter().all(|host| {
            host.as_ref()
                .is_some_and(|(host, port)| host_allowed(cfg, host, port.as_deref()))
        });
    if allowed {
        return Ok(());
    }
    Err(ScienceError::misdirected(
        codes::HOST_NOT_ALLOWED,
        "the Host of this request is not one this server answers to",
        Some("add the host name clients use to ASTROBURST_ALLOWED_HOSTS".into()),
    ))
}

fn check_origin(cfg: &ScienceConfig, headers: &HeaderMap) -> Result<(), ScienceError> {
    let allowed = headers.get_all(header::ORIGIN).iter().all(|value| {
        value
            .to_str()
            .ok()
            .and_then(origin_host)
            .is_some_and(|(host, port)| host_allowed(cfg, &host, Some(&port)))
    });
    if allowed {
        return Ok(());
    }
    Err(ScienceError::forbidden(
        codes::ORIGIN_NOT_ALLOWED,
        "the Origin of this request is not a page this server answers to",
        Some("call the server from a local client, or add the host of that page to ASTROBURST_ALLOWED_HOSTS".into()),
    ))
}

fn origin_host(origin: &str) -> Option<(String, String)> {
    let (scheme, authority) = origin.split_once("://")?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => "80",
        "https" => "443",
        _ => return None,
    };
    let (host, port) = host_and_port(authority)?;
    Some((host, port.unwrap_or_else(|| default_port.to_owned())))
}

fn host_allowed(cfg: &ScienceConfig, host: &str, port: Option<&str>) -> bool {
    let local = LOCAL_HOSTS.contains(&host) || bind_host(cfg.bind.ip()).as_deref() == Some(host);
    (local && (cfg.guarded() || on_the_bound_port(cfg, port)))
        || cfg.allowed_hosts.iter().any(|allowed| allowed == host)
}

fn on_the_bound_port(cfg: &ScienceConfig, port: Option<&str>) -> bool {
    let bound = cfg.bind.port();
    bound == 0 || port.is_none_or(|port| port == bound.to_string())
}

fn host_and_port(authority: &str) -> Option<(String, Option<String>)> {
    let host = host_name(authority)?;
    let rest = match authority.strip_prefix('[') {
        Some(inner) => &inner[inner.find(']')? + 1..],
        None => &authority[authority.find(':').unwrap_or(authority.len())..],
    };
    Some((host, rest.strip_prefix(':').map(str::to_owned)))
}

fn bind_host(ip: IpAddr) -> Option<String> {
    match ip {
        ip if ip.is_unspecified() => None,
        IpAddr::V4(v4) => Some(v4.to_string()),
        IpAddr::V6(v6) => Some(format!("[{v6}]")),
    }
}

fn bearer(headers: &HeaderMap) -> Option<&[u8]> {
    let value = headers.get(header::AUTHORIZATION)?.as_bytes();
    let (scheme, token) = value.split_at(value.iter().position(|b| *b == b' ')?);
    scheme
        .eq_ignore_ascii_case(b"bearer")
        .then(|| token.trim_ascii())
}

pub(crate) fn host_name(authority: &str) -> Option<String> {
    let (host, rest) = match authority.strip_prefix('[') {
        Some(inner) => {
            let close = inner.find(']')?;
            let ip: Ipv6Addr = inner[..close].parse().ok()?;
            (format!("[{ip}]"), &inner[close + 1..])
        }
        None => {
            let (name, rest) = authority.split_at(authority.find(':').unwrap_or(authority.len()));
            let name = name.strip_suffix('.').unwrap_or(name);
            let valid = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'));
            (valid.then(|| name.to_ascii_lowercase())?, rest)
        }
    };
    let port_ok = rest.is_empty()
        || rest
            .strip_prefix(':')
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()));
    port_ok.then_some(host)
}

pub(crate) fn resolve_under_roots(
    roots: &[PathBuf],
    user_path: &str,
) -> Result<PathBuf, ScienceError> {
    let opened_inside = files_the_loader_opens(user_path)
        .iter()
        .all(|file| resolved_inside(roots, file).is_some());
    match resolved_inside(roots, user_path) {
        Some(full) if opened_inside => Ok(full),
        _ => Err(ScienceError::forbidden(
            codes::PATH_OUTSIDE_ROOT,
            format!("path {user_path} is not inside a folder this server may read"),
            Some("place the file under ASTROBURST_SERVER_ROOT, or start the server with ASTROBURST_SERVER_ROOT set to a folder that holds it".into()),
        )),
    }
}

fn resolved_inside(roots: &[PathBuf], user_path: &str) -> Option<PathBuf> {
    let path = Path::new(user_path);
    if names_a_device_or_foreign_share(roots, path) {
        return None;
    }
    resolved_on_disk(path).filter(|full| roots.iter().any(|root| full.starts_with(root)))
}

fn files_the_loader_opens(user_path: &str) -> Vec<String> {
    let plane = Some(ImageRef::parse(user_path).path);
    let band = BandPlaneKey::parse(user_path).map(|key| key.file);
    [plane, band]
        .into_iter()
        .flatten()
        .filter(|file| file != user_path)
        .collect()
}

pub(crate) async fn confine(cfg: &ScienceConfig, paths: Vec<String>) -> Result<(), ScienceError> {
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || {
        let roots = cfg.roots();
        paths
            .iter()
            .try_for_each(|path| resolve_under_roots(&roots, path).map(drop))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
}

fn resolved_on_disk(path: &Path) -> Option<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(full) => Some(full),
        Err(_) if std::fs::symlink_metadata(path).is_err() => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            path.file_name()
                .and_then(|name| std::fs::canonicalize(parent).ok().map(|dir| dir.join(name)))
        }
        Err(_) => None,
    }
}

#[cfg(windows)]
pub(crate) fn names_a_device_or_foreign_share(roots: &[PathBuf], path: &Path) -> bool {
    use std::path::{Component, Prefix};

    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
            _ => !share_of(path).is_some_and(|share| {
                roots
                    .iter()
                    .any(|root| share_of(root).as_ref() == Some(&share))
            }),
        },
        _ => {
            let text = path.as_os_str().as_encoded_bytes();
            let separator = |byte: Option<&u8>| matches!(byte, Some(b'\\' | b'/'));
            (separator(text.first()) && separator(text.get(1))) || text.starts_with(br"\??\")
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn names_a_device_or_foreign_share(_: &[PathBuf], _: &Path) -> bool {
    false
}

#[cfg(windows)]
fn share_of(path: &Path) -> Option<(std::ffi::OsString, std::ffi::OsString)> {
    use std::path::{Component, Prefix};

    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                Some((server.to_ascii_lowercase(), share.to_ascii_lowercase()))
            }
            _ => None,
        },
        _ => None,
    }
}

pub(crate) trait RequestPaths {
    fn request_paths(&self) -> Vec<&str>;
}

impl RequestPaths for crate::handlers::io::OpenParams {
    fn request_paths(&self) -> Vec<&str> {
        vec![self.path.as_str()]
    }
}

impl RequestPaths for crate::v2::images::OpenParams {
    fn request_paths(&self) -> Vec<&str> {
        vec![self.path.as_str()]
    }
}

impl RequestPaths for crate::handlers::stacking::StackParams {
    fn request_paths(&self) -> Vec<&str> {
        self.paths.iter().map(String::as_str).collect()
    }
}

impl RequestPaths for crate::handlers::stacking::DrizzleParams {
    fn request_paths(&self) -> Vec<&str> {
        self.paths.iter().map(String::as_str).collect()
    }
}

impl RequestPaths for crate::handlers::pipeline::PipelineParams {
    fn request_paths(&self) -> Vec<&str> {
        let calibration = [
            &self.bias_paths,
            &self.dark_paths,
            &self.flat_paths,
            &self.flat_dark_paths,
        ];
        self.channels
            .iter()
            .flat_map(|channel| &channel.paths)
            .chain(calibration.into_iter().flatten().flatten())
            .map(String::as_str)
            .collect()
    }
}

pub(crate) struct ConfinedJson<T>(pub(crate) T);

#[async_trait]
impl<S, T> FromRequest<S> for ConfinedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + RequestPaths + Send,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let science = req.extensions().get::<Arc<ScienceState>>().cloned();
        let Json(params) = Json::<T>::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let science = science.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "the path guard is not installed on this router"
            ))
            .into_response()
        })?;
        if science.cfg.guarded() {
            let paths = params
                .request_paths()
                .into_iter()
                .map(str::to_owned)
                .collect();
            confine(&science.cfg, paths)
                .await
                .map_err(IntoResponse::into_response)?;
        }
        Ok(Self(params))
    }
}
