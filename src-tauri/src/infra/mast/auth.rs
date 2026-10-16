use std::future::Future;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::api::MissionEndpoint;
use super::client::{AuthTransport, MastClient, RedirectRules};
use super::hub::{needs_token, MastHub};
use super::secret::SecretToken;
use super::settings::MastSettings;
use crate::core::mast::errors::{bad_input, classify, token_rejected, unexpected_format, OpKind, Operation, NETWORK_MESSAGE_LIMIT};
use crate::core::mast::types::{MastCmdError, MastErrorCode, MastMissionId, MastTokenStatus};
use crate::infra::config;

pub const TOKEN_SERVICE: &str = "mast";
pub const TOKEN_MIN_CHARS: usize = 16;
pub const TOKEN_MAX_CHARS: usize = 512;
pub const INFO_PATH: &str = "info";
pub const INFO_JSON_CAP: u64 = 64 * 1024;
pub const FORMAT_REASON: &str = "must be 16 to 512 printable ASCII characters without spaces";
pub const ABSENT: MastTokenStatus = MastTokenStatus { present: false, authorized: None };

pub fn check_format(raw: &str) -> Result<SecretToken, MastCmdError> {
    let trimmed = raw.trim();
    let printable = trimmed.bytes().all(|byte| (0x21..=0x7E).contains(&byte));
    let sized = (TOKEN_MIN_CHARS..=TOKEN_MAX_CHARS).contains(&trimmed.len());
    match SecretToken::new(trimmed) {
        Some(token) if printable && sized => Ok(token),
        _ => Err(bad_input("token", FORMAT_REASON)),
    }
}

fn storage_failure(error: anyhow::Error) -> MastCmdError {
    let cause: String = error.to_string().chars().take(NETWORK_MESSAGE_LIMIT).collect();
    MastCmdError::new(MastErrorCode::Cache, format!("token storage failed: {cause}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStore {
    dir: PathBuf,
}

impl TokenStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn system() -> anyhow::Result<Self> {
        config::config_dir().map(Self::new)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self) -> Result<Option<SecretToken>, MastCmdError> {
        let stored = config::load_api_key_in(&self.dir, TOKEN_SERVICE).map_err(storage_failure)?;
        Ok(stored.and_then(|raw| check_format(&raw).ok()))
    }

    pub fn save(&self, token: &SecretToken) -> Result<(), MastCmdError> {
        config::save_api_key_in(&self.dir, token.expose(), TOKEN_SERVICE).map_err(storage_failure)
    }

    pub fn delete(&self) -> Result<bool, MastCmdError> {
        config::delete_api_key_in(&self.dir, TOKEN_SERVICE).map_err(storage_failure)
    }

    pub fn transport(&self) -> Result<Option<AuthTransport>, MastCmdError> {
        let stored = config::mast_token_transport_in(&self.dir).map_err(storage_failure)?;
        Ok(stored.as_deref().and_then(AuthTransport::parse))
    }

    pub fn set_transport(&self, transport: Option<AuthTransport>) -> Result<(), MastCmdError> {
        if self.transport()? == transport {
            return Ok(());
        }
        config::set_mast_token_transport_in(&self.dir, transport.map(AuthTransport::as_str)).map_err(storage_failure)?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AuthSession {
    token: Option<SecretToken>,
    roman: Option<MastClient>,
    authorized: Option<bool>,
    generation: u64,
}

impl AuthSession {
    pub fn roman(&self) -> Option<&MastClient> {
        self.roman.as_ref()
    }

    pub fn set_roman(&mut self, client: Option<MastClient>) {
        self.roman = client;
    }

    pub fn authorized(&self) -> Option<bool> {
        self.authorized
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn holds(&self, token: &SecretToken) -> bool {
        self.roman.is_some() && self.token.as_ref() == Some(token)
    }

    fn install(&mut self, token: SecretToken, roman: MastClient) {
        *self = Self { token: Some(token), roman: Some(roman), authorized: None, generation: self.generation + 1 };
    }

    fn confirm(&mut self, roman: MastClient) {
        self.roman = Some(roman);
        self.authorized = Some(true);
    }

    fn deny(&mut self) {
        self.authorized = Some(false);
    }

    fn forget(&mut self) {
        *self = Self { generation: self.generation + 1, ..Self::default() };
    }

    fn demote(&mut self, generation: u64) -> Option<SecretToken> {
        if self.generation != generation || self.authorized != Some(true) {
            return None;
        }
        self.authorized = Some(false);
        self.token.clone()
    }
}

#[derive(Deserialize)]
struct InfoAnswer {
    anon: bool,
}

fn info_operation() -> Operation {
    Operation::new(None, OpKind::Info, INFO_PATH)
}

fn auth_check_operation() -> Operation {
    Operation::mission(MastMissionId::Roman, OpKind::AuthCheck, "auth_check")
}

fn info_client(settings: &MastSettings, token: &SecretToken) -> Result<MastClient, MastCmdError> {
    MastClient::builder()
        .base_url(&settings.auth_url)
        .timeout(settings.timeout(OpKind::Info))
        .secret(token.clone())
        .transport(AuthTransport::Header)
        .redirects(RedirectRules::Never)
        .json_cap(INFO_JSON_CAP)
        .redact_error_bodies()
        .build()
        .map_err(|error| classify(&error, &info_operation(), 0))
}

pub fn roman_client(settings: &MastSettings, token: &SecretToken, transport: AuthTransport) -> Result<MastClient, MastCmdError> {
    MastClient::builder()
        .base_url(&settings.base_url)
        .secret(token.clone())
        .transport(transport)
        .redirects(RedirectRules::Never)
        .build()
        .map_err(|error| classify(&error, &auth_check_operation(), 0))
}

async fn verify(settings: &MastSettings, token: &SecretToken) -> Result<(), MastCmdError> {
    let op = info_operation();
    let answer = info_client(settings, token)?.get_json::<Option<InfoAnswer>>(INFO_PATH, &[]).await;
    match answer {
        Ok(Some(InfoAnswer { anon: false })) => Ok(()),
        Ok(Some(InfoAnswer { anon: true })) => Err(token_rejected()),
        Ok(None) => Err(unexpected_format(&op)),
        Err(error) => Err(classify(&error, &op, 0)),
    }
}

const fn alternate(transport: AuthTransport) -> AuthTransport {
    match transport {
        AuthTransport::Header => AuthTransport::Cookie,
        AuthTransport::Cookie => AuthTransport::Header,
    }
}

async fn authorize(hub: &MastHub, store: &TokenStore, token: &SecretToken, first: AuthTransport) -> Result<bool, MastCmdError> {
    let op = auth_check_operation();
    for transport in [first, alternate(first)] {
        let client = roman_client(hub.settings(), token, transport)?;
        let endpoint = MissionEndpoint::new(client.clone().with_timeout(hub.settings().timeout(OpKind::AuthCheck)), MastMissionId::Roman);
        let answer = endpoint.auth_check().await.map_err(|error| classify(&error, &op, 0))?;
        if answer.authorized {
            store.set_transport(Some(transport))?;
            hub.auth_session().confirm(client);
            return Ok(true);
        }
    }
    hub.auth_session().deny();
    Ok(false)
}

fn install(hub: &MastHub, token: &SecretToken, transport: AuthTransport) -> Result<(), MastCmdError> {
    let roman = roman_client(hub.settings(), token, transport)?;
    hub.auth_session().install(token.clone(), roman);
    Ok(())
}

fn restore(hub: &MastHub, store: &TokenStore) -> Result<Option<SecretToken>, MastCmdError> {
    let Some(token) = store.load()? else {
        hub.auth_session().forget();
        return Ok(None);
    };
    if !hub.auth_session().holds(&token) {
        install(hub, &token, store.transport()?.unwrap_or_default())?;
    }
    Ok(Some(token))
}

const fn present(authorized: Option<bool>) -> MastTokenStatus {
    MastTokenStatus { present: true, authorized }
}

pub async fn stored_status(hub: &MastHub) -> Result<MastTokenStatus, MastCmdError> {
    let store = hub.token_store()?;
    let _serial = hub.auth_serial().lock().await;
    if restore(hub, store)?.is_none() {
        return Ok(ABSENT);
    }
    let authorized = hub.auth_session().authorized();
    Ok(present(authorized))
}

pub async fn check(hub: &MastHub) -> Result<MastTokenStatus, MastCmdError> {
    let store = hub.token_store()?;
    let _serial = hub.auth_serial().lock().await;
    let Some(token) = restore(hub, store)? else {
        return Ok(ABSENT);
    };
    if let Err(error) = verify(hub.settings(), &token).await {
        if error.code == MastErrorCode::TokenRejected {
            hub.auth_session().deny();
        }
        return Err(error);
    }
    let first = store.transport()?.unwrap_or_default();
    let authorized = authorize(hub, store, &token, first).await?;
    Ok(present(Some(authorized)))
}

pub async fn save(hub: &MastHub, raw: &str) -> Result<MastTokenStatus, MastCmdError> {
    let token = check_format(raw)?;
    let store = hub.token_store()?;
    let _serial = hub.auth_serial().lock().await;
    verify(hub.settings(), &token).await?;
    store.save(&token)?;
    install(hub, &token, AuthTransport::Header)?;
    store.set_transport(None)?;
    let authorized = authorize(hub, store, &token, AuthTransport::Header).await?;
    Ok(present(Some(authorized)))
}

pub async fn clear(hub: &MastHub) -> Result<MastTokenStatus, MastCmdError> {
    let store = hub.token_store()?;
    let _serial = hub.auth_serial().lock().await;
    store.delete()?;
    hub.auth_session().forget();
    store.set_transport(None)?;
    Ok(ABSENT)
}

fn remote_unauthorized(error: &MastCmdError) -> bool {
    error.code == MastErrorCode::Unauthorized && error.context.status == Some(401) && error.context.mission.is_some_and(needs_token)
}

async fn recheck(hub: &MastHub, generation: u64, unauthorized: MastCmdError) -> MastCmdError {
    let demoted = hub.auth_session().demote(generation);
    let Some(token) = demoted else {
        return unauthorized;
    };
    match verify(hub.settings(), &token).await {
        Err(rejected) if rejected.code == MastErrorCode::TokenRejected => rejected,
        _ => unauthorized,
    }
}

pub async fn screen_unauthorized<T>(hub: &MastHub, work: impl Future<Output = Result<T, MastCmdError>>) -> Result<T, MastCmdError> {
    let generation = hub.auth_session().generation();
    match work.await {
        Err(error) if remote_unauthorized(&error) => Err(recheck(hub, generation, error).await),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::query::PAGE_LIMIT;
    use crate::core::mast::types::{
        MastErrorContext, MastFilterValues, MastLevelClass, MastProgressEvent, MastQuery, MastWhere, PreviewSize, ProgressStage,
    };
    use crate::infra::mast::service::{image_list, preview, products, resolve, search, thumbnail, token_clear, token_save, token_status};
    use crate::infra::mast::settings::{ENV_MAST_AUTH_URL, ENV_MAST_TIMEOUT_MS, ENV_MAST_URL};
    use crate::infra::mast::testing::{serve_routes, synthetic_jpeg, Recorded, Route, TestServer, MARKER_TOKEN};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    const SAVE_ID: &str = "token:save";
    const CHECK_ID: &str = "token:check";
    const INFO: &str = "/info";
    const AUTH_CHECK: &str = "/search/roman/api/v0.1/auth_check";
    const ROMAN_PREFIX: &str = "/search/roman/";
    const ROMAN_SEARCH: &str = "/search/roman/api/v0.1/search";
    const JWST_SEARCH: &str = "/search/jwst/api/v0.1/search";
    const HST_THUMBNAIL: &str = "/search/hst/api/v0.1/get_image_thumbnail";
    const LOOKUP: &str = "/api/v0/invoke";
    const EMPTY_SEARCH: &str = r#"{"totalResults":0,"results":[],"info":[]}"#;
    const ROMAN_401: &str = r#"{"detail":"Only authorized users can make search API calls"}"#;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    fn info_body(anon: bool) -> String {
        format!(r#"{{"eppn":"","ezid":"anonymous","attrib":{{}},"anon":{anon},"scopes":[],"session":null,"token":null}}"#)
    }

    fn info_route(anon: bool) -> Route {
        Route::get(INFO).json(info_body(anon))
    }

    fn auth_check_route(authorized: bool) -> Route {
        Route::get(AUTH_CHECK).json(format!(r#"{{"authorized":{authorized}}}"#))
    }

    fn cookie_value() -> String {
        format!("mast_token={MARKER_TOKEN}")
    }

    fn bearer() -> String {
        format!("token {MARKER_TOKEN}")
    }

    fn cookie_only_auth_check() -> Vec<Route> {
        let expected = cookie_value();
        vec![
            Route::get(AUTH_CHECK).when(move |r| r.header("cookie") == Some(expected.as_str())).json(r#"{"authorized":true}"#),
            auth_check_route(false),
        ]
    }

    fn present(authorized: Option<bool>) -> MastTokenStatus {
        MastTokenStatus { present: true, authorized }
    }

    fn m16(mission: MastMissionId) -> MastQuery {
        MastQuery {
            mission,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: Some("M 16".into()) },
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        }
    }

    fn settings_for(base: &str) -> MastSettings {
        let base = base.to_owned();
        MastSettings::from_lookup(move |name| match name {
            ENV_MAST_URL | ENV_MAST_AUTH_URL => Some(base.clone()),
            ENV_MAST_TIMEOUT_MS => Some("5000".to_owned()),
            _ => None,
        })
    }

    fn hub_on(server: &TestServer, cache: &Path, config: &Path) -> Arc<MastHub> {
        Arc::new(MastHub::new(settings_for(&server.base), cache.to_path_buf()).unwrap().with_token_store(TokenStore::new(config)))
    }

    struct Rig {
        server: TestServer,
        config: TempDir,
        cache: TempDir,
        hub: Arc<MastHub>,
    }

    async fn rig(routes: Vec<Route>) -> Rig {
        let server = serve_routes(routes).await;
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let hub = hub_on(&server, cache.path(), config.path());
        Rig { server, config, cache, hub }
    }

    impl Rig {
        fn key_file(&self) -> PathBuf {
            self.config.path().join("mast_api_key.txt")
        }

        fn stored_key(&self) -> Option<String> {
            std::fs::read_to_string(self.key_file()).ok()
        }

        fn transport(&self) -> Option<String> {
            let text = std::fs::read_to_string(self.config.path().join("config.json")).ok()?;
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            value["mast_token_transport"].as_str().map(str::to_owned)
        }

        fn config_files(&self) -> Vec<String> {
            let mut names: Vec<String> =
                std::fs::read_dir(self.config.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        }

        fn count(&self, path: &str) -> usize {
            self.server.requests().iter().filter(|r| r.path() == path).count()
        }

        fn roman_transport(&self) -> AuthTransport {
            self.hub.client_for(MastMissionId::Roman, OpKind::Search).unwrap().transport()
        }
    }

    async fn status(hub: &MastHub, check: bool) -> MastTokenStatus {
        token_status(hub, CHECK_ID, check).await.unwrap()
    }

    fn spawn_save(hub: &Arc<MastHub>) -> tokio::task::JoinHandle<Result<MastTokenStatus, MastCmdError>> {
        let hub = Arc::clone(hub);
        tokio::spawn(async move {
            let worker = Arc::clone(&hub);
            hub.registry().run(SAVE_ID, async move { token_save(&worker, SAVE_ID, MARKER_TOKEN).await }).await
        })
    }

    #[tokio::test]
    async fn save_rejects_bad_format_without_request() {
        let rig = rig(vec![info_route(false), auth_check_route(true)]).await;
        let short = "a".repeat(TOKEN_MIN_CHARS - 1);
        let long = "a".repeat(TOKEN_MAX_CHARS + 1);
        let spaced = format!("{} {}", &MARKER_TOKEN[..10], &MARKER_TOKEN[10..]);
        let accented = format!("{MARKER_TOKEN}\u{e9}");
        let tabbed = format!("{MARKER_TOKEN}\tx");
        let control = format!("{MARKER_TOKEN}\u{7f}");
        for bad in ["", "   ", short.as_str(), long.as_str(), spaced.as_str(), accented.as_str(), tabbed.as_str(), control.as_str()] {
            let error = token_save(&rig.hub, SAVE_ID, bad).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad:?}");
            assert_eq!(error.message, format!("token: {FORMAT_REASON}"));
            assert_eq!(error.context, MastErrorContext::default());
        }
        let error = token_save(&rig.hub, "bad id", MARKER_TOKEN).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert!(error.message.starts_with("requestId: "), "{}", error.message);
        assert!(rig.server.requests().is_empty());
        assert!(rig.config_files().is_empty());
        assert!(!rig.hub.has_token_client());
        assert_eq!(check_format(&"a".repeat(TOKEN_MIN_CHARS)).unwrap().expose().len(), TOKEN_MIN_CHARS);
        assert_eq!(check_format(&"~".repeat(TOKEN_MAX_CHARS)).unwrap().expose().len(), TOKEN_MAX_CHARS);
        assert_eq!(check_format(&format!("  {MARKER_TOKEN}\r\n")).unwrap().expose(), MARKER_TOKEN);
    }

    #[tokio::test]
    async fn token_commands_need_an_injected_store() {
        let server = serve_routes(vec![info_route(false), auth_check_route(true)]).await;
        let cache = tempfile::tempdir().unwrap();
        let hub = MastHub::new(settings_for(&server.base), cache.path().to_path_buf()).unwrap();
        let results = [
            token_save(&hub, SAVE_ID, MARKER_TOKEN).await,
            token_status(&hub, CHECK_ID, false).await,
            token_status(&hub, CHECK_ID, true).await,
            token_clear(&hub).await,
        ];
        for result in results {
            assert_eq!(result.unwrap_err().code, MastErrorCode::Cache);
        }
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn save_with_anon_true_writes_nothing() {
        let accepted = bearer();
        let rig = rig(vec![
            Route::get(INFO).when(move |r| r.header("authorization") == Some(accepted.as_str())).json(info_body(false)),
            info_route(true),
            auth_check_route(true),
        ])
        .await;
        let other = "AB-OTHER-TOKEN-fedcba9876543210";
        let error = token_save(&rig.hub, SAVE_ID, other).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::TokenRejected);
        assert_eq!(error.context, MastErrorContext::default());
        assert!(rig.config_files().is_empty());
        let requests = rig.server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path(), INFO);
        assert_eq!(requests[0].header("authorization"), Some(format!("token {other}").as_str()));
        assert_eq!(requests[0].header("accept"), Some("application/json"));
        assert_eq!(requests[0].header("cookie"), None);
        assert!(!rig.hub.has_token_client());
        assert_eq!(status(&rig.hub, false).await, ABSENT);
        assert!(rig.config_files().is_empty());

        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let rejected = token_save(&rig.hub, SAVE_ID, other).await.unwrap_err();
        assert_eq!(rejected.code, MastErrorCode::TokenRejected);
        assert_eq!(rig.stored_key().as_deref(), Some(MARKER_TOKEN));
        assert_eq!(rig.transport().as_deref(), Some("header"));
        assert_eq!(status(&rig.hub, false).await, present(Some(true)));
        assert_eq!(rig.count(AUTH_CHECK), 1);
    }

    #[tokio::test]
    async fn save_network_failure_during_info_writes_nothing() {
        let rig = rig(vec![Route::get(INFO).close_without_answer(), auth_check_route(true)]).await;
        let error = token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Network, "{error:?}");
        assert_eq!(error.context.operation.as_deref(), Some("info"));
        assert!(rig.config_files().is_empty());
        assert_eq!(rig.count(AUTH_CHECK), 0);
        assert_eq!(status(&rig.hub, false).await, ABSENT);

        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let refused = MastHub::new(settings_for(&format!("http://127.0.0.1:{port}")), cache.path().to_path_buf())
            .unwrap()
            .with_token_store(TokenStore::new(config.path()));
        let error = token_save(&refused, SAVE_ID, MARKER_TOKEN).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Network, "{error:?}");
        assert!(std::fs::read_dir(config.path()).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn save_header_authorized_persists_header_transport() {
        let rig = rig(vec![info_route(false), auth_check_route(true)]).await;
        let saved = token_save(&rig.hub, SAVE_ID, &format!(" {MARKER_TOKEN} ")).await.unwrap();
        assert_eq!(saved, present(Some(true)));
        assert_eq!(rig.stored_key().as_deref(), Some(MARKER_TOKEN));
        assert_eq!(rig.transport().as_deref(), Some("header"));
        let requests = rig.server.requests();
        let paths: Vec<&str> = requests.iter().map(Recorded::path).collect();
        assert_eq!(paths, [INFO, AUTH_CHECK]);
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert_eq!(request.header("authorization"), Some(bearer().as_str()), "{}", request.target);
            assert_eq!(request.header("cookie"), None, "{}", request.target);
        }
        assert!(rig.hub.has_token_client());
        assert_eq!(rig.roman_transport(), AuthTransport::Header);
        assert_eq!(status(&rig.hub, false).await, present(Some(true)));
        assert_eq!(rig.server.requests().len(), 2);
    }

    #[tokio::test]
    async fn save_cookie_fallback_persists_cookie_transport() {
        let mut routes = vec![info_route(false)];
        routes.extend(cookie_only_auth_check());
        let rig = rig(routes).await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        assert_eq!(rig.transport().as_deref(), Some("cookie"));
        let checks: Vec<Recorded> = rig.server.requests().into_iter().filter(|r| r.path() == AUTH_CHECK).collect();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].header("authorization"), Some(bearer().as_str()));
        assert_eq!(checks[0].header("cookie"), None);
        assert_eq!(checks[1].header("cookie"), Some(cookie_value().as_str()));
        assert_eq!(checks[1].header("authorization"), None);
        assert_eq!(rig.roman_transport(), AuthTransport::Cookie);
        assert_eq!(status(&rig.hub, false).await, present(Some(true)));
    }

    #[tokio::test]
    async fn save_not_authorized_persists_token_and_reports_false() {
        let rig = rig(vec![info_route(false), auth_check_route(false)]).await;
        config::set_mast_token_transport_in(rig.config.path(), Some("cookie")).unwrap();
        assert_eq!(rig.transport().as_deref(), Some("cookie"));
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(false)));
        assert_eq!(rig.stored_key().as_deref(), Some(MARKER_TOKEN));
        assert_eq!(rig.transport(), None);
        assert_eq!(rig.count(AUTH_CHECK), 2);
        assert!(rig.hub.has_token_client());
        assert_eq!(status(&rig.hub, false).await, present(Some(false)));
        assert_eq!(rig.server.requests().len(), 3);
    }

    #[tokio::test]
    async fn save_network_failure_during_auth_check_keeps_the_token() {
        let rig = rig(vec![info_route(false), Route::get(AUTH_CHECK).close_without_answer()]).await;
        let error = token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Network, "{error:?}");
        assert_eq!(error.context.operation.as_deref(), Some("search/roman/api/v0.1/auth_check"));
        assert_eq!(rig.stored_key().as_deref(), Some(MARKER_TOKEN));
        assert_eq!(rig.transport(), None);
        assert_eq!(status(&rig.hub, false).await, present(None));
        assert!(rig.hub.has_token_client());
    }

    #[tokio::test]
    async fn cancel_during_info_writes_nothing_and_during_auth_check_keeps_the_token() {
        let slow_info = rig(vec![Route::get(INFO).delay(Duration::from_secs(20)).json(info_body(false)), auth_check_route(true)]).await;
        let task = spawn_save(&slow_info.hub);
        slow_info.server.wait_for(|r| r.iter().any(|x| x.path() == INFO)).await;
        assert_eq!(slow_info.hub.registry().cancel(Some(&[SAVE_ID.to_owned()]), None).unwrap(), 1);
        assert_eq!(task.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert!(slow_info.config_files().is_empty());
        assert_eq!(status(&slow_info.hub, false).await, ABSENT);
        let aborted = slow_info.server.wait_for(|r| r.iter().any(|x| x.path() == INFO && x.aborted)).await;
        assert!(aborted.iter().any(|x| x.path() == INFO && x.aborted));

        let slow_check = rig(vec![info_route(false), Route::get(AUTH_CHECK).delay(Duration::from_secs(20)).json(r#"{"authorized":true}"#)]).await;
        let task = spawn_save(&slow_check.hub);
        slow_check.server.wait_for(|r| r.iter().any(|x| x.path() == AUTH_CHECK)).await;
        assert_eq!(slow_check.hub.registry().cancel(None, Some("token:")).unwrap(), 1);
        assert_eq!(task.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert_eq!(slow_check.stored_key().as_deref(), Some(MARKER_TOKEN));
        assert_eq!(status(&slow_check.hub, false).await, present(None));
        assert!(slow_check.hub.registry().is_empty());
    }

    #[tokio::test]
    async fn authorization_header_only_on_roman_and_info() {
        let rig = rig(vec![
            info_route(false),
            auth_check_route(true),
            Route::post(JWST_SEARCH).json(fixture("jwst_m16_l3.json")),
            Route::get(HST_THUMBNAIL).header("Content-Type", "image/jpeg").bytes(synthetic_jpeg(32, 24, false, None)),
            Route::post(LOOKUP).json(fixture("lookup_m16.json")),
            Route::post(ROMAN_SEARCH).json(EMPTY_SEARCH),
        ])
        .await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        search(&rig.hub, "g1:search", &m16(MastMissionId::Jwst), false).await.unwrap();
        thumbnail(&rig.hub, "g1:thumb:ICK909030", MastMissionId::Hst, "ICK909030", MastLevelClass::Unknown, None).await.unwrap();
        resolve(&rig.hub, "M16").await.unwrap().unwrap();
        search(&rig.hub, "g2:search", &m16(MastMissionId::Roman), false).await.unwrap();
        let requests = rig.server.requests();
        for path in [INFO, AUTH_CHECK, JWST_SEARCH, HST_THUMBNAIL, LOOKUP, ROMAN_SEARCH] {
            assert_eq!(requests.iter().filter(|r| r.path() == path).count(), 1, "{path}");
        }
        assert_eq!(requests.len(), 6);
        let bearer = bearer();
        for request in &requests {
            let carries = request.path() == INFO || request.path().starts_with(ROMAN_PREFIX);
            assert_eq!(request.header("authorization"), carries.then_some(bearer.as_str()), "{}", request.target);
            assert_eq!(request.header("cookie"), None, "{}", request.target);
        }
    }

    #[tokio::test]
    async fn cookie_only_on_roman_paths() {
        let mut routes = vec![
            info_route(false),
            Route::post(JWST_SEARCH).json(fixture("jwst_m16_l3.json")),
            Route::post(LOOKUP).json(fixture("lookup_m16.json")),
            Route::post(ROMAN_SEARCH).json(EMPTY_SEARCH),
        ];
        routes.extend(cookie_only_auth_check());
        let rig = rig(routes).await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        search(&rig.hub, "g1:search", &m16(MastMissionId::Jwst), false).await.unwrap();
        resolve(&rig.hub, "M16").await.unwrap().unwrap();
        search(&rig.hub, "g2:search", &m16(MastMissionId::Roman), false).await.unwrap();
        assert_eq!(status(&rig.hub, true).await, present(Some(true)));
        let requests = rig.server.requests();
        assert_eq!(rig.count(INFO), 2);
        assert_eq!(rig.count(AUTH_CHECK), 3);
        assert_eq!(rig.count(ROMAN_SEARCH), 1);
        let header_attempt = requests.iter().position(|r| r.path() == AUTH_CHECK).unwrap();
        let (bearer, cookie) = (bearer(), cookie_value());
        for (index, request) in requests.iter().enumerate() {
            let roman = request.path().starts_with(ROMAN_PREFIX);
            let by_cookie = roman && index != header_attempt;
            let by_header = request.path() == INFO || index == header_attempt;
            assert_eq!(request.header("cookie"), by_cookie.then_some(cookie.as_str()), "{}", request.target);
            assert_eq!(request.header("authorization"), by_header.then_some(bearer.as_str()), "{}", request.target);
        }
    }

    #[tokio::test]
    async fn status_without_check_makes_no_request() {
        let rig = rig(vec![info_route(false), auth_check_route(true)]).await;
        assert_eq!(status(&rig.hub, false).await, ABSENT);
        assert_eq!(status(&rig.hub, true).await, ABSENT);
        assert!(rig.server.requests().is_empty());
        assert!(rig.config_files().is_empty());
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let after_save = rig.server.requests().len();
        assert_eq!(status(&rig.hub, false).await, present(Some(true)));

        let restarted = hub_on(&rig.server, rig.cache.path(), rig.config.path());
        assert!(!restarted.has_token_client());
        assert_eq!(status(&restarted, false).await, present(None));
        assert!(restarted.has_token_client());
        assert_eq!(restarted.client_for(MastMissionId::Roman, OpKind::Search).unwrap().transport(), AuthTransport::Header);
        assert_eq!(rig.server.requests().len(), after_save);

        std::fs::write(rig.key_file(), "value").unwrap();
        assert_eq!(status(&rig.hub, false).await, ABSENT);
        assert!(!rig.hub.has_token_client());
        assert_eq!(status(&rig.hub, true).await, ABSENT);
        std::fs::remove_file(rig.key_file()).unwrap();
        assert_eq!(status(&restarted, false).await, ABSENT);
        assert!(!restarted.has_token_client());
        assert_eq!(rig.server.requests().len(), after_save);
        let error = token_status(&rig.hub, "bad id", false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
    }

    #[tokio::test]
    async fn status_check_rechecks_info_then_auth_check() {
        let rig = rig(vec![Route::get(INFO).times(2).json(info_body(false)), info_route(true), auth_check_route(true)]).await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let before = rig.server.requests().len();
        assert_eq!(status(&rig.hub, true).await, present(Some(true)));
        let requests = rig.server.requests();
        let paths: Vec<&str> = requests[before..].iter().map(Recorded::path).collect();
        assert_eq!(paths, [INFO, AUTH_CHECK]);
        for request in &requests[before..] {
            assert_eq!(request.header("authorization"), Some(bearer().as_str()));
        }
        let error = token_status(&rig.hub, CHECK_ID, true).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::TokenRejected);
        assert_eq!(rig.count(INFO), 3);
        assert_eq!(rig.count(AUTH_CHECK), 2);
        assert_eq!(status(&rig.hub, false).await, present(Some(false)));
        assert_eq!(rig.stored_key().as_deref(), Some(MARKER_TOKEN));
    }

    #[tokio::test]
    async fn roman_401_after_authorized_triggers_one_recheck() {
        let expired = rig(vec![
            Route::get(INFO).times(1).json(info_body(false)),
            info_route(true),
            auth_check_route(true),
            Route::post(ROMAN_SEARCH).status(401).json(ROMAN_401),
        ])
        .await;
        assert_eq!(token_save(&expired.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let error = search(&expired.hub, "g1:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::TokenRejected);
        assert_eq!(expired.count(INFO), 2);
        assert_eq!(status(&expired.hub, false).await, present(Some(false)));
        let again = search(&expired.hub, "g2:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(again.code, MastErrorCode::Unauthorized);
        assert_eq!(again.context.status, Some(401));
        assert_eq!(again.context.mission, Some(MastMissionId::Roman));
        assert_eq!(expired.count(INFO), 2);
        assert_eq!(expired.count(ROMAN_SEARCH), 2);

        let removed = rig(vec![info_route(false), auth_check_route(true), Route::post(ROMAN_SEARCH).status(401).json(ROMAN_401)]).await;
        assert_eq!(token_save(&removed.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let error = search(&removed.hub, "g1:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Unauthorized);
        assert_eq!(error.context.status, Some(401));
        assert_eq!(removed.count(INFO), 2);
        assert_eq!(status(&removed.hub, false).await, present(Some(false)));
        search(&removed.hub, "g2:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(removed.count(INFO), 2);

        let unconfirmed = rig(vec![info_route(false), auth_check_route(false), Route::post(ROMAN_SEARCH).status(401).json(ROMAN_401)]).await;
        assert_eq!(token_save(&unconfirmed.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(false)));
        let error = search(&unconfirmed.hub, "g1:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Unauthorized);
        assert_eq!(unconfirmed.count(INFO), 1);
    }

    #[tokio::test]
    async fn stale_401_from_a_replaced_token_keeps_the_new_confirmation() {
        let old = "AB-OTHER-TOKEN-fedcba9876543210";
        let old_bearer = format!("token {old}");
        let rig = rig(vec![
            Route::post(ROMAN_SEARCH)
                .when(move |r| r.header("authorization") == Some(old_bearer.as_str()))
                .delay(Duration::from_millis(2500))
                .status(401)
                .json(ROMAN_401),
            info_route(false),
            auth_check_route(true),
            Route::post(ROMAN_SEARCH).json(EMPTY_SEARCH),
        ])
        .await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, old).await.unwrap(), present(Some(true)));
        let hub = Arc::clone(&rig.hub);
        let stale = tokio::spawn(async move { search(&hub, "g1:search", &m16(MastMissionId::Roman), false).await });
        rig.server.wait_for(|r| r.iter().any(|x| x.path() == ROMAN_SEARCH)).await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let error = stale.await.unwrap().unwrap_err();
        assert_eq!(error.code, MastErrorCode::Unauthorized);
        assert_eq!(error.context.status, Some(401));
        assert_eq!(rig.count(INFO), 2);
        assert_eq!(status(&rig.hub, false).await, present(Some(true)));
        search(&rig.hub, "g2:search", &m16(MastMissionId::Roman), false).await.unwrap();
        assert_eq!(rig.count(ROMAN_SEARCH), 2);
    }

    #[tokio::test]
    async fn recheck_covers_image_and_product_calls() {
        let rig = rig(vec![
            Route::get(INFO).times(1).json(info_body(false)),
            Route::get(INFO).times(1).json(info_body(true)),
            Route::get(INFO).times(1).json(info_body(false)),
            info_route(true),
            auth_check_route(true),
            Route::get("/search/roman/api/v0.1/get_image_list").status(401).json(ROMAN_401),
            Route::post("/search/roman/api/v0.1/post_list_products").status(401).json(ROMAN_401),
        ])
        .await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let listed = image_list(&rig.hub, MastMissionId::Roman, "r0001").await.unwrap_err();
        assert_eq!(listed.code, MastErrorCode::TokenRejected);
        assert_eq!(rig.count(INFO), 2);
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let listed = products(&rig.hub, "g1:products", MastMissionId::Roman, &["r0001".to_owned()], false).await.unwrap_err();
        assert_eq!(listed.code, MastErrorCode::TokenRejected);
        assert_eq!(rig.count(INFO), 4);
        assert_eq!(status(&rig.hub, false).await, present(Some(false)));
    }

    #[tokio::test]
    async fn clear_removes_file_transport_and_client() {
        let mut routes = vec![info_route(false), Route::post(ROMAN_SEARCH).json(EMPTY_SEARCH)];
        routes.extend(cookie_only_auth_check());
        let rig = rig(routes).await;
        assert_eq!(token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        assert!(rig.key_file().is_file());
        assert_eq!(rig.transport().as_deref(), Some("cookie"));
        assert_eq!(token_clear(&rig.hub).await.unwrap(), ABSENT);
        assert!(!rig.key_file().exists());
        assert_eq!(rig.transport(), None);
        assert!(!rig.hub.has_token_client());
        let before = rig.server.requests().len();
        let error = search(&rig.hub, "g1:search", &m16(MastMissionId::Roman), false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Unauthorized);
        assert_eq!(error.context.status, None);
        assert_eq!(status(&rig.hub, false).await, ABSENT);
        assert_eq!(status(&rig.hub, true).await, ABSENT);
        assert_eq!(token_clear(&rig.hub).await.unwrap(), ABSENT);
        assert_eq!(rig.server.requests().len(), before);
        assert_eq!(rig.count(ROMAN_SEARCH), 0);
    }

    #[tokio::test]
    async fn info_html_answer_is_decode_error() {
        let rig = rig(vec![
            Route::get(INFO).times(1).html("<html><body>Anon: False</body></html>"),
            Route::get(INFO).times(1).json("null"),
            Route::get(INFO).times(1).json(""),
            Route::get(INFO).times(1).json(r#"{"anon":"false"}"#),
            Route::get(INFO).times(1).json(r#"{"token":null}"#),
            Route::get(INFO).times(1).status(302).header("Location", "/elsewhere"),
            Route::get("/elsewhere").json(info_body(false)),
            auth_check_route(true),
        ])
        .await;
        for round in 0..6 {
            let error = token_save(&rig.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::Decode, "round {round}: {error:?}");
            assert_eq!(error.context.operation.as_deref(), Some("info"), "round {round}");
            assert_eq!(error.context.detail, None, "round {round}");
        }
        assert_eq!(rig.count(INFO), 6);
        assert_eq!(rig.count("/elsewhere"), 0);
        assert_eq!(rig.count(AUTH_CHECK), 0);
        assert!(rig.config_files().is_empty());
    }

    fn record(printed: &mut Vec<String>, error: &MastCmdError) {
        printed.push(format!("{error:?}"));
        printed.push(error.to_string());
        printed.push(serde_json::to_string(error).unwrap());
    }

    #[tokio::test]
    async fn marker_token_never_in_errors_debug_or_events() {
        let mut printed: Vec<String> = Vec::new();
        let rejected = rig(vec![info_route(true)]).await;
        record(&mut printed, &token_save(&rejected.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err());
        let offline = rig(vec![Route::get(INFO).close_without_answer()]).await;
        record(&mut printed, &token_save(&offline.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err());
        let html = rig(vec![Route::get(INFO).html("<html>Anon</html>")]).await;
        record(&mut printed, &token_save(&html.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err());
        record(&mut printed, &token_save(&html.hub, SAVE_ID, &format!("{MARKER_TOKEN} {MARKER_TOKEN}")).await.unwrap_err());
        let unreachable = rig(vec![info_route(false), Route::get(AUTH_CHECK).close_without_answer()]).await;
        record(&mut printed, &token_save(&unreachable.hub, SAVE_ID, MARKER_TOKEN).await.unwrap_err());

        let preview_list = r#"[{"url":"/get_image_preview?fileset_name=r0001&filename=r0001_cal.jpg","filename":"r0001_cal.jpg","category":"PREVIEW"}]"#;
        let roman = rig(vec![
            Route::get(INFO).times(1).json(info_body(false)),
            info_route(true),
            auth_check_route(true),
            Route::post(ROMAN_SEARCH).times(1).json(EMPTY_SEARCH),
            Route::post(ROMAN_SEARCH).status(401).json(ROMAN_401),
            Route::get("/search/roman/api/v0.1/get_image_list?fileset_name=r0001&category=THUMBNAIL").json("[]"),
            Route::get("/search/roman/api/v0.1/get_image_list?fileset_name=r0001&category=PREVIEW").json(preview_list),
            Route::get("/search/roman/api/v0.1/get_image_preview")
                .header("Content-Type", "application/octet-stream")
                .bytes(synthetic_jpeg(64, 48, false, None)),
        ])
        .await;
        assert_eq!(token_save(&roman.hub, SAVE_ID, MARKER_TOKEN).await.unwrap(), present(Some(true)));
        let page = search(&roman.hub, "g1:search", &m16(MastMissionId::Roman), false).await.unwrap();
        printed.push(serde_json::to_string(&page).unwrap());
        let events = Arc::new(Mutex::new(Vec::<MastProgressEvent>::new()));
        let sink = {
            let events = Arc::clone(&events);
            move |event: MastProgressEvent| events.lock().unwrap().push(event)
        };
        let shown = preview(&roman.hub, "g1:preview:r0001", MastMissionId::Roman, "r0001", "r0001_cal.jpg", PreviewSize::Pane, &sink)
            .await
            .unwrap();
        printed.push(serde_json::to_string(&shown).unwrap());
        let events = events.lock().unwrap().clone();
        assert_eq!(events.last().map(|e| e.stage), Some(ProgressStage::Ready));
        for event in &events {
            printed.push(format!("{event:?}"));
            printed.push(serde_json::to_string(event).unwrap());
        }
        let debug_hub = format!("{:?}", roman.hub);
        assert!(debug_hub.contains("has_token_client: true"), "{debug_hub}");
        printed.push(debug_hub);
        printed.push(format!("{:#?}", roman.hub.client_for(MastMissionId::Roman, OpKind::Preview).unwrap()));
        printed.push(format!("{:?}", TokenStore::new(roman.config.path())));
        record(&mut printed, &search(&roman.hub, "g1:search", &m16(MastMissionId::Roman), true).await.unwrap_err());
        record(&mut printed, &search(&roman.hub, "g2:search", &m16(MastMissionId::Roman), true).await.unwrap_err());
        printed.push(format!("{:?}", roman.hub));

        assert!(printed.len() > 20);
        for text in &printed {
            assert!(!text.contains(MARKER_TOKEN), "{text}");
            assert!(!text.contains("0123456789abcdef"), "{text}");
        }
        assert!(printed.iter().any(|t| t.contains("mast_token_rejected")));
        assert!(printed.iter().any(|t| t.contains("mast_unauthorized")));
    }
}
