use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::Manager;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, image_ref, load_cached_full, output_stem,
    render_named_and_save, resolve_output_dir, source_path, OutputValues,
};
use crate::core::astrometry::catalog::{
    query_gaia_cached, solver_cone_query, CatalogRow, ConeQuery, GAIA_MATCH_MAG_LIMIT, GAIA_VIZIER_TABLE,
};
use crate::core::astrometry::frames::{convert_from_icrs, convert_to_icrs, SkyFrame};
use crate::core::astrometry::gaia_tie::{GAIA_DR3_ATTRIBUTION, GAIA_DR3_ATTRIBUTION_SIRIL, GAIA_ESA_ACKNOWLEDGEMENT};
use crate::core::astrometry::grid::{wcs_grid, WcsGrid, DEFAULT_DENSITY, MAX_DENSITY, MIN_DENSITY};
use crate::core::astrometry::local_solve::{
    solve_local, CatalogProvider, CatalogTier, FailureReason, FieldSize, ImageParity, LocalSolution,
    LocalSolveParams, RankingKey, SolveFailure, TierChain, MAX_NEAR_RADIUS_DEG, TIER0_MEAN_STARS_PER_DEG2,
};
use crate::core::astrometry::plate_solve::{
    orientation_east_of_north_deg, parity_name, SolveConfig, SolveResult, SOLVER_ASTROMETRY_NET, SOLVER_LOCAL,
};
use crate::core::astrometry::solve_hints::{solve_hints, Pointing, ScaleRange, SolveHints};
use crate::core::astrometry::wcs::{
    angular_separation, pixel_center, pixel_edge_corners, position_angle_deg, WcsKind, WcsTransform,
};
use crate::infra::catalog_store::{
    CatalogLease, CatalogStore, DatasetStatus, StoreRoots, SIRIL_GAIA_ID, TWOMASS_ID, TYCHO2_ID, ZENODO_RECORD_ID,
};
use crate::infra::config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::writer::{filter_header, is_wcs_card};
use crate::infra::image_source::{load_plane, load_plane_header};
use crate::infra::progress::ProgressHandle;
use crate::infra::wcs_source::{load_wcs, tan_pv_warning};
use crate::types::constants::{
    DEFAULT_API_KEY_SERVICE, HEADER_NAXIS1, HEADER_NAXIS2, RES_A_SKY, RES_B_SKY, RES_CENTER_DEC,
    RES_CENTER_RA, RES_DIMENSIONS, RES_EAST_VEC, RES_ELAPSED_MS, RES_FITS_PATH, RES_FLIPPED,
    RES_FOV_ARCMIN, RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN, RES_FRAME, RES_GWCS_FRAMES, RES_GWCS_REFUSAL,
    RES_GWCS_SOURCE, RES_GWCS_STEPS, RES_GWCS_VS_SIP_MAX_MAS, RES_GWCS_VS_SIP_MAX_PX, RES_NAXIS1,
    RES_NAXIS2, RES_NORTH_VEC, RES_ON_IMAGE, RES_PARITY, RES_PIXEL_LENGTH, RES_PIXEL_SCALE_ARCSEC,
    RES_PIXEL_SCALE_X_ARCSEC, RES_PIXEL_SCALE_Y_ARCSEC, RES_PNG_PATH, RES_POINTS,
    RES_POSITION_ANGLE_DEG, RES_PROJECTION, RES_ROTATION_DEG, RES_SEPARATION_ARCMIN,
    RES_SEPARATION_ARCSEC, RES_SEPARATION_DEG, RES_SIP_INV_ERR_PX, RES_SIP_MAX_ERR_PX,
    RES_SIP_PRESENT, RES_WCS_KIND, RES_WCS_WARNING,
};
use crate::types::config::AppConfig;
use crate::types::header::HduHeader;

pub(crate) mod gaia_refine;

const MAX_UPLOAD_DIM: usize = 2048;
const MAX_SKY_POINTS: usize = 20_000;
const MAX_LATITUDE_DEG: f64 = 90.0;
const PARITY_NORMAL: &str = "normal";
const PARITY_FLIPPED: &str = "flipped";
const ABPROC_PLATE_SOLVED: &str = "platesolved";
const RADESYS_KEY: &str = "RADESYS";
const DEPRECATED_RADESYS_KEY: &str = "RADECSYS";
const PLATE_SOLVE_PROGRESS_EVENT: &str = "plate-solve-progress";

fn load_header_and_wcs(path: &str) -> anyhow::Result<(crate::types::header::HduHeader, WcsTransform)> {
    let header = load_plane_header(&image_ref(path))?;
    let wcs = load_wcs(path, &header)?;
    Ok((header, wcs))
}

type WcsStamp = (u64, Option<std::time::SystemTime>);

#[derive(Clone)]
struct CachedWcs {
    wcs: std::sync::Arc<WcsTransform>,
    naxis1: usize,
    naxis2: usize,
}

static WCS_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (WcsStamp, CachedWcs)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const WCS_CACHE_CAPACITY: usize = 64;

type GridCacheKey = (String, &'static str, u8);

static GRID_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<GridCacheKey, (WcsStamp, std::sync::Arc<WcsGrid>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const GRID_CACHE_CAPACITY: usize = 32;

fn file_stamp(path: &str) -> Option<WcsStamp> {
    std::fs::metadata(source_path(path)).ok().map(|m| (m.len(), m.modified().ok()))
}

fn load_wcs_with_dims_cached(path: &str) -> anyhow::Result<CachedWcs> {
    let stamp = file_stamp(path);

    if let Some(st) = &stamp {
        let cache = WCS_CACHE.lock().unwrap();
        if let Some((cached_stamp, cached)) = cache.get(path) {
            if cached_stamp == st {
                return Ok(cached.clone());
            }
        }
    }

    let header = load_plane_header(&image_ref(path))?;
    let (naxis1, naxis2) = image_dims(&header);
    let cached = CachedWcs {
        wcs: std::sync::Arc::new(load_wcs(path, &header)?),
        naxis1,
        naxis2,
    };

    if let Some(st) = stamp {
        let mut cache = WCS_CACHE.lock().unwrap();
        if cache.len() >= WCS_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(path.to_string(), (st, cached.clone()));
    }

    Ok(cached)
}

fn load_wcs_cached(path: &str) -> anyhow::Result<std::sync::Arc<WcsTransform>> {
    load_wcs_with_dims_cached(path).map(|c| c.wcs)
}

fn load_grid_cached(path: &str, frame: SkyFrame, density: u8) -> anyhow::Result<std::sync::Arc<WcsGrid>> {
    let stamp = file_stamp(path);
    let key: GridCacheKey = (path.to_string(), frame.name(), density);

    if let Some(st) = &stamp {
        let cache = GRID_CACHE.lock().unwrap();
        if let Some((cached_stamp, grid)) = cache.get(&key) {
            if cached_stamp == st {
                return Ok(std::sync::Arc::clone(grid));
            }
        }
    }

    let cached = load_wcs_with_dims_cached(path)?;
    let grid = std::sync::Arc::new(
        wcs_grid(&cached.wcs, cached.naxis1, cached.naxis2, frame, density)
            .map_err(|e| anyhow::anyhow!(e))?,
    );

    if let Some(st) = stamp {
        let mut cache = GRID_CACHE.lock().unwrap();
        if cache.len() >= GRID_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(key, (st, std::sync::Arc::clone(&grid)));
    }

    Ok(grid)
}

fn plate_solve_timeout(loaded: anyhow::Result<AppConfig>) -> anyhow::Result<u64> {
    let secs = match loaded {
        Ok(cfg) => cfg.plate_solve_timeout_secs,
        Err(e) => {
            log::warn!("config unavailable ({:#}); using the default plate-solve timeout", e);
            AppConfig::default().plate_solve_timeout_secs
        }
    };
    if secs == 0 {
        anyhow::bail!("the plate-solve timeout is 0 seconds; set it to at least 1 second in Settings");
    }
    Ok(secs)
}

fn resolve_api_key(provided: Option<String>) -> Option<String> {
    if let Some(ref k) = provided {
        if !k.is_empty() {
            return provided;
        }
    }
    config::load_api_key(DEFAULT_API_KEY_SERVICE).ok().flatten()
}

const SOLVER_AUTO: &str = "auto";
const SOLVER_CHOICES: [&str; 3] = [SOLVER_AUTO, SOLVER_LOCAL, SOLVER_ASTROMETRY_NET];
const CODE_LOCAL_NO_SOLUTION: &str = "local_no_solution";
const CODE_LOCAL_MISSING_HINT: &str = "local_missing_hint";
const CODE_LOCAL_NO_CATALOGUE: &str = "local_no_catalogue";
const CODE_NO_API_KEY: &str = "astrometry_net_no_key";
const CODE_CANCELLED: &str = "cancelled";
const CANCELLED_MESSAGE: &str = "Plate solve cancelled";
const NO_API_KEY_MESSAGE: &str = "astrometry.net needs an API key: save it in Settings; nothing was uploaded";
const NO_NOVA_MESSAGE: &str = "this build has no astrometry.net support (cargo feature \"astrometry-net\")";
const PLATE_SOLVE_HOLDER: &str = "a running plate solve";
const PANEL_HINT_SOURCE: &str = "typed in the panel";
const SAME_CENTRE_DEG: f64 = 1e-5;
const SCALE_UNITS_ARCMIN_WIDTH: &str = "arcminwidth";
const SCALE_UNITS_DEG_WIDTH: &str = "degwidth";
const ARCSEC_PER_DEG: f64 = 3600.0;
const ARCSEC_PER_ARCMIN: f64 = 60.0;
const STAGE_LOAD: &str = "load";
pub(crate) const TIER0_LABEL: &str = "tier0-tycho2";
const TIER0_DEPTH_MAG: f64 = 12.0;
pub(crate) const TIER1_SIRIL_LABEL: &str = "tier1-gaia-siril";
const TIER1_SIRIL_DEPTH_MAG: f64 = 17.0;
pub(crate) const TIER1_2MASS_LABEL: &str = "tier1-2mass";
const TIER1_2MASS_DEPTH_MAG: f64 = 16.0;
pub(crate) const TIER2_LABEL: &str = "tier2-gaia-vizier";
const TIER2_FIELD_STARS: f64 = 900.0;
const TIER2_MIN_ROWS: usize = 1_000;
const TIER2_MAX_ROWS: usize = 20_000;
const TIER2_NEAR_FIELDS: f64 = 0.5;
const ONLINE_CANCEL_POLL: Duration = Duration::from_millis(100);
const ONLINE_QUERY_STOPPED: &str = "the online catalogue query stopped without an answer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SolverChoice {
    Auto,
    Local,
    AstrometryNet,
}

impl SolverChoice {
    fn parse(text: Option<&str>) -> Result<Self, String> {
        match text.map(str::trim).filter(|t| !t.is_empty()) {
            None => Ok(Self::Auto),
            Some(t) if t.eq_ignore_ascii_case(SOLVER_AUTO) => Ok(Self::Auto),
            Some(t) if t.eq_ignore_ascii_case(SOLVER_LOCAL) => Ok(Self::Local),
            Some(t) if t.eq_ignore_ascii_case(SOLVER_ASTROMETRY_NET) => Ok(Self::AstrometryNet),
            Some(t) => Err(format!("unknown solver {t:?}; use one of {}", SOLVER_CHOICES.join(", "))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PlateSolveRequest {
    pub path: String,
    pub solver: Option<String>,
    pub api_key: Option<String>,
    pub scale_lower: Option<f64>,
    pub scale_upper: Option<f64>,
    pub scale_units: Option<String>,
    pub downsample_factor: Option<u32>,
    pub center_ra: Option<f64>,
    pub center_dec: Option<f64>,
    pub radius: Option<f64>,
    pub near_radius_deg: Option<f64>,
}

pub(crate) struct NovaSettings {
    pub api_url: String,
    pub timeout_secs: u64,
}

pub(crate) struct NovaUpload {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub config: SolveConfig,
}

pub(crate) type NovaFuture = Pin<Box<dyn Future<Output = anyhow::Result<SolveResult>> + Send>>;

pub(crate) struct TierRequest<'a> {
    pub hints: &'a SolveHints,
    pub width: usize,
    pub height: usize,
    pub online: bool,
}

#[derive(Default)]
pub(crate) struct TierSet {
    pub chain: TierChain,
    pub labels: Vec<&'static str>,
    pub notes: Vec<String>,
}

impl TierSet {
    pub fn push(&mut self, tier: CatalogTier, label: &'static str, provider: Box<dyn CatalogProvider>) {
        self.chain.tiers.push((tier, provider));
        self.labels.push(label);
    }
}

pub(crate) trait SolveEnv: Send + Sync {
    fn tiers(&self, request: &TierRequest<'_>) -> TierSet;
    fn stored_api_key(&self) -> Option<String>;
    fn nova_settings(&self) -> anyhow::Result<NovaSettings>;
    fn has_nova(&self) -> bool;
    fn nova(&self, upload: NovaUpload) -> Option<NovaFuture>;
    fn cancelled(&self) -> bool;
    fn stage(&self, stage: &str);
}

struct StoreProvider {
    lease: CatalogLease,
    label: &'static str,
    depth_mag: f64,
    ranking: RankingKey,
}

impl CatalogProvider for StoreProvider {
    fn cone(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        mag_limit: Option<f64>,
    ) -> Result<(Vec<CatalogRow>, &'static str), String> {
        let mut rows = self
            .lease
            .cone(ra_deg, dec_deg, radius_deg, None)
            .map_err(|error| error.to_string())?;
        if let Some(limit) = mag_limit {
            rows.retain(|row| row.g.is_some_and(|mag| mag <= limit));
        }
        Ok((rows, self.label))
    }

    fn depth_mag(&self) -> f64 {
        self.depth_mag
    }

    fn ranking_key(&self) -> RankingKey {
        self.ranking
    }
}

pub(crate) type GaiaFetch = Arc<dyn Fn(&ConeQuery) -> Result<Vec<CatalogRow>, String> + Send + Sync>;
pub(crate) type CancelCheck = Arc<dyn Fn() -> bool + Send + Sync>;

pub(crate) struct GaiaRegionProvider {
    query: ConeQuery,
    fetch: GaiaFetch,
    cancelled: CancelCheck,
    rows: OnceLock<Result<Arc<Vec<CatalogRow>>, String>>,
}

impl GaiaRegionProvider {
    pub(crate) fn new(query: ConeQuery, fetch: GaiaFetch, cancelled: CancelCheck) -> Self {
        Self {
            query,
            fetch,
            cancelled,
            rows: OnceLock::new(),
        }
    }

    fn region(&self) -> Result<Arc<Vec<CatalogRow>>, String> {
        self.rows
            .get_or_init(|| self.fetch_unless_cancelled().map(Arc::new))
            .clone()
    }

    fn fetch_unless_cancelled(&self) -> Result<Vec<CatalogRow>, String> {
        if (self.cancelled)() {
            return Err(CANCELLED_MESSAGE.to_string());
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let (fetch, query) = (Arc::clone(&self.fetch), self.query.clone());
        std::thread::spawn(move || sender.send(fetch(&query)));
        loop {
            match receiver.recv_timeout(ONLINE_CANCEL_POLL) {
                Ok(rows) => return rows,
                Err(RecvTimeoutError::Timeout) if (self.cancelled)() => return Err(CANCELLED_MESSAGE.to_string()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(ONLINE_QUERY_STOPPED.to_string()),
            }
        }
    }
}

impl CatalogProvider for GaiaRegionProvider {
    fn cone(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        mag_limit: Option<f64>,
    ) -> Result<(Vec<CatalogRow>, &'static str), String> {
        let region = self.region()?;
        let rows = region
            .iter()
            .filter(|row| angular_separation(ra_deg, dec_deg, row.ra, row.dec) <= radius_deg)
            .filter(|row| mag_limit.is_none_or(|limit| row.g.is_some_and(|mag| mag <= limit)))
            .cloned()
            .collect();
        Ok((rows, TIER2_LABEL))
    }

    fn depth_mag(&self) -> f64 {
        self.query.mag_limit.unwrap_or(GAIA_MATCH_MAG_LIMIT)
    }
}

fn plain_error(text: impl Into<String>) -> serde_json::Value {
    serde_json::Value::String(text.into())
}

fn coded_error(code: &str, message: impl Into<String>, context: serde_json::Value) -> serde_json::Value {
    json!({ "message": message.into(), "code": code, "context": context })
}

fn cancelled_error() -> serde_json::Value {
    coded_error(CODE_CANCELLED, CANCELLED_MESSAGE, json!({}))
}

fn join_error(error: tokio::task::JoinError) -> serde_json::Value {
    plain_error(format!("Task join failed: {error}"))
}

fn tier_number(tier: CatalogTier) -> u8 {
    match tier {
        CatalogTier::Tier0 => 0,
        CatalogTier::Tier1 => 1,
        CatalogTier::Tier2 => 2,
    }
}

pub(crate) async fn run_plate_solve(
    env: Arc<dyn SolveEnv>,
    request: PlateSolveRequest,
) -> Result<serde_json::Value, serde_json::Value> {
    let choice = SolverChoice::parse(request.solver.as_deref()).map_err(plain_error)?;
    let solved = match choice {
        SolverChoice::AstrometryNet => solve_with_nova(env, request).await,
        SolverChoice::Auto | SolverChoice::Local => {
            let online = choice == SolverChoice::Auto;
            tokio::task::spawn_blocking(move || solve_locally(env.as_ref(), &request, online))
                .await
                .map_err(join_error)?
        }
    }?;
    Ok(with_solve_credit(solved))
}

struct NovaPrepared {
    upload_path: String,
    upload_dims: (usize, usize),
    original_dims: (usize, usize),
    ds_factor: Option<(f64, f64)>,
    config: SolveConfig,
    _resolved: Option<Arc<tempfile::TempDir>>,
    _upload: Option<tempfile::NamedTempFile>,
}

fn prepare_nova_upload(
    request: &PlateSolveRequest,
    api_key: String,
    settings: NovaSettings,
) -> anyhow::Result<NovaPrepared> {
    let r = image_ref(&request.path);
    let (resolved_path, resolved) = resolve_single_image(&r.path)?;
    let loaded = load_plane(&r)?;
    let (image, header) = (loaded.arr, loaded.header);
    let (naxis2, naxis1) = image.dim();

    let target_dims: Option<(usize, usize)> = if let Some(f) = request.downsample_factor.filter(|&f| f > 1) {
        Some(((naxis2 / f as usize).max(1), (naxis1 / f as usize).max(1)))
    } else if naxis1 > MAX_UPLOAD_DIM || naxis2 > MAX_UPLOAD_DIM {
        let scale = MAX_UPLOAD_DIM as f64 / naxis1.max(naxis2) as f64;
        Some((
            ((naxis2 as f64 * scale).round() as usize).max(1),
            ((naxis1 as f64 * scale).round() as usize).max(1),
        ))
    } else {
        None
    };

    let (upload_path, upload, ds_factor, upload_dims) = if let Some((ds_rows, ds_cols)) = target_dims {
        log::info!("Plate solve: downsampling {}x{} to {}x{} for upload", naxis1, naxis2, ds_cols, ds_rows);
        let downsampled = crate::core::alignment::downsample::area_downsample(&image, ds_rows, ds_cols);
        let tmp_file = tempfile::Builder::new().suffix(".fits").tempfile()?;
        let tmp_path = tmp_file.path().to_string_lossy().to_string();
        crate::infra::fits::writer::write_fits_mono(&tmp_path, &downsampled, Some(&header))?;
        let fx = naxis1 as f64 / ds_cols as f64;
        let fy = naxis2 as f64 / ds_rows as f64;
        (tmp_path, Some(tmp_file), Some((fx, fy)), (ds_cols, ds_rows))
    } else if r.is_auto() {
        (resolved_path.to_string_lossy().to_string(), None, None, (naxis1, naxis2))
    } else {
        let tmp_file = tempfile::Builder::new().suffix(".fits").tempfile()?;
        let tmp_path = tmp_file.path().to_string_lossy().to_string();
        crate::infra::fits::writer::write_fits_mono(&tmp_path, &image, Some(&header))?;
        (tmp_path, Some(tmp_file), None, (naxis1, naxis2))
    };

    let is_pixel_scale = request.scale_units.as_deref().is_none_or(|u| u.contains("pix"));
    let hint_scale = if is_pixel_scale {
        ds_factor.map_or(1.0, |(fx, fy)| (fx * fy).sqrt())
    } else {
        1.0
    };

    let config = SolveConfig {
        api_url: settings.api_url,
        api_key,
        ra_hint: request.center_ra,
        dec_hint: request.center_dec,
        radius_hint: request.radius,
        scale_low: request.scale_lower.map(|v| v * hint_scale),
        scale_high: request.scale_upper.map(|v| v * hint_scale),
        scale_units: request.scale_units.clone(),
        timeout_secs: settings.timeout_secs,
    };

    Ok(NovaPrepared {
        upload_path,
        upload_dims,
        original_dims: (naxis1, naxis2),
        ds_factor,
        config,
        _resolved: resolved,
        _upload: upload,
    })
}

fn cards_flipped(cards: &[(String, String)], (width, height): (usize, usize)) -> Option<bool> {
    if cards.is_empty() {
        return None;
    }
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key.trim(), value.clone());
    }
    header.set(HEADER_NAXIS1, width.max(1).to_string());
    header.set(HEADER_NAXIS2, height.max(1).to_string());
    WcsTransform::from_header(&header)
        .ok()
        .map(|wcs| wcs.orientation(width.max(1), height.max(1)).flipped)
}

fn sip_order_of(cards: &[(String, String)]) -> Option<u8> {
    cards
        .iter()
        .find(|(key, _)| key.trim() == "A_ORDER")
        .and_then(|(_, value)| value.trim().parse::<u8>().ok())
        .filter(|&order| order > 0)
}

fn finish_nova_result(result: &mut SolveResult, prepared: &NovaPrepared) {
    if let Some((fx, fy)) = prepared.ds_factor {
        rescale_solve_to_original(result, fx, fy, prepared.original_dims.0, prepared.original_dims.1);
        result.wcs_cards = rescale_wcs_cards_to_original(&result.wcs_cards, prepared.upload_dims, prepared.original_dims);
    }
    result.solver = SOLVER_ASTROMETRY_NET.to_string();
    if let Some(flipped) = cards_flipped(&result.wcs_cards, prepared.original_dims) {
        result.parity = Some(parity_name(flipped).to_string());
    }
    result.sip_order = sip_order_of(&result.wcs_cards);
}

async fn solve_with_nova(
    env: Arc<dyn SolveEnv>,
    request: PlateSolveRequest,
) -> Result<serde_json::Value, serde_json::Value> {
    if !env.has_nova() {
        return Err(plain_error(NO_NOVA_MESSAGE));
    }
    let blocking_env = Arc::clone(&env);
    let prepared = tokio::task::spawn_blocking(move || -> Result<NovaPrepared, serde_json::Value> {
        let api_key = request
            .api_key
            .clone()
            .filter(|key| !key.trim().is_empty())
            .or_else(|| blocking_env.stored_api_key())
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| coded_error(CODE_NO_API_KEY, NO_API_KEY_MESSAGE, json!({})))?;
        let settings = blocking_env.nova_settings().map_err(|e| plain_error(e.to_string()))?;
        prepare_nova_upload(&request, api_key, settings).map_err(|e| plain_error(e.to_string()))
    })
    .await
    .map_err(join_error)??;
    let upload = NovaUpload {
        path: prepared.upload_path.clone(),
        width: prepared.upload_dims.0,
        height: prepared.upload_dims.1,
        config: prepared.config.clone(),
    };
    log::info!(
        "Plate solve: sending {} ({}x{}) to {}",
        upload.path,
        upload.width,
        upload.height,
        upload.config.api_url
    );
    let solve = env.nova(upload).ok_or_else(|| plain_error(NO_NOVA_MESSAGE))?;
    let mut result = solve.await.map_err(|e| plain_error(e.to_string()))?;
    finish_nova_result(&mut result, &prepared);
    serde_json::to_value(&result).map_err(|e| plain_error(e.to_string()))
}

fn checked_near_radius(value: Option<f64>) -> Result<Option<f64>, String> {
    match value {
        None => Ok(None),
        Some(r) if r.is_finite() && (0.0..=MAX_NEAR_RADIUS_DEG).contains(&r) => Ok(Some(r)),
        Some(r) => Err(format!(
            "the near-search radius must be 0 to {MAX_NEAR_RADIUS_DEG} degrees, got {r}"
        )),
    }
}

fn scale_to_arcsec_per_px(value: f64, units: Option<&str>, width: usize) -> Result<f64, String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("a scale bound must be a number above 0, got {value}"));
    }
    let units = units.map(str::trim).filter(|u| !u.is_empty());
    let per_px = match units {
        None => value,
        Some(u) if u.contains("pix") => value,
        Some(u) if u.eq_ignore_ascii_case(SCALE_UNITS_ARCMIN_WIDTH) => {
            value * ARCSEC_PER_ARCMIN / width.max(1) as f64
        }
        Some(u) if u.eq_ignore_ascii_case(SCALE_UNITS_DEG_WIDTH) => value * ARCSEC_PER_DEG / width.max(1) as f64,
        Some(u) => {
            return Err(format!(
                "unknown scale units {u:?}; use arcsecperpix, {SCALE_UNITS_ARCMIN_WIDTH} or {SCALE_UNITS_DEG_WIDTH}"
            ))
        }
    };
    Ok(per_px)
}

fn panel_centre(request: &PlateSolveRequest) -> Result<Option<(f64, f64)>, String> {
    match (request.center_ra, request.center_dec) {
        (None, None) => Ok(None),
        (Some(ra), Some(dec)) if ra.is_finite() && dec.is_finite() && dec.abs() <= MAX_LATITUDE_DEG => {
            Ok(Some((ra.rem_euclid(360.0), dec)))
        }
        (Some(ra), Some(dec)) => Err(format!(
            "the centre hint ({ra}, {dec}) is not a sky position: RA in degrees and Dec from -90 to 90"
        )),
        _ => Err("the centre hint needs both RA and Dec".to_string()),
    }
}

fn panel_scale(request: &PlateSolveRequest, width: usize) -> Result<Option<(f64, f64)>, String> {
    match (request.scale_lower, request.scale_upper) {
        (None, None) => Ok(None),
        (Some(low), Some(high)) => {
            let units = request.scale_units.as_deref();
            let low = scale_to_arcsec_per_px(low, units, width)?;
            let high = scale_to_arcsec_per_px(high, units, width)?;
            if low >= high {
                return Err(format!("the scale hint {low}..{high} arcsec/px must have low below high"));
            }
            Ok(Some((low, high)))
        }
        _ => Err("the scale hint needs both a lower and an upper bound".to_string()),
    }
}

fn hint_source_text(hints: &SolveHints) -> String {
    let centre = hints.centre.as_ref().map_or("none", |c| c.source.as_str());
    let scale = hints.scale.as_ref().map_or("none", |s| s.source.as_str());
    format!("centre {centre}; scale {scale}")
}

pub(crate) fn with_panel_hints(
    mut hints: SolveHints,
    request: &PlateSolveRequest,
    width: usize,
    height: usize,
) -> Result<(SolveHints, String), String> {
    if let Some((ra, dec)) = panel_centre(request)? {
        let same = hints
            .centre
            .as_ref()
            .is_some_and(|c| angular_separation(c.ra, c.dec, ra, dec) <= SAME_CENTRE_DEG);
        if !same {
            hints.centre = Some(Pointing {
                ra,
                dec,
                source: PANEL_HINT_SOURCE.to_string(),
            });
        }
    }
    if let Some((low, high)) = panel_scale(request, width)? {
        let header_inside = hints
            .scale
            .as_ref()
            .is_some_and(|s| low <= s.low_arcsec && s.high_arcsec <= high);
        if !header_inside {
            hints.scale = Some(ScaleRange {
                low_arcsec: low,
                high_arcsec: high,
                source: PANEL_HINT_SOURCE.to_string(),
            });
            hints.fov_deg = Some(0.5 * (low + high) * width.max(height) as f64 / ARCSEC_PER_DEG);
        }
    }
    let source = hint_source_text(&hints);
    Ok((hints, source))
}

fn field_of(hints: &SolveHints, width: usize, height: usize) -> FieldSize {
    let scale_deg = hints
        .scale
        .as_ref()
        .map_or(0.0, |s| (s.low_arcsec * s.high_arcsec).sqrt() / ARCSEC_PER_DEG);
    FieldSize {
        width_deg: width as f64 * scale_deg,
        height_deg: height as f64 * scale_deg,
    }
}

pub(crate) fn attempt_order(chain: &TierChain, field: FieldSize, expected_tier0_stars: Option<f64>) -> Vec<usize> {
    let Some((chosen, _)) = chain.choose(field, expected_tier0_stars) else {
        return Vec::new();
    };
    let is_tier0 = |tier: CatalogTier| tier == CatalogTier::Tier0;
    let Some(first) = chain
        .tiers
        .iter()
        .position(|(tier, _)| is_tier0(*tier) == is_tier0(chosen))
    else {
        return Vec::new();
    };
    let rest = |tier0: bool| {
        (0..chain.tiers.len()).filter(move |&i| i != first && is_tier0(chain.tiers[i].0) == tier0)
    };
    std::iter::once(first).chain(rest(false)).chain(rest(true)).collect()
}

fn tier2_reach_deg(scale: &ScaleRange, width: usize, height: usize) -> f64 {
    TIER2_NEAR_FIELDS * scale.high_arcsec * width.max(height) as f64 / ARCSEC_PER_DEG
}

fn tier_params(tier: CatalogTier, near_radius_deg: Option<f64>, tier2_reach_deg: f64) -> LocalSolveParams {
    let mut params = LocalSolveParams::default();
    if let Some(radius) = near_radius_deg {
        params.near_radius_deg = radius;
    }
    if tier == CatalogTier::Tier2 {
        params.near_radius_deg = tier2_reach_deg;
    }
    params
}

pub(crate) fn tier2_query(hints: &SolveHints, width: usize, height: usize) -> Option<ConeQuery> {
    let centre = hints.centre.as_ref()?;
    let scale = hints.scale.as_ref()?;
    let half_diagonal_deg = 0.5 * (width as f64).hypot(height as f64) * scale.high_arcsec / ARCSEC_PER_DEG;
    let radius_deg =
        tier2_reach_deg(scale, width, height) + LocalSolveParams::default().footprint_margin * half_diagonal_deg;
    let query = solver_cone_query(centre.ra, centre.dec, radius_deg, TIER2_MIN_ROWS);
    let narrowest_area = (scale.low_arcsec / ARCSEC_PER_DEG).powi(2) * (width * height) as f64;
    let region_area = std::f64::consts::PI * query.radius_deg * query.radius_deg;
    let wanted = TIER2_FIELD_STARS * region_area / narrowest_area;
    let max_rows = if wanted.is_finite() {
        (wanted.ceil() as usize).clamp(TIER2_MIN_ROWS, TIER2_MAX_ROWS)
    } else {
        TIER2_MAX_ROWS
    };
    Some(ConeQuery { max_rows, ..query })
}

struct OfflineTier {
    tier: CatalogTier,
    id: &'static str,
    label: &'static str,
    depth_mag: f64,
    ranking: RankingKey,
}

const OFFLINE_TIERS: [OfflineTier; 3] = [
    OfflineTier {
        tier: CatalogTier::Tier0,
        id: TYCHO2_ID,
        label: TIER0_LABEL,
        depth_mag: TIER0_DEPTH_MAG,
        ranking: RankingKey::MAGNITUDE,
    },
    OfflineTier {
        tier: CatalogTier::Tier1,
        id: SIRIL_GAIA_ID,
        label: TIER1_SIRIL_LABEL,
        depth_mag: TIER1_SIRIL_DEPTH_MAG,
        ranking: RankingKey::MAGNITUDE,
    },
    OfflineTier {
        tier: CatalogTier::Tier1,
        id: TWOMASS_ID,
        label: TIER1_2MASS_LABEL,
        depth_mag: TIER1_2MASS_DEPTH_MAG,
        ranking: RankingKey::TWO_MASS_J_KS,
    },
];

pub(crate) fn offline_tiers(store: &CatalogStore, config: &AppConfig) -> TierSet {
    let mut set = TierSet::default();
    for offline in &OFFLINE_TIERS {
        let installed = matches!(
            store.status(offline.id, config),
            Ok(DatasetStatus::Installed { .. } | DatasetStatus::External { .. })
        );
        if !installed {
            if offline.tier == CatalogTier::Tier0 {
                set.notes.push(format!(
                    "{}: the bundled Tycho-2 catalogue was not found in this installation",
                    offline.label
                ));
            }
            continue;
        }
        match store.open(offline.id, config, PLATE_SOLVE_HOLDER) {
            Ok(lease) => set.push(
                offline.tier,
                offline.label,
                Box::new(StoreProvider {
                    lease,
                    label: offline.label,
                    depth_mag: offline.depth_mag,
                    ranking: offline.ranking,
                }),
            ),
            Err(error) => set.notes.push(format!("{}: skipped, {error}", offline.label)),
        }
    }
    set
}

pub(crate) fn add_online_tier(
    set: &mut TierSet,
    request: &TierRequest<'_>,
    online: Option<GaiaFetch>,
    cancelled: CancelCheck,
) {
    if !request.online {
        return;
    }
    if let (Some(fetch), Some(query)) = (online, tier2_query(request.hints, request.width, request.height)) {
        set.push(
            CatalogTier::Tier2,
            TIER2_LABEL,
            Box::new(GaiaRegionProvider::new(query, fetch, cancelled)),
        );
    }
}

fn missing_hints(hints: &SolveHints) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if hints.centre.is_none() {
        missing.push("centre");
    }
    if hints.scale.is_none() {
        missing.push("scale");
    }
    missing
}

fn fallback_context(env: &dyn SolveEnv) -> serde_json::Map<String, serde_json::Value> {
    let mut context = serde_json::Map::new();
    context.insert("canFallback".into(), json!(env.has_nova()));
    context.insert("apiKeyStored".into(), json!(env.has_nova() && env.stored_api_key().is_some()));
    context
}

fn missing_hint_error(env: &dyn SolveEnv, missing: &[&str], hints: &SolveHints) -> serde_json::Value {
    let mut context = fallback_context(env);
    context.insert("missing".into(), json!(missing));
    context.insert("warnings".into(), json!(hints.warnings));
    coded_error(
        CODE_LOCAL_MISSING_HINT,
        format!(
            "The offline solver needs an approximate {} and none was found in the header or the panel; type it in the hint fields, or try astrometry.net, which can solve without hints",
            missing.join(" and a pixel ")
        ),
        serde_json::Value::Object(context),
    )
}

fn no_catalogue_error(env: &dyn SolveEnv, notes: &[String], expected: f64) -> serde_json::Value {
    let mut context = fallback_context(env);
    context.insert("expectedTier0Stars".into(), json!(expected));
    context.insert("notes".into(), json!(notes));
    coded_error(
        CODE_LOCAL_NO_CATALOGUE,
        format!(
            "No offline star catalogue is available ({}); install one in Settings > Catalogs",
            if notes.is_empty() { "none installed".to_string() } else { notes.join("; ") }
        ),
        serde_json::Value::Object(context),
    )
}

fn no_solution_error(
    env: &dyn SolveEnv,
    tiers: &TierSet,
    tried: &[(usize, SolveFailure)],
    expected: f64,
) -> serde_json::Value {
    let best = tried
        .iter()
        .filter_map(|(index, failure)| failure.best.as_deref().map(|best| (*index, best)))
        .max_by(|(_, a), (_, b)| {
            a.matched
                .cmp(&b.matched)
                .then(b.false_alarm_log10.total_cmp(&a.false_alarm_log10))
        });
    let mut context = fallback_context(env);
    context.insert(
        "best".into(),
        best.map_or(serde_json::Value::Null, |(index, best)| {
            json!({
                "catalog": tiers.labels[index],
                "centre": [best.centre.0, best.centre.1],
                "matched": best.matched,
                "predicted": best.predicted,
                "rmsPx": best.rms_px,
                "falseAlarmLog10": best.false_alarm_log10,
                "coverageLog10": best.coverage_log10,
            })
        }),
    );
    context.insert("expectedTier0Stars".into(), json!(expected));
    context.insert(
        "centresTried".into(),
        json!(tried.iter().map(|(_, f)| f.centres_tried).sum::<usize>()),
    );
    context.insert(
        "detections".into(),
        json!(tried.iter().map(|(_, f)| f.detections).max().unwrap_or(0)),
    );
    context.insert(
        "tiersTried".into(),
        json!(tried
            .iter()
            .map(|(index, failure)| json!({
                "catalog": tiers.labels[*index],
                "tier": tier_number(tiers.chain.tiers[*index].0),
                "message": failure.message,
                "centresTried": failure.centres_tried,
            }))
            .collect::<Vec<_>>()),
    );
    context.insert("notes".into(), json!(tiers.notes));
    let attempts: Vec<String> = tried
        .iter()
        .map(|(index, failure)| format!("{}: {}", tiers.labels[*index], failure.message))
        .collect();
    coded_error(
        CODE_LOCAL_NO_SOLUTION,
        format!("No offline solution. {}", attempts.join("; ")),
        serde_json::Value::Object(context),
    )
}

fn local_result(solution: &LocalSolution, tier: CatalogTier, label: &str, (width, height): (usize, usize)) -> SolveResult {
    let scale = solution.scale_arcsec;
    SolveResult {
        ra_center: solution.centre.0,
        dec_center: solution.centre.1,
        orientation: orientation_east_of_north_deg(solution.tan.cd),
        pixel_scale: scale,
        field_w_arcmin: scale * width as f64 / ARCSEC_PER_ARCMIN,
        field_h_arcmin: scale * height as f64 / ARCSEC_PER_ARCMIN,
        annotations: Vec::new(),
        wcs_cards: solution.wcs_cards.clone(),
        solver: SOLVER_LOCAL.to_string(),
        tier: Some(tier_number(tier)),
        catalog: Some(label.to_string()),
        matched: Some(solution.matched),
        predicted: Some(solution.predicted),
        rms_px: Some(solution.rms_px),
        sip_order: solution.sip_order,
        sip_max_err_px: solution.sip_max_err_px,
        hint_source: None,
        elapsed_ms: None,
        parity: Some(parity_name(solution.parity == ImageParity::Flipped).to_string()),
        warnings: Vec::new(),
    }
}

fn with_search_note(mut failure: SolveFailure, requested_deg: f64, searched_deg: f64) -> SolveFailure {
    if failure.reason == FailureReason::NoSolution && requested_deg > searched_deg {
        failure.message = format!(
            "{}; the online tier searches {searched_deg:.2} deg around the hint, not the requested {requested_deg:.2} deg",
            failure.message
        );
    }
    failure
}

fn solve_locally(
    env: &dyn SolveEnv,
    request: &PlateSolveRequest,
    online: bool,
) -> Result<serde_json::Value, serde_json::Value> {
    let started = Instant::now();
    let near_radius = checked_near_radius(request.near_radius_deg).map_err(plain_error)?;
    env.stage(STAGE_LOAD);
    let loaded = load_plane(&image_ref(&request.path)).map_err(|e| plain_error(format!("{e:#}")))?;
    let (height, width) = loaded.arr.dim();
    let (hints, hint_source) =
        with_panel_hints(solve_hints(&loaded.header, width, height), request, width, height).map_err(plain_error)?;
    let missing = missing_hints(&hints);
    if !missing.is_empty() {
        return Err(missing_hint_error(env, &missing, &hints));
    }
    let tiers = env.tiers(&TierRequest {
        hints: &hints,
        width,
        height,
        online,
    });
    let field = field_of(&hints, width, height);
    let expected = hints
        .expected_tier0_stars
        .unwrap_or(TIER0_MEAN_STARS_PER_DEG2 * field.width_deg * field.height_deg);
    let order = attempt_order(&tiers.chain, field, hints.expected_tier0_stars);
    if order.is_empty() {
        return Err(no_catalogue_error(env, &tiers.notes, expected));
    }
    let requested_radius = near_radius.unwrap_or(LocalSolveParams::default().near_radius_deg);
    let tier2_reach = hints
        .scale
        .as_ref()
        .map_or(0.0, |scale| tier2_reach_deg(scale, width, height));
    let mut tried: Vec<(usize, SolveFailure)> = Vec::new();
    for index in order {
        let (tier, provider) = &tiers.chain.tiers[index];
        let label = tiers.labels[index];
        let params = tier_params(*tier, near_radius, tier2_reach);
        let stage = |text: &str| env.stage(&format!("{label}: {text}"));
        let cancel = || env.cancelled();
        match solve_local(&loaded.arr, &hints, provider.as_ref(), &params, &cancel, &stage) {
            Ok(solution) => {
                let mut warnings = tiers.notes.clone();
                warnings.extend(
                    tried
                        .iter()
                        .map(|(i, failure)| format!("{}: {}", tiers.labels[*i], failure.message)),
                );
                warnings.extend(solution.warnings.iter().cloned());
                let result = SolveResult {
                    hint_source: Some(hint_source),
                    elapsed_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
                    warnings,
                    ..local_result(&solution, *tier, label, (width, height))
                };
                return serde_json::to_value(&result).map_err(|e| plain_error(e.to_string()));
            }
            Err(failure) if failure.reason == FailureReason::Cancelled || env.cancelled() => {
                return Err(cancelled_error())
            }
            Err(failure) if *tier == CatalogTier::Tier2 => {
                tried.push((index, with_search_note(failure, requested_radius, tier2_reach)))
            }
            Err(failure) => tried.push((index, failure)),
        }
    }
    Err(no_solution_error(env, &tiers, &tried, expected))
}

struct AppSolveEnv {
    progress: ProgressHandle,
    resource_dir: Option<PathBuf>,
}

impl SolveEnv for AppSolveEnv {
    fn tiers(&self, request: &TierRequest<'_>) -> TierSet {
        let config = config::load_config().unwrap_or_else(|error| {
            log::warn!("config unavailable ({error:#}); external catalogues are not used for this solve");
            AppConfig::default()
        });
        let mut set = match StoreRoots::from_app(self.resource_dir.as_deref()) {
            Ok(roots) => offline_tiers(&CatalogStore::new(roots), &config),
            Err(error) => TierSet {
                notes: vec![format!("the catalogue folders are not reachable: {error:#}")],
                ..TierSet::default()
            },
        };
        let online: Option<GaiaFetch> = cfg!(feature = "vizier").then(|| {
            Arc::new(|query: &ConeQuery| query_gaia_cached(query).map(|hit| hit.rows.as_ref().clone())) as GaiaFetch
        });
        let progress = self.progress.clone();
        add_online_tier(&mut set, request, online, Arc::new(move || progress.is_cancelled()));
        set
    }

    fn stored_api_key(&self) -> Option<String> {
        resolve_api_key(None)
    }

    fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
        Ok(NovaSettings {
            api_url: config::astrometry_api_url()?,
            timeout_secs: plate_solve_timeout(config::load_config())?,
        })
    }

    fn has_nova(&self) -> bool {
        cfg!(feature = "astrometry-net")
    }

    #[cfg(feature = "astrometry-net")]
    fn nova(&self, upload: NovaUpload) -> Option<NovaFuture> {
        let progress = self.progress.clone();
        Some(Box::pin(async move {
            let cancelled = move || progress.is_cancelled();
            crate::infra::astrometry::plate_solve::solve_astrometry_net(
                &upload.path,
                upload.width,
                upload.height,
                &upload.config,
                &cancelled,
            )
            .await
        }))
    }

    #[cfg(not(feature = "astrometry-net"))]
    fn nova(&self, upload: NovaUpload) -> Option<NovaFuture> {
        drop(upload);
        None
    }

    fn cancelled(&self) -> bool {
        self.progress.is_cancelled()
    }

    fn stage(&self, stage: &str) {
        self.progress.emit_stage(stage);
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn plate_solve_cmd(
    app: tauri::AppHandle,
    path: String,
    api_key: Option<String>,
    scale_lower: Option<f64>,
    scale_upper: Option<f64>,
    scale_units: Option<String>,
    downsample_factor: Option<u32>,
    center_ra: Option<f64>,
    center_dec: Option<f64>,
    radius: Option<f64>,
    solver: Option<String>,
    near_radius_deg: Option<f64>,
) -> Result<serde_json::Value, serde_json::Value> {
    let env: Arc<dyn SolveEnv> = Arc::new(AppSolveEnv {
        progress: ProgressHandle::new(&app, PLATE_SOLVE_PROGRESS_EVENT, 1),
        resource_dir: app.path().resource_dir().ok(),
    });
    let request = PlateSolveRequest {
        path,
        solver,
        api_key,
        scale_lower,
        scale_upper,
        scale_units,
        downsample_factor,
        center_ra,
        center_dec,
        radius,
        near_radius_deg,
    };
    run_plate_solve(env, request).await
}

pub(crate) fn solve_hints_json(hints: &SolveHints) -> serde_json::Value {
    json!({
        "centre": hints.centre.as_ref().map(|c| json!({ "ra": c.ra, "dec": c.dec, "source": c.source })),
        "scale": hints.scale.as_ref().map(|s| json!({
            "low_arcsec": s.low_arcsec,
            "high_arcsec": s.high_arcsec,
            "source": s.source,
        })),
        "fov_deg": hints.fov_deg,
        "epoch_year": hints.epoch_year,
        "cfa": hints.cfa.as_ref().map(|cfa| cfa.name()),
        "object": hints.object.as_ref().map(|o| json!({
            "text": o.text,
            "designation": o.designation,
            "ra": o.ra,
            "dec": o.dec,
        })),
        "expected_tier0_stars": hints.expected_tier0_stars,
        "warnings": hints.warnings,
    })
}

fn header_solve_hints(path: &str) -> anyhow::Result<serde_json::Value> {
    let header = load_plane_header(&image_ref(path))?;
    let (width, height) = image_dims(&header);
    Ok(solve_hints_json(&solve_hints(&header, width, height)))
}

#[tauri::command]
pub async fn solve_hints_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(header_solve_hints(&path))
}

fn rescale_solve_to_original(result: &mut SolveResult, fx: f64, fy: f64, width: usize, height: usize) {
    let f_mean = (fx * fy).sqrt();
    result.pixel_scale /= f_mean;
    result.field_w_arcmin = result.pixel_scale * width as f64 / 60.0;
    result.field_h_arcmin = result.pixel_scale * height as f64 / 60.0;

    for ann in &mut result.annotations {
        ann.pixelx = fx * (ann.pixelx - 0.5) + 0.5;
        ann.pixely = fy * (ann.pixely - 0.5) + 0.5;
        if let Some(r) = ann.radius.as_mut() {
            *r *= f_mean;
        }
    }
}

fn rescale_wcs_cards_to_original(
    cards: &[(String, String)],
    upload_dims: (usize, usize),
    original_dims: (usize, usize),
) -> Vec<(String, String)> {
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key.trim(), value.clone());
    }
    let updates = crate::core::imaging::resample::compute_wcs_updates(
        &header,
        (upload_dims.1, upload_dims.0),
        (original_dims.1, original_dims.0),
    );
    for (key, value) in updates {
        if key.starts_with("NAXIS") {
            continue;
        }
        header.set_f64(&key, value);
    }
    header.cards
}

#[cfg(test)]
pub(crate) fn write_solved_wcs(
    path: &str,
    wcs_cards: &[(String, String)],
    output_dir: &str,
) -> anyhow::Result<serde_json::Value> {
    write_wcs_copy(path, wcs_cards, output_dir, None)
}

#[cfg(test)]
pub(crate) fn write_wcs_copy(
    path: &str,
    wcs_cards: &[(String, String)],
    output_dir: &str,
    gaia_history: Option<&[String]>,
) -> anyhow::Result<serde_json::Value> {
    write_wcs_copy_with_credit(path, wcs_cards, output_dir, gaia_history, &[])
}

struct SolveCredit {
    history: String,
    text: String,
    citation: &'static str,
}

fn solve_credit(catalog: Option<&str>) -> Option<SolveCredit> {
    match catalog.map(str::trim) {
        Some(TIER1_SIRIL_LABEL) => Some(SolveCredit {
            history: format!("Plate solved against the Gaia DR3 astrometric extract from Siril ({TIER1_SIRIL_LABEL}, Zenodo {ZENODO_RECORD_ID})"),
            text: format!("Gaia DR3, ESA/Gaia/DPAC, via the Siril extract (Zenodo {ZENODO_RECORD_ID})"),
            citation: GAIA_DR3_ATTRIBUTION_SIRIL,
        }),
        Some(TIER2_LABEL) => Some(SolveCredit {
            history: format!("Plate solved against Gaia DR3 online ({TIER2_LABEL}, VizieR {GAIA_VIZIER_TABLE})"),
            text: gaia_refine::GAIA_TIE_SHORT_ATTRIBUTION.to_string(),
            citation: GAIA_DR3_ATTRIBUTION,
        }),
        _ => None,
    }
}

pub(crate) fn solve_credit_history(catalog: Option<&str>) -> Vec<String> {
    solve_credit(catalog).map_or_else(Vec::new, |credit| {
        vec![credit.history, credit.citation.to_string(), GAIA_ESA_ACKNOWLEDGEMENT.to_string()]
    })
}

fn solve_credit_value(catalog: Option<&str>) -> serde_json::Value {
    solve_credit(catalog).map_or(serde_json::Value::Null, |credit| json!({ "text": credit.text, "citation": credit.citation }))
}

fn with_solve_credit(mut solved: serde_json::Value) -> serde_json::Value {
    let credit = solve_credit_value(solved["catalog"].as_str());
    if let Some(fields) = solved.as_object_mut() {
        fields.insert("credit".to_string(), credit);
    }
    solved
}

pub(crate) fn write_wcs_copy_with_credit(
    path: &str,
    wcs_cards: &[(String, String)],
    output_dir: &str,
    gaia_history: Option<&[String]>,
    credit: &[String],
) -> anyhow::Result<serde_json::Value> {
    let started = std::time::Instant::now();
    let tie = gaia_history.unwrap_or_default();
    let mut history = tie.to_vec();
    history.extend(credit.iter().filter(|line| !tie.contains(line)).cloned());
    let name = output_stem(path);
    let source_header = cached_header(path)?;
    let planes = source_header.get_i64("NAXIS3").unwrap_or(1);
    if source_header.get_i64("NAXIS").unwrap_or(0) > 2 && planes > 1 {
        anyhow::bail!("{name} has {planes} planes; writing a solved WCS supports single-plane images only");
    }

    let entry = load_cached_full(path)?;
    let source = entry.header().cloned().unwrap_or_else(HduHeader::empty);
    let mut merged = filter_header(&source, false, true).unwrap_or_else(HduHeader::empty);
    if wcs_cards.iter().any(|(key, _)| key.trim() == RADESYS_KEY) {
        merged.remove(DEPRECATED_RADESYS_KEY);
    }
    for (key, value) in wcs_cards {
        let key = key.trim();
        if is_wcs_card(key) {
            merged.set(key, value.clone());
        }
    }
    let abproc = if tie.is_empty() { ABPROC_PLATE_SOLVED } else { gaia_refine::ABPROC_GAIA_TIED };
    let mut header = derived_output_header(Some(&merged), abproc, OutputValues::Linear);
    for line in gaia_refine::history_cards(&history) {
        header.cards.push(("HISTORY".to_string(), line));
    }

    let (rows, cols) = entry.arr().dim();
    let mut probe = header.clone();
    probe.set(HEADER_NAXIS1, cols.to_string());
    probe.set(HEADER_NAXIS2, rows.to_string());
    let wcs = WcsTransform::from_header(&probe).map_err(|e| {
        anyhow::anyhow!("The solved WCS is not a usable celestial WCS ({e:#}); nothing was written.")
    })?;

    let output_dir = resolve_output_dir(output_dir)?;
    let (png_path, fits_path) =
        render_named_and_save(entry.arr(), &output_dir, &format!("{name}_wcs"), true, Some(&header))?;
    let fits_path = fits_path.ok_or_else(|| anyhow::anyhow!("the solved WCS FITS was not written"))?;

    let (cx, cy) = pixel_center(cols, rows);
    let centre = wcs.pixel_to_world(cx, cy);
    let orientation = wcs.orientation(cols, rows);
    Ok(json!({
        RES_FITS_PATH: fits_path,
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        RES_CENTER_RA: centre.ra,
        RES_CENTER_DEC: centre.dec,
        RES_PIXEL_SCALE_ARCSEC: wcs.pixel_scale_arcsec(),
        RES_SIP_PRESENT: orientation.sip_present,
        RES_SIP_MAX_ERR_PX: orientation.sip_max_err_px,
        RES_SIP_INV_ERR_PX: orientation.sip_inv_err_px,
        RES_ELAPSED_MS: started.elapsed().as_millis() as u64,
    }))
}

#[tauri::command]
pub async fn write_solved_wcs_cmd(
    path: String,
    wcs_cards: Vec<(String, String)>,
    output_dir: String,
    gaia_history: Option<Vec<String>>,
    solve_catalog: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(write_wcs_copy_with_credit(
        &path,
        &wcs_cards,
        &output_dir,
        gaia_history.as_deref(),
        &solve_credit_history(solve_catalog.as_deref()),
    ))
}

fn image_dims(header: &crate::types::header::HduHeader) -> (usize, usize) {
    let compressed = header
        .get("ZIMAGE")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("T"));
    let (key1, key2) = if compressed {
        ("ZNAXIS1", "ZNAXIS2")
    } else {
        (HEADER_NAXIS1, HEADER_NAXIS2)
    };
    let dim = |key: &str| {
        header
            .get_i64(key)
            .filter(|n| *n > 0)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    (dim(key1), dim(key2))
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (header, wcs) = load_header_and_wcs(&path)?;
        let (naxis1, naxis2) = image_dims(&header);
        let pixel_scale = wcs.pixel_scale_arcsec();
        let (fov_w, fov_h) = wcs.field_of_view(naxis1, naxis2);
        let (cx, cy) = pixel_center(naxis1, naxis2);
        let center = wcs.pixel_to_world(cx, cy);
        let orientation = wcs.orientation(naxis1, naxis2);
        let gwcs = orientation.gwcs.as_ref();

        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_FOV_ARCMIN: [fov_w, fov_h],
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
            RES_ROTATION_DEG: orientation.rotation_deg,
            RES_FLIPPED: orientation.flipped,
            RES_PARITY: if orientation.flipped { PARITY_FLIPPED } else { PARITY_NORMAL },
            RES_PIXEL_SCALE_X_ARCSEC: orientation.pixel_scale_x_arcsec,
            RES_PIXEL_SCALE_Y_ARCSEC: orientation.pixel_scale_y_arcsec,
            RES_PROJECTION: orientation.projection,
            RES_SIP_PRESENT: orientation.sip_present,
            RES_SIP_MAX_ERR_PX: orientation.sip_max_err_px,
            RES_SIP_INV_ERR_PX: orientation.sip_inv_err_px,
            RES_NORTH_VEC: orientation.north_vec,
            RES_EAST_VEC: orientation.east_vec,
            RES_WCS_KIND: orientation.wcs_kind,
            RES_GWCS_STEPS: gwcs.map(|g| g.n_steps),
            RES_GWCS_FRAMES: gwcs.map(|g| &g.frames),
            RES_GWCS_SOURCE: gwcs.map(|g| &g.source),
            RES_GWCS_VS_SIP_MAX_MAS: gwcs.and_then(|g| g.vs_header_sip_max_mas),
            RES_GWCS_VS_SIP_MAX_PX: gwcs.and_then(|g| g.vs_header_sip_max_px),
            RES_GWCS_REFUSAL: wcs.gwcs_refusal(),
            RES_WCS_WARNING: (wcs.kind() == WcsKind::Header).then(|| tan_pv_warning(&header)).flatten(),
        }))
    })
}

#[tauri::command]
pub async fn pixel_to_world_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    frame: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let wcs = load_wcs_cached(&path)?;
        let coords = wcs.pixel_to_world_bounded_batch(&points);
        let out: Vec<serde_json::Value> = coords
            .into_iter()
            .map(|c| {
                let (lon, lat) = convert_from_icrs(frame, c.ra, c.dec);
                if lon.is_finite() && lat.is_finite() {
                    json!([lon, lat])
                } else {
                    serde_json::Value::Null
                }
            })
            .collect();
        Ok(json!({ RES_POINTS: out, RES_FRAME: frame.name() }))
    })
}

fn pixel_on_image(x: f64, y: f64, naxis1: usize, naxis2: usize) -> bool {
    x >= -0.5 && x < naxis1 as f64 - 0.5 && y >= -0.5 && y < naxis2 as f64 - 0.5
}

fn finite_pair(name: &str, p: (f64, f64)) -> anyhow::Result<()> {
    if !p.0.is_finite() || !p.1.is_finite() {
        anyhow::bail!("point {} must have finite coordinates, got ({}, {})", name, p.0, p.1);
    }
    Ok(())
}

fn latitude_within_poles(name: &str, lat: f64) -> anyhow::Result<()> {
    if lat.is_finite() && lat.abs() > MAX_LATITUDE_DEG {
        anyhow::bail!(
            "{} latitude {} must be within -{} and {} degrees",
            name,
            lat,
            MAX_LATITUDE_DEG,
            MAX_LATITUDE_DEG
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn world_to_pixel_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    frame: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        if points.len() > MAX_SKY_POINTS {
            anyhow::bail!(
                "points holds {} entries, more than the {} allowed per call",
                points.len(),
                MAX_SKY_POINTS
            );
        }
        for (i, &(_, lat)) in points.iter().enumerate() {
            latitude_within_poles(&format!("points[{}]", i), lat)?;
        }
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let cached = load_wcs_with_dims_cached(&path)?;
        let icrs: Vec<(f64, f64)> = points
            .iter()
            .map(|&(lon, lat)| {
                if lon.is_finite() && lat.is_finite() {
                    convert_to_icrs(frame, lon, lat)
                } else {
                    (f64::NAN, f64::NAN)
                }
            })
            .collect();
        let pixels = cached.wcs.world_to_pixel_batch(&icrs);
        let mut out = Vec::with_capacity(pixels.len());
        let mut on_image = Vec::with_capacity(pixels.len());
        for &(x, y) in &pixels {
            if x.is_finite() && y.is_finite() {
                out.push(json!([x, y]));
                on_image.push(pixel_on_image(x, y, cached.naxis1, cached.naxis2));
            } else {
                out.push(serde_json::Value::Null);
                on_image.push(false);
            }
        }
        Ok(json!({
            RES_POINTS: out,
            RES_ON_IMAGE: on_image,
            RES_FRAME: frame.name(),
            RES_NAXIS1: cached.naxis1,
            RES_NAXIS2: cached.naxis2,
        }))
    })
}

#[tauri::command]
pub async fn sky_separation_cmd(
    path: Option<String>,
    a: (f64, f64),
    b: (f64, f64),
    pixel: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        finite_pair("a", a)?;
        finite_pair("b", b)?;
        let (a_sky, b_sky, pixel_length, pixel_scale) = if pixel.unwrap_or(false) {
            let path = match path.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => anyhow::bail!("pixel points need a file path, but path is empty"),
            };
            let wcs = load_wcs_cached(path)?;
            let ca = wcs.pixel_to_world(a.0, a.1);
            let cb = wcs.pixel_to_world(b.0, b.1);
            if !(ca.ra.is_finite() && ca.dec.is_finite() && cb.ra.is_finite() && cb.dec.is_finite()) {
                anyhow::bail!("one of the pixel points does not project onto the sky");
            }
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            ((ca.ra, ca.dec), (cb.ra, cb.dec), json!(length), json!(wcs.pixel_scale_arcsec()))
        } else {
            latitude_within_poles("point a", a.1)?;
            latitude_within_poles("point b", b.1)?;
            (a, b, serde_json::Value::Null, serde_json::Value::Null)
        };
        let separation_deg = angular_separation(a_sky.0, a_sky.1, b_sky.0, b_sky.1);
        let position_angle = position_angle_deg(a_sky.0, a_sky.1, b_sky.0, b_sky.1);
        Ok(json!({
            RES_A_SKY: [a_sky.0, a_sky.1],
            RES_B_SKY: [b_sky.0, b_sky.1],
            RES_SEPARATION_DEG: separation_deg,
            RES_SEPARATION_ARCMIN: separation_deg * 60.0,
            RES_SEPARATION_ARCSEC: separation_deg * 3600.0,
            RES_POSITION_ANGLE_DEG: position_angle,
            RES_PIXEL_LENGTH: pixel_length,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
        }))
    })
}

#[tauri::command]
pub async fn grid_lines_cmd(
    path: String,
    frame: Option<String>,
    density: Option<u8>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let density = density.unwrap_or(DEFAULT_DENSITY).clamp(MIN_DENSITY, MAX_DENSITY);
        match load_grid_cached(&path, frame, density) {
            Ok(grid) => Ok(serde_json::to_value(&*grid)?),
            Err(e) => Ok(json!({ "error": format!("{:#}", e) })),
        }
    })
}

struct SkyFootprint {
    center_ra: f64,
    center_dec: f64,
    corners: [(f64, f64); 4],
    fov_w_arcmin: f64,
    fov_h_arcmin: f64,
}

fn sky_footprint(path: &str) -> anyhow::Result<SkyFootprint> {
    let (header, wcs) = load_header_and_wcs(path)?;
    let (n1, n2) = image_dims(&header);
    if n1 == 0 || n2 == 0 {
        anyhow::bail!("Missing NAXIS1/NAXIS2 in header");
    }
    let mut corners = [(0.0f64, 0.0f64); 4];
    for (i, &(x, y)) in pixel_edge_corners(n1, n2).iter().enumerate() {
        let c = wcs.pixel_to_world(x, y);
        if !c.ra.is_finite() || !c.dec.is_finite() {
            anyhow::bail!("Image corner does not project to a valid sky position");
        }
        corners[i] = (c.ra, c.dec);
    }
    let (cx, cy) = pixel_center(n1, n2);
    let center = wcs.pixel_to_world(cx, cy);
    if !center.ra.is_finite() || !center.dec.is_finite() {
        anyhow::bail!("Image center does not project to a valid sky position");
    }
    let (fov_w, fov_h) = wcs.field_of_view(n1, n2);
    Ok(SkyFootprint {
        center_ra: center.ra,
        center_dec: center.dec,
        corners,
        fov_w_arcmin: fov_w,
        fov_h_arcmin: fov_h,
    })
}

fn wrap180(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

fn footprint_overlap_fraction(a: &SkyFootprint, b: &SkyFootprint) -> f64 {
    let ra0 = a.center_ra;
    let dec0 = a.center_dec;
    let cosd = dec0.to_radians().cos().max(1e-6);

    let bbox = |fp: &SkyFootprint| {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &(ra, dec) in &fp.corners {
            let x = wrap180(ra - ra0) * cosd;
            let y = dec - dec0;
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        (xmin, xmax, ymin, ymax)
    };

    let (ax1, ax2, ay1, ay2) = bbox(a);
    let (bx1, bx2, by1, by2) = bbox(b);

    let ix = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
    let iy = (ay2.min(by2) - ay1.max(by1)).max(0.0);
    let area_a = (ax2 - ax1) * (ay2 - ay1);
    let area_b = (bx2 - bx1) * (by2 - by1);
    let min_area = area_a.min(area_b);
    if min_area <= 0.0 {
        return 0.0;
    }
    (ix * iy) / min_area
}

#[tauri::command]
pub async fn check_pointing_overlap_cmd(
    paths: Vec<String>,
    threshold: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let threshold = threshold.unwrap_or(0.05).clamp(0.0, 1.0);

        let footprints: Vec<(String, Result<SkyFootprint, String>)> = paths
            .iter()
            .map(|p| (p.clone(), sky_footprint(p).map_err(|e| format!("{:#}", e))))
            .collect();

        let files: Vec<serde_json::Value> = footprints
            .iter()
            .map(|(p, r)| match r {
                Ok(fp) => json!({
                    "path": p,
                    "has_wcs": true,
                    "center_ra": fp.center_ra,
                    "center_dec": fp.center_dec,
                    "fov_w_arcmin": fp.fov_w_arcmin,
                    "fov_h_arcmin": fp.fov_h_arcmin,
                }),
                Err(e) => json!({ "path": p, "has_wcs": false, "error": e }),
            })
            .collect();

        let mut pairs = Vec::new();
        let mut any_disjoint = false;
        for i in 0..footprints.len() {
            for j in (i + 1)..footprints.len() {
                match (&footprints[i].1, &footprints[j].1) {
                    (Ok(fa), Ok(fb)) => {
                        let fraction = footprint_overlap_fraction(fa, fb);
                        if fraction < threshold {
                            any_disjoint = true;
                            let sep = angular_separation(
                                fa.center_ra, fa.center_dec,
                                fb.center_ra, fb.center_dec,
                            ) * 60.0;
                            pairs.push(json!({
                                "a": footprints[i].0,
                                "b": footprints[j].0,
                                "status": "disjoint",
                                "fraction": fraction,
                                "separation_arcmin": sep,
                            }));
                        }
                    }
                    _ => {
                        pairs.push(json!({
                            "a": footprints[i].0,
                            "b": footprints[j].0,
                            "status": "unknown",
                            "fraction": serde_json::Value::Null,
                            "separation_arcmin": serde_json::Value::Null,
                        }));
                    }
                }
            }
        }

        Ok(json!({
            "files": files,
            "pairs": pairs,
            "any_disjoint": any_disjoint,
            "threshold": threshold,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        footprint_overlap_fraction, image_dims, load_grid_cached, load_wcs_with_dims_cached, plate_solve_timeout,
        sky_footprint, wrap180, SkyFootprint, PLATE_SOLVE_PROGRESS_EVENT,
    };
    use crate::types::config::AppConfig;
    use crate::core::astrometry::frames::SkyFrame;
    use crate::core::astrometry::grid::{DEFAULT_DENSITY, MAX_DENSITY};
    use crate::core::imaging::region::test_support::{make_header, north_up_cd, wcs_cards};
    use crate::infra::astrometry::plate_solve::test_support::nova_wcs_cards;

    fn write_north_up_fits(path: &str, size: usize) {
        let cards = wcs_cards(north_up_cd());
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let header = make_header(&pairs);
        let arr = ndarray::Array2::<f32>::zeros((size, size));
        crate::infra::fits::writer::write_fits_mono(path, &arr, Some(&header)).unwrap();
    }

    fn north_up_wcs_header() -> crate::types::header::HduHeader {
        let cards = wcs_cards(north_up_cd());
        let pairs: Vec<(&str, &str)> = cards
            .iter()
            .filter(|(k, _)| !k.starts_with("NAXIS"))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        make_header(&pairs)
    }

    #[test]
    fn tile_compressed_images_use_their_decompressed_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rice.fits").to_string_lossy().to_string();
        let arr = ndarray::Array2::<f32>::from_shape_fn((30, 40), |(r, c)| (r * 40 + c) as f32);
        crate::infra::fits::writer::write_fits_mono_rice(&path, &arr, Some(&north_up_wcs_header()), 16, 0.0).unwrap();

        let cached = load_wcs_with_dims_cached(&path).unwrap();
        assert_eq!((cached.naxis1, cached.naxis2), (40, 30));
        let footprint = sky_footprint(&path).unwrap();
        assert!((footprint.fov_w_arcmin - 40.0 / 60.0).abs() < 1e-9, "fov_w {}", footprint.fov_w_arcmin);
        assert!((footprint.fov_h_arcmin - 30.0 / 60.0).abs() < 1e-9, "fov_h {}", footprint.fov_h_arcmin);

        let plain = make_header(&[("NAXIS1", "64"), ("NAXIS2", "32")]);
        assert_eq!(image_dims(&plain), (64, 32));
        let table = make_header(&[("XTENSION", "BINTABLE"), ("ZIMAGE", "T"), ("NAXIS1", "8"), ("NAXIS2", "23"), ("ZNAXIS1", "37"), ("ZNAXIS2", "23")]);
        assert_eq!(image_dims(&table), (37, 23));
        assert_eq!(image_dims(&make_header(&[("NAXIS1", "-4")])), (0, 0));
    }

    #[test]
    fn footprint_centre_and_corners_use_pixel_centres_and_edges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("footprint.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let fp = sky_footprint(&path).unwrap();
        assert!((fp.center_ra - 150.0).abs() < 1e-9 && (fp.center_dec - 2.0).abs() < 1e-9, "({}, {})", fp.center_ra, fp.center_dec);
        let height_arcsec = (fp.corners[2].1 - fp.corners[1].1) * 3600.0;
        assert!((height_arcsec - 100.0).abs() < 1e-4, "footprint spans {height_arcsec} arcsec");
    }

    #[test]
    fn rescale_maps_an_upload_space_solution_to_the_original_frame() {
        use crate::infra::astrometry::plate_solve::{FieldAnnotation, SolveResult};
        let mut result = SolveResult {
            ra_center: 10.0,
            dec_center: 20.0,
            orientation: Some(30.0),
            pixel_scale: 2.0,
            field_w_arcmin: 2.0 * 2048.0 / 60.0,
            field_h_arcmin: 2.0 * 1024.0 / 60.0,
            annotations: vec![FieldAnnotation {
                kind: "ngc".into(),
                names: vec!["NGC 1".into()],
                pixelx: 1024.5,
                pixely: 512.5,
                radius: Some(10.0),
            }],
            wcs_cards: Vec::new(),
            ..SolveResult::default()
        };
        super::rescale_solve_to_original(&mut result, 2.0, 2.0, 4096, 2048);
        assert!((result.pixel_scale - 1.0).abs() < 1e-12);
        assert!((result.field_w_arcmin - 4096.0 / 60.0).abs() < 1e-9);
        assert!((result.field_h_arcmin - 2048.0 / 60.0).abs() < 1e-9);
        let ann = &result.annotations[0];
        assert!((ann.pixelx - 2048.5).abs() < 1e-12 && (ann.pixely - 1024.5).abs() < 1e-12);
        assert_eq!(ann.radius, Some(20.0));
        assert_eq!((result.ra_center, result.dec_center, result.orientation), (10.0, 20.0, Some(30.0)));
    }

    #[test]
    fn rescaled_wcs_cards_reproduce_the_upload_solution_at_the_original_corners() {
        use crate::core::astrometry::wcs::{angular_separation, pixel_center, pixel_edge_corners, WcsTransform};

        let wcs_of = |cards: &[(String, String)], (cols, rows): (usize, usize)| {
            let mut h = crate::types::header::HduHeader::empty();
            for (k, v) in cards {
                h.set(k, v.clone());
            }
            h.set("NAXIS1", cols.to_string());
            h.set("NAXIS2", rows.to_string());
            WcsTransform::from_header(&h).unwrap()
        };
        let centred = |(cols, rows): (usize, usize)| -> Vec<(String, String)> {
            nova_wcs_cards()
                .into_iter()
                .map(|(k, v)| match k.as_str() {
                    "CRPIX1" => (k, format!("{}", cols as f64 / 2.0 + 0.5)),
                    "CRPIX2" => (k, format!("{}", rows as f64 / 2.0 + 0.5)),
                    _ => (k, v),
                })
                .collect()
        };
        let cases = [
            ((1024usize, 683usize), (2048usize, 1366usize), nova_wcs_cards()),
            ((2048, 1365), (6000, 4000), centred((2048, 1365))),
        ];
        for (upload, original, cards) in cases {
            let rescaled = super::rescale_wcs_cards_to_original(&cards, upload, original);
            let upload_wcs = wcs_of(&cards, upload);
            let orig_wcs = wcs_of(&rescaled, original);
            let fx = original.0 as f64 / upload.0 as f64;
            let fy = original.1 as f64 / upload.1 as f64;
            let scale = orig_wcs.pixel_scale_arcsec();
            let mut points: Vec<(f64, f64)> = pixel_edge_corners(original.0, original.1).to_vec();
            points.push(pixel_center(original.0, original.1));
            for (x, y) in points {
                let xu = (x + 0.5) / fx - 0.5;
                let yu = (y + 0.5) / fy - 0.5;
                let up = upload_wcs.pixel_to_world(xu, yu);
                let orig = orig_wcs.pixel_to_world(x, y);
                let sep_px = angular_separation(up.ra, up.dec, orig.ra, orig.dec) * 3600.0 / scale;
                assert!(sep_px < 0.05, "{upload:?} -> {original:?} at ({x}, {y}): {sep_px} px apart");
                let (pu, pv) = upload_wcs.world_to_pixel(up.ra, up.dec);
                let (px, py) = orig_wcs.world_to_pixel(up.ra, up.dec);
                let (ex, ey) = (fx * (pu + 0.5) - 0.5, fy * (pv + 0.5) - 0.5);
                assert!(
                    (px - ex).abs() < 0.05 && (py - ey).abs() < 0.05,
                    "{upload:?} -> {original:?} at ({x}, {y}): ({px}, {py}) vs ({ex}, {ey})"
                );
            }
        }
    }

    fn source_with_cards(
        dir: &tempfile::TempDir,
        name: &str,
        extra: &[(String, String)],
    ) -> (String, ndarray::Array2<f32>) {
        let mut pairs = wcs_cards(north_up_cd());
        pairs.extend(extra.iter().cloned());
        pairs.push(("BUNIT".to_string(), "MJy/sr".to_string()));
        pairs.push(("EXPTIME".to_string(), "300".to_string()));
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let arr = ndarray::Array2::from_shape_fn((48, 64), |(y, x)| (y * 64 + x) as f32 * 0.5 + 10.0);
        let path = dir.path().join(name).to_string_lossy().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &arr, Some(&make_header(&refs))).unwrap();
        (path, arr)
    }

    fn source_with_old_wcs(dir: &tempfile::TempDir, name: &str) -> (String, ndarray::Array2<f32>) {
        source_with_cards(dir, name, &[])
    }

    fn stale_wcs_cards() -> Vec<(String, String)> {
        [
            ("CDELT1", "-2.777777777778E-04"),
            ("CDELT2", "2.777777777778E-04"),
            ("PC1_1", "1.0"),
            ("PC1_2", "0.0"),
            ("PC2_1", "0.0"),
            ("PC2_2", "1.0"),
            ("CROTA2", "12.5"),
            ("A_ORDER", "3"),
            ("A_0_3", "4.0E-11"),
            ("A_3_0", "-7.0E-11"),
            ("B_ORDER", "3"),
            ("B_0_3", "6.0E-11"),
            ("B_3_0", "-2.0E-11"),
            ("PV1_1", "0.0"),
            ("PV1_2", "90.0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn write_solved_wcs_drops_the_stale_wcs_cards_nova_does_not_send() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source_with_cards(&dir, "stale_src.fits", &stale_wcs_cards());
        let out = dir.path().join("out").to_string_lossy().to_string();
        let source = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&path)).unwrap();
        for (key, _) in stale_wcs_cards() {
            assert!(source.get(&key).is_some(), "{key} is missing from the fixture");
        }

        let result = super::write_solved_wcs(&path, &nova_wcs_cards(), &out).unwrap();

        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        let header = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap();
        for key in [
            "CDELT1", "CDELT2", "PC1_1", "PC1_2", "PC2_1", "PC2_2", "CROTA2", "A_0_3", "A_3_0", "B_0_3", "B_3_0",
            "PV1_1", "PV1_2",
        ] {
            assert!(header.get(key).is_none(), "stale {key} survived as {:?}", header.get(key));
        }
        assert_eq!(header.get_i64("A_ORDER"), Some(2));
        assert_eq!(header.get_i64("B_ORDER"), Some(2));

        let mut written: Vec<String> = header
            .cards
            .iter()
            .map(|(k, _)| k.trim().to_string())
            .filter(|k| crate::infra::fits::writer::is_wcs_card(k))
            .collect();
        written.sort();
        written.dedup();
        let mut expected: Vec<String> = nova_wcs_cards().into_iter().map(|(k, _)| k).collect();
        expected.sort();
        assert_eq!(written, expected);

        let mut nova_only = crate::types::header::HduHeader::empty();
        for (k, v) in nova_wcs_cards() {
            nova_only.set(&k, v);
        }
        nova_only.set("NAXIS1", "64".to_string());
        nova_only.set("NAXIS2", "48".to_string());
        let expected_wcs = crate::core::astrometry::wcs::WcsTransform::from_header(&nova_only).unwrap();
        let written_wcs = crate::core::astrometry::wcs::WcsTransform::from_header(&header).unwrap();
        for (x, y) in [(0.0, 0.0), (63.0, 0.0), (0.0, 47.0), (63.0, 47.0), (31.5, 23.5)] {
            let ours = written_wcs.pixel_to_world(x, y);
            let theirs = expected_wcs.pixel_to_world(x, y);
            assert!(
                (ours.ra - theirs.ra).abs() < 1e-9 && (ours.dec - theirs.dec).abs() < 1e-9,
                "({x}, {y}): ({}, {}) vs ({}, {})",
                ours.ra,
                ours.dec,
                theirs.ra,
                theirs.dec
            );
        }
        assert_eq!(result["sip_present"], true);
    }

    #[tokio::test]
    async fn write_solved_wcs_replaces_the_old_wcs_and_keeps_pixels_and_units() {
        let dir = tempfile::tempdir().unwrap();
        let (path, arr) = source_with_old_wcs(&dir, "solved_src.fits");
        let out = dir.path().join("out").to_string_lossy().to_string();

        let result = super::write_solved_wcs(&path, &nova_wcs_cards(), &out).unwrap();

        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        assert!(fits_path.ends_with("solved_src_wcs.fits"), "{fits_path}");
        assert!(std::path::Path::new(&fits_path).exists(), "{fits_path} missing");
        assert!(std::path::Path::new(result["png_path"].as_str().unwrap()).exists(), "{}", result["png_path"]);
        assert_eq!(result["dimensions"], serde_json::json!([64, 48]));

        let header = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap();
        assert_eq!(header.get("CTYPE1").map(str::trim), Some("RA---TAN-SIP"));
        assert_eq!(header.get("CTYPE2").map(str::trim), Some("DEC--TAN-SIP"));
        assert_eq!(header.get_f64("CRVAL1"), Some(83.822));
        assert_eq!(header.get_f64("CRVAL2"), Some(-5.391));
        assert_eq!(header.get_f64("CD1_1"), Some(-5.55e-4));
        assert_eq!(header.get_f64("A_2_0"), Some(1.2e-7));
        assert_eq!(header.get("BUNIT").map(str::trim), Some("MJy/sr"));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(header.get("ABPROC").map(str::trim), Some("platesolved"));
        assert!(header.get("ABDISP").is_none());

        let written = crate::infra::fits::reader::load_fits_image(&fits_path).unwrap();
        assert_eq!(written, arr);

        let info = super::get_wcs_info(fits_path.clone()).await.unwrap();
        for key in ["center_ra", "center_dec", "pixel_scale_arcsec"] {
            let ours = result[key].as_f64().unwrap();
            let theirs = info[key].as_f64().unwrap();
            assert!((ours - theirs).abs() < 1e-9, "{key}: {ours} vs {theirs}");
        }
        let mut expected = crate::types::header::HduHeader::empty();
        for (k, v) in nova_wcs_cards() {
            expected.set(&k, v);
        }
        expected.set("NAXIS1", "64".to_string());
        expected.set("NAXIS2", "48".to_string());
        let centre = crate::core::astrometry::wcs::WcsTransform::from_header(&expected).unwrap().pixel_to_world(31.5, 23.5);
        assert!((result["center_ra"].as_f64().unwrap() - centre.ra).abs() < 1e-9, "{} vs {}", result["center_ra"], centre.ra);
        assert!((result["center_dec"].as_f64().unwrap() - centre.dec).abs() < 1e-9, "{} vs {}", result["center_dec"], centre.dec);
        assert_eq!(result["sip_present"], true);
        assert_eq!(info["sip_present"], true);
        assert!(result["elapsed_ms"].is_u64(), "{}", result["elapsed_ms"]);
    }

    #[test]
    fn write_solved_wcs_refuses_cards_without_a_celestial_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source_with_old_wcs(&dir, "nocrval.fits");
        let cards: Vec<(String, String)> = nova_wcs_cards().into_iter().filter(|(k, _)| k != "CRVAL1").collect();
        let out = dir.path().join("out");

        let err = format!("{:#}", super::write_solved_wcs(&path, &cards, out.to_str().unwrap()).unwrap_err());

        assert!(err.starts_with("The solved WCS is not a usable celestial WCS"), "{err}");
        assert!(err.contains("CRVAL1"), "{err}");
        assert!(!out.join("nocrval_wcs.fits").exists(), "a FITS was written despite the refusal");
        assert!(!out.exists(), "the output dir was created despite the refusal");
    }

    #[test]
    fn write_solved_wcs_refuses_a_multi_plane_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgb_src.fits").to_string_lossy().to_string();
        let plane = ndarray::Array2::<f32>::from_elem((16, 24), 1.0);
        crate::infra::fits::writer::write_fits_rgb_bitpix(&path, &plane, &plane, &plane, None, -32).unwrap();
        let out = dir.path().join("out");

        let err = format!("{:#}", super::write_solved_wcs(&path, &nova_wcs_cards(), out.to_str().unwrap()).unwrap_err());

        assert!(err.contains("rgb_src has 3 planes"), "{err}");
        assert!(err.contains("single-plane images only"), "{err}");
        assert!(!out.exists(), "the output dir was created despite the refusal");
    }

    #[test]
    fn grid_cache_is_keyed_by_path_frame_density_and_file_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grid.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let first = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert_eq!(first.frame, "icrs");
        assert!(!first.lines.is_empty() && !first.labels.is_empty());
        assert!((first.lat_step_deg * 3600.0 - 15.0).abs() < 1e-9, "lat step {}", first.lat_step_deg);

        let again = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &again), "same key must hit the cache");

        let galactic = load_grid_cached(&path, SkyFrame::Galactic, DEFAULT_DENSITY).unwrap();
        assert_eq!(galactic.frame, "galactic");
        assert!(!std::sync::Arc::ptr_eq(&first, &galactic));

        let denser = load_grid_cached(&path, SkyFrame::Icrs, MAX_DENSITY).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &denser));
        assert!(denser.lines.len() > first.lines.len(), "{} vs {}", denser.lines.len(), first.lines.len());

        write_north_up_fits(&path, 200);
        let refreshed = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &refreshed), "a changed file must invalidate the cache");
        assert!(refreshed.lat_step_deg > first.lat_step_deg, "{} vs {}", refreshed.lat_step_deg, first.lat_step_deg);

        let missing = dir.path().join("missing.fits").to_string_lossy().to_string();
        assert!(load_grid_cached(&missing, SkyFrame::Icrs, DEFAULT_DENSITY).is_err());
    }

    fn footprint(ra: f64, dec: f64, size_deg: f64) -> SkyFootprint {
        let half = size_deg / 2.0;
        let cosd = dec.to_radians().cos().max(1e-6);
        let dra = half / cosd;
        SkyFootprint {
            center_ra: ra,
            center_dec: dec,
            corners: [
                (ra - dra, dec - half),
                (ra + dra, dec - half),
                (ra + dra, dec + half),
                (ra - dra, dec + half),
            ],
            fov_w_arcmin: size_deg * 60.0,
            fov_h_arcmin: size_deg * 60.0,
        }
    }

    #[test]
    fn plate_solve_uses_the_configured_timeout_and_refuses_zero() {
        let configured = AppConfig { plate_solve_timeout_secs: 45, ..AppConfig::default() };
        assert_eq!(plate_solve_timeout(Ok(configured)).unwrap(), 45);
        let fallback = plate_solve_timeout(Err(anyhow::anyhow!("unreadable config.json"))).unwrap();
        assert_eq!(fallback, AppConfig::default().plate_solve_timeout_secs);
        let zero = AppConfig { plate_solve_timeout_secs: 0, ..AppConfig::default() };
        let err = plate_solve_timeout(Ok(zero)).unwrap_err().to_string();
        assert!(err.contains("at least 1 second"), "{err}");
    }

    #[test]
    fn plate_solve_cancels_through_the_event_the_frontend_names() {
        assert_eq!(PLATE_SOLVE_PROGRESS_EVENT, "plate-solve-progress");
    }

    #[test]
    fn identical_footprints_fully_overlap() {
        let a = footprint(10.0, 5.0, 0.1);
        let b = footprint(10.0, 5.0, 0.1);
        assert!((footprint_overlap_fraction(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_pointings_have_zero_overlap() {
        let a = footprint(6.95, 1.65, 0.04);
        let b = footprint(7.40, 1.81, 0.04);
        assert_eq!(footprint_overlap_fraction(&a, &b), 0.0);
    }

    #[test]
    fn ra_wraparound_is_handled() {
        let a = footprint(359.95, 0.0, 0.2);
        let b = footprint(0.05, 0.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.4, "expected overlap across RA=0, got {}", f);
        assert!((wrap180(359.8) + 0.2).abs() < 1e-9);
    }

    #[test]
    fn high_declination_neighbors_overlap_partially() {
        let a = footprint(100.0, 80.0, 0.2);
        let b = footprint(100.0 + 0.1 / 80.0f64.to_radians().cos(), 80.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.3 && f < 0.7, "expected partial overlap, got {}", f);
    }

    fn sky_of(v: &serde_json::Value, key: &str) -> (f64, f64) {
        (v[key][0].as_f64().unwrap(), v[key][1].as_f64().unwrap())
    }

    fn pair_at(v: &serde_json::Value, i: usize) -> (f64, f64) {
        (v[i][0].as_f64().unwrap(), v[i][1].as_f64().unwrap())
    }

    fn angle_diff(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[tokio::test]
    async fn world_to_pixel_round_trips_pixel_to_world_and_flags_off_image_points() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let pixels = vec![(12.25, 77.5), (0.0, 0.0), (99.0, 99.0)];
        let sky = super::pixel_to_world_cmd(path.clone(), pixels.clone(), None).await.unwrap();
        let sky_points: Vec<(f64, f64)> = (0..pixels.len()).map(|i| pair_at(&sky["points"], i)).collect();

        let mut points = sky_points.clone();
        points.push((sky_points[0].0 + 10.0, sky_points[0].1));
        points.push((f64::NAN, 2.0));
        let back = super::world_to_pixel_cmd(path, points, Some("icrs".into())).await.unwrap();
        assert_eq!(back["frame"], "icrs");
        assert_eq!(back["naxis1"], 100);
        assert_eq!(back["naxis2"], 100);
        for (i, &(px, py)) in pixels.iter().enumerate() {
            let (x, y) = pair_at(&back["points"], i);
            assert!((x - px).abs() < 1e-6 && (y - py).abs() < 1e-6, "point {i}: ({x}, {y}) vs ({px}, {py})");
            assert_eq!(back["on_image"][i], true);
        }
        assert_eq!(back["on_image"][3], false);
        assert!(back["points"][4].is_null());
        assert_eq!(back["on_image"][4], false);
    }

    #[tokio::test]
    async fn world_to_pixel_rejects_too_many_points_and_unknown_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 10);
        let too_many = vec![(150.0, 2.0); super::MAX_SKY_POINTS + 1];
        let err = super::world_to_pixel_cmd(path.clone(), too_many, None).await.unwrap_err();
        assert!(err.contains("20000"), "{err}");
        let err = super::world_to_pixel_cmd(path, vec![(150.0, 2.0)], Some("supergalactic".into())).await.unwrap_err();
        assert!(err.contains("supergalactic"), "{err}");
    }

    #[tokio::test]
    async fn galactic_input_lands_where_its_icrs_equivalent_lands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("galactic.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);
        let (l, b) = crate::core::astrometry::frames::icrs_to_galactic(150.0, 2.0);
        assert!((l - 236.9546).abs() < 0.01 && (b - 41.9051).abs() < 0.01, "galactic ({l}, {b})");

        let gal = super::world_to_pixel_cmd(path.clone(), vec![(l, b)], Some("galactic".into())).await.unwrap();
        let icrs = super::world_to_pixel_cmd(path, vec![(150.0, 2.0)], None).await.unwrap();
        assert_eq!(gal["frame"], "galactic");
        let (gx, gy) = pair_at(&gal["points"], 0);
        let (ix, iy) = pair_at(&icrs["points"], 0);
        assert!((gx - ix).abs() < 1e-6 && (gy - iy).abs() < 1e-6, "({gx}, {gy}) vs ({ix}, {iy})");
        assert!((ix - 49.5).abs() < 1e-6 && (iy - 49.5).abs() < 1e-6, "centre ({ix}, {iy})");
    }

    #[tokio::test]
    async fn pixel_mode_separation_measures_a_vertical_line_with_north_up_position_angles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sep.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let up = super::sky_separation_cmd(Some(path.clone()), (49.5, 40.0), (49.5, 50.0), Some(true)).await.unwrap();
        assert!((up["separation_arcsec"].as_f64().unwrap() - 10.0).abs() < 1e-6, "{}", up["separation_arcsec"]);
        assert!((up["separation_arcmin"].as_f64().unwrap() - 10.0 / 60.0).abs() < 1e-7);
        assert!((up["separation_deg"].as_f64().unwrap() - 10.0 / 3600.0).abs() < 1e-9);
        let pa_up = up["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_up, 0.0) < 1e-6, "{pa_up}");
        assert!((up["pixel_length"].as_f64().unwrap() - 10.0).abs() < 1e-12);
        assert!((up["pixel_scale_arcsec"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        let (ra, dec) = sky_of(&up, "a_sky");
        assert!((ra - 150.0).abs() < 1e-6 && (dec - 2.0 + 9.5 / 3600.0).abs() < 1e-9, "a_sky ({ra}, {dec})");

        let down = super::sky_separation_cmd(Some(path.clone()), (49.5, 50.0), (49.5, 40.0), Some(true)).await.unwrap();
        let pa_down = down["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_down, 180.0) < 1e-6, "{pa_down}");

        let left = super::sky_separation_cmd(Some(path), (49.5, 49.5), (39.5, 49.5), Some(true)).await.unwrap();
        let pa_left = left["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_left, 90.0) < 1e-6, "{pa_left}");
    }

    #[tokio::test]
    async fn sky_mode_separation_needs_no_path_and_pixel_mode_validates_its_inputs() {
        let sky = super::sky_separation_cmd(None, (10.0, 20.0), (11.0, 21.0), None).await.unwrap();
        assert!((sky["position_angle_deg"].as_f64().unwrap() - 42.9531).abs() < 0.01);
        assert!(sky["pixel_length"].is_null() && sky["pixel_scale_arcsec"].is_null());
        assert_eq!(sky["a_sky"], serde_json::json!([10.0, 20.0]));

        let err = super::sky_separation_cmd(None, (0.0, 0.0), (1.0, 1.0), Some(true)).await.unwrap_err();
        assert!(err.contains("pixel points need a file path"), "{err}");
        let err = super::sky_separation_cmd(None, (f64::NAN, 0.0), (1.0, 1.0), None).await.unwrap_err();
        assert!(err.contains("point a"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("offsky.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 10);
        let err = super::sky_separation_cmd(Some(path), (0.0, 0.0), (f64::MAX, f64::MAX), Some(true)).await.unwrap_err();
        assert!(err.contains("does not project onto the sky"), "{err}");
    }

    fn write_fits_centred_at_dec_88(path: &str) {
        let cards: Vec<(String, String)> = wcs_cards(north_up_cd())
            .into_iter()
            .map(|(k, v)| if k == "CRVAL2" { (k, "88.0".to_string()) } else { (k, v) })
            .collect();
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let arr = ndarray::Array2::<f32>::zeros((100, 100));
        crate::infra::fits::writer::write_fits_mono(path, &arr, Some(&make_header(&pairs))).unwrap();
    }

    #[tokio::test]
    async fn sky_latitudes_beyond_the_poles_are_refused_while_pixel_rows_above_ninety_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("polar.fits").to_string_lossy().to_string();
        write_fits_centred_at_dec_88(&path);

        let err = super::world_to_pixel_cmd(path.clone(), vec![(150.0, 88.0), (150.0, 95.0)], None).await.unwrap_err();
        assert!(err.contains("points[1]") && err.contains("95"), "{err}");
        let err = super::world_to_pixel_cmd(path.clone(), vec![(10.0, -90.5)], Some("galactic".into())).await.unwrap_err();
        assert!(err.contains("points[0]") && err.contains("-90.5"), "{err}");
        let pole = super::world_to_pixel_cmd(path.clone(), vec![(150.0, 90.0), (f64::NAN, 2.0), (150.0, f64::INFINITY)], None).await.unwrap();
        assert!(pole["points"][0].is_array(), "{}", pole["points"][0]);
        assert!(pole["points"][1].is_null() && pole["points"][2].is_null(), "{}", pole["points"]);

        let err = super::sky_separation_cmd(None, (10.0, 95.0), (10.0, 20.0), None).await.unwrap_err();
        assert!(err.contains("point a") && err.contains("95"), "{err}");
        let err = super::sky_separation_cmd(None, (10.0, 20.0), (10.0, -91.0), Some(false)).await.unwrap_err();
        assert!(err.contains("point b") && err.contains("-91"), "{err}");
        let at_pole = super::sky_separation_cmd(None, (10.0, 90.0), (10.0, 20.0), None).await.unwrap();
        assert!((at_pole["separation_deg"].as_f64().unwrap() - 70.0).abs() < 1e-9, "{}", at_pole["separation_deg"]);

        let pixel_rows = super::sky_separation_cmd(Some(path), (49.5, 95.0), (49.5, 40.0), Some(true)).await.unwrap();
        assert!((pixel_rows["pixel_length"].as_f64().unwrap() - 55.0).abs() < 1e-12, "{}", pixel_rows["pixel_length"]);
    }

    #[tokio::test]
    async fn get_wcs_info_reports_the_sip_fit_residual() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sip_residual.fits").to_string_lossy().to_string();
        let mut cards = wcs_cards(north_up_cd());
        cards.push(("SIPMXERR".to_string(), "0.0123".to_string()));
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::infra::fits::writer::write_fits_mono(&path, &ndarray::Array2::<f32>::zeros((100, 100)), Some(&make_header(&pairs))).unwrap();

        let info = super::get_wcs_info(path).await.unwrap();
        assert_eq!(info["sip_max_err_px"].as_f64(), Some(0.0123), "{info}");
        assert_eq!(info.get("sip_inv_err_px"), Some(&serde_json::Value::Null), "{info}");
        assert_eq!(info["sip_present"], false);

        let (source, _) = source_with_old_wcs(&dir, "solved_residual_src.fits");
        let out = dir.path().join("out").to_string_lossy().to_string();
        let mut solver_cards = nova_wcs_cards();
        solver_cards.push(("SIPMXERR".to_string(), "0.0098".to_string()));
        solver_cards.push(("SIPIVERR".to_string(), "0.012".to_string()));
        let result = super::write_solved_wcs(&source, &solver_cards, &out).unwrap();
        assert_eq!(result["sip_max_err_px"].as_f64(), Some(0.0098), "{result}");
        assert_eq!(result["sip_inv_err_px"].as_f64(), Some(0.012), "{result}");
        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        let written = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap();
        assert_eq!(written.get_f64("SIPMXERR"), Some(0.0098));
        assert_eq!(written.get_f64("SIPIVERR"), Some(0.012));
        let info = super::get_wcs_info(fits_path).await.unwrap();
        assert_eq!(info["sip_max_err_px"].as_f64(), Some(0.0098), "{info}");
        assert_eq!(info["sip_inv_err_px"].as_f64(), Some(0.012), "{info}");
    }

    #[tokio::test]
    async fn get_wcs_info_names_the_pv_cards_a_tan_header_does_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tan_pv.fits").to_string_lossy().to_string();
        let mut cards = wcs_cards(north_up_cd());
        cards.extend(
            [("PV1_1", "1.02"), ("PV1_2", "-3.0E-2"), ("PV2_1", "1.0"), ("PV2_4", "6.0E-3")].map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::infra::fits::writer::write_fits_mono(&path, &ndarray::Array2::<f32>::zeros((100, 100)), Some(&make_header(&pairs))).unwrap();

        let info = super::get_wcs_info(path).await.unwrap();
        assert_eq!(info["wcs_warning"], "PV distortion not applied: PV1_1, PV1_2, PV2_4", "{info}");
        assert_eq!(info["projection"], "TAN", "{info}");
        let (nx, ny) = sky_of(&info, "north_vec");
        assert!(nx.abs() < 1e-6 && (ny - 1.0).abs() < 1e-6, "the linear TAN is north up: north ({nx}, {ny})");
        let (ex, ey) = sky_of(&info, "east_vec");
        assert!((ex + 1.0).abs() < 1e-6 && ey.abs() < 1e-6, "east ({ex}, {ey})");

        let plain = dir.path().join("plain.fits").to_string_lossy().to_string();
        write_north_up_fits(&plain, 100);
        let info = super::get_wcs_info(plain).await.unwrap();
        assert_eq!(info.get("wcs_warning"), Some(&serde_json::Value::Null), "{info}");
    }

    #[tokio::test]
    async fn wcs_info_reports_the_orientation_of_a_north_up_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("info.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);
        let info = super::get_wcs_info(path).await.unwrap();
        assert_eq!(info["projection"], "TAN");
        assert_eq!(info["parity"], "normal");
        assert_eq!(info["flipped"], false);
        assert_eq!(info["sip_present"], false);
        assert!(info["rotation_deg"].as_f64().unwrap().abs() < 1e-6);
        assert!((info["pixel_scale_x_arcsec"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        let (nx, ny) = sky_of(&info, "north_vec");
        assert!(nx.abs() < 1e-6 && (ny - 1.0).abs() < 1e-6, "north ({nx}, {ny})");
        let (ex, ey) = sky_of(&info, "east_vec");
        assert!((ex + 1.0).abs() < 1e-6 && ey.abs() < 1e-6, "east ({ex}, {ey})");
    }

    mod plate_solve_routing {
        use std::path::Path;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        use serde_json::Value;
        use sha2::{Digest, Sha256};

        use super::super::{
            add_online_tier, attempt_order, header_solve_hints, offline_tiers, run_plate_solve, tier2_query,
            with_panel_hints, CancelCheck, GaiaFetch, GaiaRegionProvider, NovaFuture, NovaSettings, NovaUpload,
            PlateSolveRequest,
            SolveEnv, TierRequest, TierSet, TIER0_LABEL, TIER1_2MASS_LABEL, TIER1_SIRIL_LABEL, TIER2_LABEL,
        };
        use crate::core::astrometry::catalog::{CatalogRow, ConeQuery, GAIA_MATCH_MAG_LIMIT, MAX_CONE_RADIUS_DEG};
        use crate::core::astrometry::healpix_cat::{write_catalog, CatHeader, CatalogStar, Record};
        use crate::core::astrometry::local_solve::{CatalogProvider, CatalogTier, FieldSize, TierChain};
        use crate::core::astrometry::solve_hints::{Pointing, ScaleRange, SolveHints};
        use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
        use crate::core::imaging::region::test_support::make_header;
        use crate::core::synth::psf::{render_stars, GaussianPsf};
        use crate::core::synth::star_field::Star;
        use crate::infra::catalog_store::{CatalogStore, StoreRoots, SIRIL_GAIA_ID, TYCHO2_FILE, TYCHO2_ID};
        use crate::infra::net::test_server::{serve, Resource, TestServer};
        use crate::types::config::AppConfig;

        const SKY_CENTRE: (f64, f64) = (83.82, -5.39);
        const SKY_FRAME: (usize, usize) = (1024, 768);
        const SKY_SCALE_ARCSEC: f64 = 3.5;
        const SKY_ROTATION_DEG: f64 = 20.0;
        const SKY_PIXEL_UM: f64 = 3.76;
        const SKY_DENSITY_PER_DEG2: f64 = 400.0;
        const SKY_CAP_DEG: f64 = 1.6;
        const SKY_BRIGHT_MAG: f64 = 8.0;
        const SKY_DEPTH_MAG: f64 = 13.0;
        const FLUX_AT_DEPTH: f64 = 900.0;
        const BACKGROUND: f64 = 100.0;
        const NOISE_SIGMA: f64 = 5.0;
        const FWHM_PX: f64 = 3.0;
        const ARCSEC_PER_RADIAN: f64 = 206_264.806;

        struct MemoryProvider {
            rows: Vec<CatalogRow>,
            label: &'static str,
        }

        impl CatalogProvider for MemoryProvider {
            fn cone(
                &self,
                ra_deg: f64,
                dec_deg: f64,
                radius_deg: f64,
                mag_limit: Option<f64>,
            ) -> Result<(Vec<CatalogRow>, &'static str), String> {
                let rows = self
                    .rows
                    .iter()
                    .filter(|row| angular_separation(ra_deg, dec_deg, row.ra, row.dec) <= radius_deg)
                    .filter(|row| mag_limit.is_none_or(|limit| row.g.is_some_and(|mag| mag <= limit)))
                    .cloned()
                    .collect();
                Ok((rows, self.label))
            }

            fn depth_mag(&self) -> f64 {
                SKY_DEPTH_MAG
            }
        }

        struct FakeEnv {
            tiers: Mutex<Option<TierSet>>,
            stored_key: Option<String>,
            nova_url: String,
            nova_calls: AtomicUsize,
            stages: Mutex<Vec<String>>,
            cancel: Arc<AtomicBool>,
        }

        impl FakeEnv {
            fn new(tiers: TierSet, stored_key: Option<&str>, nova: &TestServer) -> Arc<Self> {
                Arc::new(Self {
                    tiers: Mutex::new(Some(tiers)),
                    stored_key: stored_key.map(str::to_string),
                    nova_url: nova.base.clone(),
                    nova_calls: AtomicUsize::new(0),
                    stages: Mutex::new(Vec::new()),
                    cancel: Arc::new(AtomicBool::new(false)),
                })
            }

            fn nova_calls(&self) -> usize {
                self.nova_calls.load(Ordering::SeqCst)
            }
        }

        impl SolveEnv for FakeEnv {
            fn tiers(&self, _request: &TierRequest<'_>) -> TierSet {
                self.tiers.lock().unwrap().take().unwrap_or_default()
            }

            fn stored_api_key(&self) -> Option<String> {
                self.stored_key.clone()
            }

            fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
                Ok(NovaSettings {
                    api_url: self.nova_url.clone(),
                    timeout_secs: 5,
                })
            }

            fn has_nova(&self) -> bool {
                cfg!(feature = "astrometry-net")
            }

            #[cfg(feature = "astrometry-net")]
            fn nova(&self, upload: NovaUpload) -> Option<NovaFuture> {
                self.nova_calls.fetch_add(1, Ordering::SeqCst);
                Some(Box::pin(async move {
                    let never = || false;
                    crate::infra::astrometry::plate_solve::solve_astrometry_net(
                        &upload.path,
                        upload.width,
                        upload.height,
                        &upload.config,
                        &never,
                    )
                    .await
                }))
            }

            #[cfg(not(feature = "astrometry-net"))]
            fn nova(&self, upload: NovaUpload) -> Option<NovaFuture> {
                self.nova_calls.fetch_add(1, Ordering::SeqCst);
                drop(upload);
                None
            }

            fn cancelled(&self) -> bool {
                self.cancel.load(Ordering::SeqCst)
            }

            fn stage(&self, stage: &str) {
                self.stages.lock().unwrap().push(stage.to_string());
            }
        }

        fn catalog_row(index: usize, ra: f64, dec: f64, mag: f64) -> CatalogRow {
            CatalogRow {
                id: format!("syn{index}"),
                ra,
                dec,
                ra_epoch: ra,
                dec_epoch: dec,
                pm_ra_masyr: None,
                pm_dec_masyr: None,
                g: Some(mag),
                bp: None,
                rp: None,
                bp_rp: None,
                parallax_mas: None,
                teff_k: None,
            }
        }

        fn destination((ra, dec): (f64, f64), bearing_deg: f64, distance_deg: f64) -> (f64, f64) {
            let (ra1, dec1) = (ra.to_radians(), dec.to_radians());
            let (bearing, distance) = (bearing_deg.to_radians(), distance_deg.to_radians());
            let dec2 = (dec1.sin() * distance.cos() + dec1.cos() * distance.sin() * bearing.cos()).asin();
            let ra2 = ra1
                + (bearing.sin() * distance.sin() * dec1.cos()).atan2(distance.cos() - dec1.sin() * dec2.sin());
            (ra2.to_degrees().rem_euclid(360.0), dec2.to_degrees())
        }

        fn sky_rows(seed: u64) -> Vec<CatalogRow> {
            let mut rng = StdRng::seed_from_u64(seed);
            let cap_area = 2.0 * std::f64::consts::PI * (1.0 - SKY_CAP_DEG.to_radians().cos())
                * (180.0 / std::f64::consts::PI).powi(2);
            let count = (SKY_DENSITY_PER_DEG2 * cap_area).round() as usize;
            let (bright, faint) = (10f64.powf(0.4 * SKY_BRIGHT_MAG), 10f64.powf(0.4 * SKY_DEPTH_MAG));
            (0..count)
                .map(|index| {
                    let cos_min = SKY_CAP_DEG.to_radians().cos();
                    let cos_theta = cos_min + (1.0 - cos_min) * rng.gen::<f64>();
                    let distance = cos_theta.acos().to_degrees();
                    let (ra, dec) = destination(SKY_CENTRE, 360.0 * rng.gen::<f64>(), distance);
                    let mag = (bright + rng.gen::<f64>() * (faint - bright)).log10() / 0.4;
                    catalog_row(index, ra, dec, mag)
                })
                .collect()
        }

        fn truth_wcs() -> WcsTransform {
            let s = SKY_SCALE_ARCSEC / 3600.0;
            let (sin, cos) = SKY_ROTATION_DEG.to_radians().sin_cos();
            let (w, h) = SKY_FRAME;
            let cards = [
                ("NAXIS1", w.to_string()),
                ("NAXIS2", h.to_string()),
                ("CTYPE1", "RA---TAN".to_string()),
                ("CTYPE2", "DEC--TAN".to_string()),
                ("CRVAL1", SKY_CENTRE.0.to_string()),
                ("CRVAL2", SKY_CENTRE.1.to_string()),
                ("CRPIX1", ((w as f64 + 1.0) / 2.0).to_string()),
                ("CRPIX2", ((h as f64 + 1.0) / 2.0).to_string()),
                ("CD1_1", format!("{:.15e}", -s * cos)),
                ("CD1_2", format!("{:.15e}", s * sin)),
                ("CD2_1", format!("{:.15e}", s * sin)),
                ("CD2_2", format!("{:.15e}", s * cos)),
            ];
            let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (*k, v.as_str())).collect();
            WcsTransform::from_header(&make_header(&pairs)).unwrap()
        }

        fn add_noise(image: &mut ndarray::Array2<f32>, seed: u64) {
            let mut rng = StdRng::seed_from_u64(seed);
            for value in image.iter_mut() {
                let (u1, u2): (f64, f64) = (rng.gen::<f64>().max(1e-12), rng.gen());
                let gauss = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                *value += (BACKGROUND + NOISE_SIGMA * gauss) as f32;
            }
        }

        fn header_without_wcs(pointing: bool) -> crate::types::header::HduHeader {
            let focal_mm = ARCSEC_PER_RADIAN * SKY_PIXEL_UM / 1000.0 / SKY_SCALE_ARCSEC;
            let mut cards = vec![
                ("XPIXSZ".to_string(), SKY_PIXEL_UM.to_string()),
                ("FOCALLEN".to_string(), format!("{focal_mm:.4}")),
                ("DATE-OBS".to_string(), "2026-01-15T22:00:00".to_string()),
            ];
            if pointing {
                cards.push(("RA".to_string(), SKY_CENTRE.0.to_string()));
                cards.push(("DEC".to_string(), SKY_CENTRE.1.to_string()));
            }
            let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            make_header(&pairs)
        }

        fn write_sky_frame(dir: &Path, rows: &[CatalogRow]) -> String {
            let truth = truth_wcs();
            let (w, h) = SKY_FRAME;
            let stars: Vec<Star> = rows
                .iter()
                .filter_map(|row| {
                    let (x, y) = truth.world_to_pixel(row.ra, row.dec);
                    let near = x > -10.0 && y > -10.0 && x < w as f64 + 10.0 && y < h as f64 + 10.0;
                    near.then(|| Star {
                        x,
                        y,
                        z: 0.0,
                        flux: FLUX_AT_DEPTH * 10f64.powf(-0.4 * (row.g.unwrap() - SKY_DEPTH_MAG)),
                        temperature: 5800.0,
                    })
                })
                .collect();
            let mut image = render_stars(&stars, &GaussianPsf::from_fwhm(FWHM_PX), w as u32, h as u32);
            add_noise(&mut image, 7);
            let path = dir.join("sky.fits").to_string_lossy().to_string();
            crate::infra::fits::writer::write_fits_mono(&path, &image, Some(&header_without_wcs(true))).unwrap();
            path
        }

        fn write_noise_frame(dir: &Path, name: &str, pointing: bool) -> String {
            let (w, h) = SKY_FRAME;
            let mut image = ndarray::Array2::<f32>::zeros((h, w));
            add_noise(&mut image, 11);
            let path = dir.join(name).to_string_lossy().to_string();
            crate::infra::fits::writer::write_fits_mono(&path, &image, Some(&header_without_wcs(pointing))).unwrap();
            path
        }

        fn memory_tiers(rows: &[CatalogRow]) -> TierSet {
            let mut set = TierSet::default();
            set.push(
                CatalogTier::Tier0,
                TIER0_LABEL,
                Box::new(MemoryProvider {
                    rows: rows.to_vec(),
                    label: TIER0_LABEL,
                }),
            );
            set
        }

        fn request(path: &str) -> PlateSolveRequest {
            PlateSolveRequest {
                path: path.to_string(),
                ..PlateSolveRequest::default()
            }
        }

        fn code(error: &Value) -> &str {
            error["code"].as_str().unwrap_or("")
        }

        #[tokio::test]
        async fn plate_solve_cmd_routes_local_first_and_never_uploads_without_a_key() {
            let dir = tempfile::tempdir().unwrap();
            let rows = sky_rows(3);
            let sky = write_sky_frame(dir.path(), &rows);
            let nova = serve(Resource::new(b"{}".to_vec()));

            let env = FakeEnv::new(memory_tiers(&rows), None, &nova);
            let solved = run_plate_solve(env.clone(), request(&sky)).await.unwrap();
            assert_eq!(solved["solver"], "local", "{solved}");
            assert_eq!(solved["tier"], 0, "{solved}");
            assert_eq!(solved["catalog"], TIER0_LABEL, "{solved}");
            assert!(solved["matched"].as_u64().unwrap() >= 8, "{solved}");
            assert!(solved["predicted"].as_u64().unwrap() >= solved["matched"].as_u64().unwrap(), "{solved}");
            assert!(solved["rms_px"].as_f64().unwrap() <= 1.0, "{solved}");
            assert_eq!(solved["parity"], "normal", "{solved}");
            let orientation = solved["orientation"].as_f64().unwrap();
            assert!((orientation - SKY_ROTATION_DEG).abs() < 0.05, "orientation {orientation}");
            let scale = solved["pixel_scale_arcsec"].as_f64().unwrap();
            assert!((scale / SKY_SCALE_ARCSEC - 1.0).abs() < 1e-3, "scale {scale}");
            let (ra, dec) = (solved["center_ra"].as_f64().unwrap(), solved["center_dec"].as_f64().unwrap());
            let truth_centre = truth_wcs().pixel_to_world((SKY_FRAME.0 as f64 - 1.0) / 2.0, (SKY_FRAME.1 as f64 - 1.0) / 2.0);
            let off_px = angular_separation(ra, dec, truth_centre.ra, truth_centre.dec) * 3600.0 / SKY_SCALE_ARCSEC;
            assert!(off_px < 0.5, "centre {off_px} px from the truth");
            let hint = solved["hint_source"].as_str().unwrap();
            assert!(hint.contains("RA/DEC") && hint.contains("XPIXSZ"), "{hint}");
            assert!(solved["elapsed_ms"].is_u64(), "{solved}");
            assert!(solved["annotations"].as_array().unwrap().is_empty());
            assert!(env.stages.lock().unwrap().iter().any(|s| s.starts_with(TIER0_LABEL)), "{:?}", env.stages);
            assert_eq!(env.nova_calls(), 0);
            assert!(nova.requests().is_empty(), "{:?}", nova.requests());

            let cards: Vec<(String, String)> = serde_json::from_value(solved["wcs_cards"].clone()).unwrap();
            let out = dir.path().join("out").to_string_lossy().to_string();
            let written = super::super::write_solved_wcs(&sky, &cards, &out).unwrap();
            let info = super::super::get_wcs_info(written["fits_path"].as_str().unwrap().to_string()).await.unwrap();
            let round_trip = angular_separation(ra, dec, info["center_ra"].as_f64().unwrap(), info["center_dec"].as_f64().unwrap()) * 3600.0;
            assert!(round_trip <= 0.01, "Write WCS round trip moved the centre by {round_trip} arcsec");

            let noise = write_noise_frame(dir.path(), "noise.fits", true);
            let keyed = FakeEnv::new(memory_tiers(&rows), Some("stored-key"), &nova);
            let refused = run_plate_solve(keyed.clone(), request(&noise)).await.unwrap_err();
            assert_eq!(code(&refused), "local_no_solution", "{refused}");
            assert_eq!(refused["context"]["canFallback"], cfg!(feature = "astrometry-net"), "{refused}");
            assert_eq!(refused["context"]["tiersTried"][0]["catalog"], TIER0_LABEL, "{refused}");
            assert!(refused["context"]["expectedTier0Stars"].as_f64().unwrap() > 0.0, "{refused}");
            assert!(refused["message"].as_str().unwrap().starts_with("No offline solution"), "{refused}");
            assert_eq!(keyed.nova_calls(), 0, "a local failure fell back to astrometry.net on its own");
            assert!(nova.requests().is_empty(), "{:?}", nova.requests());

            let keyless = FakeEnv::new(TierSet::default(), None, &nova);
            let explicit = PlateSolveRequest {
                solver: Some("astrometry.net".into()),
                ..request(&sky)
            };
            let refused = run_plate_solve(keyless.clone(), explicit.clone()).await.unwrap_err();
            if cfg!(feature = "astrometry-net") {
                assert_eq!(code(&refused), "astrometry_net_no_key", "{refused}");
                let message = refused["message"].as_str().unwrap();
                assert!(message.contains("save it in Settings") && !message.contains("panel"), "{message}");
            } else {
                assert!(refused.as_str().unwrap().contains("astrometry-net"), "{refused}");
            }
            assert_eq!(keyless.nova_calls(), 0);
            assert!(nova.requests().is_empty(), "{:?}", nova.requests());

            let unknown = run_plate_solve(keyless.clone(), PlateSolveRequest { solver: Some("blind".into()), ..request(&sky) })
                .await
                .unwrap_err();
            let unknown = unknown.as_str().unwrap();
            assert!(unknown.contains("\"blind\"") && unknown.contains("auto, local, astrometry.net"), "{unknown}");

            if cfg!(feature = "astrometry-net") {
                let typed = FakeEnv::new(TierSet::default(), None, &nova);
                let with_key = PlateSolveRequest {
                    api_key: Some("typed-key".into()),
                    ..explicit
                };
                let answered = run_plate_solve(typed.clone(), with_key).await.unwrap_err();
                assert_eq!(typed.nova_calls(), 1);
                let seen = nova.requests();
                assert_eq!(seen.first().map(|r| r.target.as_str()), Some("/api/login"), "{seen:?}");
                assert!(answered.as_str().unwrap().contains("login"), "{answered}");
            }
        }

        #[tokio::test]
        async fn a_solve_on_the_siril_gaia_extract_carries_its_credit_and_a_tycho_2_solve_carries_none() {
            let dir = tempfile::tempdir().unwrap();
            let rows = sky_rows(3);
            let sky = write_sky_frame(dir.path(), &rows);
            let nova = serve(Resource::new(b"{}".to_vec()));
            let mut siril = TierSet::default();
            siril.push(
                CatalogTier::Tier1,
                TIER1_SIRIL_LABEL,
                Box::new(MemoryProvider { rows: rows.clone(), label: TIER1_SIRIL_LABEL }),
            );

            let on_siril = run_plate_solve(FakeEnv::new(siril, None, &nova), request(&sky)).await.unwrap();
            let on_tycho = run_plate_solve(FakeEnv::new(memory_tiers(&rows), None, &nova), request(&sky)).await.unwrap();

            assert_eq!(on_siril["catalog"], TIER1_SIRIL_LABEL, "{on_siril}");
            assert_eq!(
                on_siril["credit"],
                serde_json::json!({
                    "text": "Gaia DR3, ESA/Gaia/DPAC, via the Siril extract (Zenodo 14692304)",
                    "citation": crate::core::astrometry::gaia_tie::GAIA_DR3_ATTRIBUTION_SIRIL,
                }),
                "{on_siril}"
            );
            assert_eq!(on_tycho["catalog"], TIER0_LABEL, "{on_tycho}");
            assert!(on_tycho.get("credit").is_some_and(Value::is_null), "{on_tycho}");
            assert_eq!(
                super::super::solve_credit_value(Some(TIER2_LABEL)),
                serde_json::json!({
                    "text": "Gaia DR3, ESA/Gaia/DPAC, via VizieR",
                    "citation": crate::core::astrometry::gaia_tie::GAIA_DR3_ATTRIBUTION,
                })
            );
            for other in [Some(TIER1_2MASS_LABEL), Some(""), None] {
                assert!(super::super::solve_credit_value(other).is_null(), "{other:?}");
            }
        }

        #[tokio::test]
        async fn a_frame_without_pointing_cards_asks_for_a_centre_and_offers_astrometry_net() {
            let dir = tempfile::tempdir().unwrap();
            let blind = write_noise_frame(dir.path(), "blind.fits", false);
            let nova = serve(Resource::new(b"{}".to_vec()));
            let env = FakeEnv::new(memory_tiers(&sky_rows(5)), None, &nova);
            let refused = run_plate_solve(env.clone(), request(&blind)).await.unwrap_err();
            assert_eq!(code(&refused), "local_missing_hint", "{refused}");
            assert_eq!(refused["context"]["missing"], serde_json::json!(["centre"]), "{refused}");
            assert_eq!(refused["context"]["canFallback"], cfg!(feature = "astrometry-net"));
            assert!(refused["message"].as_str().unwrap().contains("centre"), "{refused}");
            assert_eq!(env.nova_calls(), 0);

            let empty = FakeEnv::new(TierSet::default(), None, &nova);
            let sky = write_noise_frame(dir.path(), "pointed.fits", true);
            let refused = run_plate_solve(empty, request(&sky)).await.unwrap_err();
            assert_eq!(code(&refused), "local_no_catalogue", "{refused}");
            assert!(nova.requests().is_empty());

            let bad_radius = PlateSolveRequest {
                near_radius_deg: Some(40.0),
                ..request(&sky)
            };
            let refused = run_plate_solve(FakeEnv::new(TierSet::default(), None, &nova), bad_radius).await.unwrap_err();
            assert!(refused.as_str().unwrap().contains("near-search radius"), "{refused}");
        }

        struct NamedProvider(&'static str);

        impl CatalogProvider for NamedProvider {
            fn cone(&self, _: f64, _: f64, _: f64, _: Option<f64>) -> Result<(Vec<CatalogRow>, &'static str), String> {
                Ok((Vec::new(), self.0))
            }

            fn depth_mag(&self) -> f64 {
                12.0
            }
        }

        fn chain(tiers: &[CatalogTier]) -> TierChain {
            TierChain {
                tiers: tiers
                    .iter()
                    .map(|&tier| (tier, Box::new(NamedProvider("t")) as Box<dyn CatalogProvider>))
                    .collect(),
            }
        }

        #[test]
        fn the_tier_chain_tries_the_chosen_tier_first_then_the_other_tiers_with_tier_0_last() {
            use CatalogTier::{Tier0, Tier1, Tier2};
            let small = FieldSize { width_deg: 0.3, height_deg: 0.2 };
            let wide = FieldSize { width_deg: 3.0, height_deg: 2.0 };
            let full = chain(&[Tier0, Tier1, Tier1, Tier2]);
            assert_eq!(attempt_order(&full, small, None), [1, 2, 3, 0]);
            assert_eq!(attempt_order(&full, wide, None), [0, 1, 2, 3]);
            assert_eq!(attempt_order(&full, small, Some(80.0)), [0, 1, 2, 3]);
            assert_eq!(attempt_order(&chain(&[Tier0, Tier2]), small, None), [1, 0]);
            assert_eq!(attempt_order(&chain(&[Tier0]), small, None), [0]);
            assert_eq!(attempt_order(&chain(&[Tier1]), wide, None), [0]);
            assert!(attempt_order(&chain(&[]), wide, None).is_empty());
        }

        fn hints(centre: (f64, f64), scale: (f64, f64)) -> SolveHints {
            SolveHints {
                centre: Some(Pointing { ra: centre.0, dec: centre.1, source: "RA/DEC".into() }),
                scale: Some(ScaleRange { low_arcsec: scale.0, high_arcsec: scale.1, source: "XPIXSZ 3.76 µm / FOCALLEN 221.6 mm".into() }),
                ..SolveHints::default()
            }
        }

        #[test]
        fn panel_hints_override_the_header_unless_they_only_repeat_or_widen_it() {
            let header = hints((10.0, 20.0), (3.15, 3.85));
            let panel = |ra: Option<f64>, dec: Option<f64>, scale: Option<(f64, f64)>, units: Option<&str>| PlateSolveRequest {
                center_ra: ra,
                center_dec: dec,
                scale_lower: scale.map(|s| s.0),
                scale_upper: scale.map(|s| s.1),
                scale_units: units.map(str::to_string),
                ..PlateSolveRequest::default()
            };
            let (kept, source) = with_panel_hints(header.clone(), &panel(None, None, None, None), 1024, 768).unwrap();
            assert_eq!(kept, header);
            assert_eq!(source, "centre RA/DEC; scale XPIXSZ 3.76 µm / FOCALLEN 221.6 mm");

            let (same, _) = with_panel_hints(header.clone(), &panel(Some(10.000001), Some(20.0), Some((0.1, 10.0)), Some("arcsecperpix")), 1024, 768).unwrap();
            assert_eq!(same, header, "a prefilled centre and the default 0.1-10 range keep the header hints and sources");

            let (typed, source) = with_panel_hints(header.clone(), &panel(Some(371.0), Some(20.0), Some((3.4, 3.6)), None), 1024, 768).unwrap();
            let centre = typed.centre.unwrap();
            assert_eq!((centre.ra, centre.dec, centre.source.as_str()), (11.0, 20.0, "typed in the panel"));
            let scale = typed.scale.unwrap();
            assert_eq!((scale.low_arcsec, scale.high_arcsec, scale.source.as_str()), (3.4, 3.6, "typed in the panel"));
            assert!((typed.fov_deg.unwrap() - 3.5 * 1024.0 / 3600.0).abs() < 1e-12);
            assert_eq!(source, "centre typed in the panel; scale typed in the panel");

            let (width_units, _) = with_panel_hints(header.clone(), &panel(None, None, Some((0.9, 1.1)), Some("degwidth")), 1024, 768).unwrap();
            let scale = width_units.scale.unwrap();
            assert!((scale.low_arcsec - 0.9 * 3600.0 / 1024.0).abs() < 1e-12 && (scale.high_arcsec - 1.1 * 3600.0 / 1024.0).abs() < 1e-12);
            let (arcmin, _) = with_panel_hints(SolveHints::default(), &panel(None, None, Some((50.0, 70.0)), Some("arcminwidth")), 1000, 500).unwrap();
            let scale = arcmin.scale.unwrap();
            assert!((scale.low_arcsec - 3.0).abs() < 1e-12 && (scale.high_arcsec - 4.2).abs() < 1e-12);

            for (bad, needle) in [
                (panel(Some(10.0), None, None, None), "both RA and Dec"),
                (panel(Some(10.0), Some(95.0), None, None), "not a sky position"),
                (panel(None, None, Some((2.0, 1.0)), None), "low below high"),
                (panel(None, None, Some((-1.0, 1.0)), None), "above 0"),
                (panel(None, None, Some((1.0, 2.0)), Some("furlongs")), "\"furlongs\""),
                (PlateSolveRequest { scale_lower: Some(1.0), ..PlateSolveRequest::default() }, "lower and an upper"),
            ] {
                let error = with_panel_hints(header.clone(), &bad, 1024, 768).unwrap_err();
                assert!(error.contains(needle), "{error} should name {needle}");
            }
        }

        #[test]
        fn the_online_tier_fetches_one_region_and_serves_every_solver_cone_from_it() {
            let calls = Arc::new(AtomicUsize::new(0));
            let rows = vec![
                catalog_row(0, 10.0, 20.0, 9.0),
                catalog_row(1, 10.3, 20.0, 15.0),
                catalog_row(2, 12.0, 20.0, 10.0),
            ];
            let counted = Arc::clone(&calls);
            let served = rows.clone();
            let fetch: GaiaFetch = Arc::new(move |query: &ConeQuery| {
                counted.fetch_add(1, Ordering::SeqCst);
                assert_eq!((query.ra, query.dec), (10.0, 20.0));
                Ok(served.clone())
            });
            let query = tier2_query(&hints((10.0, 20.0), (1.0, 1.2)), 2000, 1500).unwrap();
            let provider = GaiaRegionProvider::new(query, fetch, never_cancelled());
            let (near, label) = provider.cone(10.0, 20.0, 0.5, None).unwrap();
            assert_eq!(label, TIER2_LABEL);
            assert_eq!(near.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["syn0", "syn1"]);
            let (bright, _) = provider.cone(10.0, 20.0, 0.5, Some(12.0)).unwrap();
            assert_eq!(bright.len(), 1);
            let (far, _) = provider.cone(12.0, 20.0, 0.1, None).unwrap();
            assert_eq!(far.len(), 1);
            assert_eq!(calls.load(Ordering::SeqCst), 1, "every cone must come from the one region query");

            let failing: GaiaFetch = Arc::new(|_: &ConeQuery| Err("VizieR request failed: connection refused".into()));
            let offline = GaiaRegionProvider::new(
                tier2_query(&hints((10.0, 20.0), (1.0, 1.2)), 2000, 1500).unwrap(),
                failing,
                never_cancelled(),
            );
            let error = offline.cone(10.0, 20.0, 0.5, None).unwrap_err();
            assert!(!error.contains(TIER2_LABEL) && error.contains("connection refused"), "{error}");
        }

        fn never_cancelled() -> CancelCheck {
            Arc::new(|| false)
        }

        #[test]
        fn the_online_region_covers_half_a_field_of_search_plus_the_footprint() {
            let query = tier2_query(&hints((10.0, 20.0), (1.0, 1.2)), 2000, 1500).unwrap();
            let widest = 1.2 * 2000.0 / 3600.0;
            let half_diagonal = 0.5 * 2500.0 * 1.2 / 3600.0;
            let radius = 0.5 * widest + 1.2 * half_diagonal;
            assert!((query.radius_deg - radius).abs() < 1e-9, "{}", query.radius_deg);
            let wanted = 900.0 * std::f64::consts::PI * radius * radius / ((1.0f64 / 3600.0).powi(2) * 3.0e6);
            assert_eq!(query.max_rows, wanted.ceil() as usize);
            assert_eq!(query.mag_limit, Some(GAIA_MATCH_MAG_LIMIT));
            let default_range = tier2_query(&hints((10.0, 20.0), (0.1, 10.0)), 2000, 1500).unwrap();
            assert_eq!((default_range.radius_deg, default_range.max_rows), (MAX_CONE_RADIUS_DEG, 20_000));
            assert!(tier2_query(&SolveHints::default(), 2000, 1500).is_none());

            let mut set = TierSet::default();
            let fetch: GaiaFetch = Arc::new(|_: &ConeQuery| Ok(Vec::new()));
            let solve_hints = hints((10.0, 20.0), (1.0, 1.2));
            let offline_only = TierRequest { hints: &solve_hints, width: 2000, height: 1500, online: false };
            add_online_tier(&mut set, &offline_only, Some(Arc::clone(&fetch)), never_cancelled());
            assert!(set.labels.is_empty(), "solver \"local\" never goes online");
            let auto = TierRequest { online: true, ..offline_only };
            add_online_tier(&mut set, &auto, None, never_cancelled());
            assert!(set.labels.is_empty(), "a build without VizieR has no online tier");
            add_online_tier(&mut set, &auto, Some(fetch), never_cancelled());
            assert_eq!(set.labels, [TIER2_LABEL]);
            assert_eq!(set.chain.tiers[0].0, CatalogTier::Tier2);
        }

        fn online_tiers(fetch: GaiaFetch, cancelled: CancelCheck) -> TierSet {
            let region = hints(SKY_CENTRE, (0.9 * SKY_SCALE_ARCSEC, 1.1 * SKY_SCALE_ARCSEC));
            let query = tier2_query(&region, SKY_FRAME.0, SKY_FRAME.1).unwrap();
            let mut set = TierSet::default();
            set.push(
                CatalogTier::Tier2,
                TIER2_LABEL,
                Box::new(GaiaRegionProvider::new(query, fetch, cancelled)),
            );
            set
        }

        #[tokio::test]
        async fn a_failed_online_attempt_says_it_searched_less_than_the_requested_radius() {
            let dir = tempfile::tempdir().unwrap();
            let sky = write_sky_frame(dir.path(), &sky_rows(3));
            let nova = serve(Resource::new(b"{}".to_vec()));
            let empty: GaiaFetch = Arc::new(|_: &ConeQuery| Ok(Vec::new()));

            let env = FakeEnv::new(online_tiers(Arc::clone(&empty), never_cancelled()), None, &nova);
            let refused = run_plate_solve(env, request(&sky)).await.unwrap_err();
            assert_eq!(code(&refused), "local_no_solution", "{refused}");
            let attempt = refused["context"]["tiersTried"][0]["message"].as_str().unwrap();
            assert!(
                attempt.contains("the online tier searches") && attempt.contains("not the requested 3.00 deg"),
                "{attempt}"
            );
            assert!(refused["message"].as_str().unwrap().contains("not the requested 3.00 deg"), "{refused}");

            let narrow = PlateSolveRequest {
                near_radius_deg: Some(0.2),
                ..request(&sky)
            };
            let env = FakeEnv::new(online_tiers(empty, never_cancelled()), None, &nova);
            let refused = run_plate_solve(env, narrow).await.unwrap_err();
            assert_eq!(code(&refused), "local_no_solution", "{refused}");
            assert!(!refused["message"].as_str().unwrap().contains("not the requested"), "{refused}");
        }

        #[tokio::test]
        async fn a_failed_online_attempt_names_its_tier_once() {
            let dir = tempfile::tempdir().unwrap();
            let sky = write_sky_frame(dir.path(), &sky_rows(3));
            let nova = serve(Resource::new(b"{}".to_vec()));
            let failing: GaiaFetch = Arc::new(|_: &ConeQuery| Err("VizieR request failed: connection refused".into()));
            let env = FakeEnv::new(online_tiers(failing, never_cancelled()), None, &nova);
            let refused = run_plate_solve(env, request(&sky)).await.unwrap_err();
            assert_eq!(code(&refused), "local_no_solution", "{refused}");
            let message = refused["message"].as_str().unwrap();
            assert_eq!(message.matches(TIER2_LABEL).count(), 1, "{message}");
            let attempt = refused["context"]["tiersTried"][0]["message"].as_str().unwrap();
            assert!(!attempt.contains(TIER2_LABEL) && attempt.contains("connection refused"), "{attempt}");
            assert!(!attempt.contains("not the requested"), "a network failure is not a search-radius problem: {attempt}");
        }

        #[tokio::test]
        async fn cancel_interrupts_a_slow_online_fetch() {
            let dir = tempfile::tempdir().unwrap();
            let sky = write_sky_frame(dir.path(), &sky_rows(3));
            let nova = serve(Resource::new(b"{}".to_vec()));
            let env = FakeEnv::new(TierSet::default(), None, &nova);
            let cancel = Arc::clone(&env.cancel);
            let cancel_requested = Arc::new(tokio::sync::Notify::new());
            let fetch_started = Arc::clone(&cancel_requested);
            let (release_fetch, fetch_released) = std::sync::mpsc::channel::<()>();
            let fetch_released = Mutex::new(fetch_released);
            let slow: GaiaFetch = Arc::new(move |_: &ConeQuery| {
                cancel.store(true, Ordering::SeqCst);
                fetch_started.notify_one();
                let _ = fetch_released.lock().unwrap().recv();
                Ok(Vec::new())
            });
            let flag = Arc::clone(&env.cancel);
            *env.tiers.lock().unwrap() = Some(online_tiers(slow, Arc::new(move || flag.load(Ordering::SeqCst))));
            let cancel_timeout = async {
                cancel_requested.notified().await;
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            };
            let result = tokio::select! {
                result = run_plate_solve(env.clone(), request(&sky)) => Some(result),
                _ = cancel_timeout => None,
            };
            drop(release_fetch);
            let refused = result
                .expect("the solve waited more than 1.5s for an online query that was cancelled")
                .unwrap_err();
            assert!(env.cancel.load(Ordering::SeqCst), "the online fetch must request cancellation");
            assert_eq!(code(&refused), "cancelled", "{refused}");
        }

        fn compact_catalogue(level: u8, cat_type: u8, centre: (f64, f64), mag: f64) -> Vec<u8> {
            let pixel = cdshealpix::nested::hash(level, centre.0.to_radians(), centre.1.to_radians()) as u32;
            let (lon, lat) = cdshealpix::nested::center(level, u64::from(pixel));
            let star = CatalogStar::at(lon.to_degrees(), lat.to_degrees(), Some(mag));
            let mut bytes = Vec::new();
            write_catalog(&CatHeader::whole("routing test", level, cat_type), &[(pixel, vec![Record::Star(star)])], &mut bytes).unwrap();
            bytes
        }

        #[test]
        fn offline_tiers_open_the_verified_catalogues_skip_missing_ones_and_name_a_bad_digest() {
            let dir = tempfile::tempdir().unwrap();
            let resources = dir.path().join("resources").join("catalogs");
            std::fs::create_dir_all(&resources).unwrap();
            let tier0 = compact_catalogue(6, 128, (56.75, 24.12), 3.0);
            std::fs::write(resources.join(TYCHO2_FILE), &tier0).unwrap();
            let siril_path = dir.path().join("siril").join("siril_cat_healpix8_astro.dat");
            std::fs::create_dir_all(siril_path.parent().unwrap()).unwrap();
            std::fs::write(&siril_path, compact_catalogue(8, 1, (56.75, 24.12), 9.0)).unwrap();
            let mut config = AppConfig::default();
            config.external_catalogs.insert(SIRIL_GAIA_ID.to_string(), siril_path.to_string_lossy().to_string());
            let roots = StoreRoots::under(&dir.path().join("data"), &dir.path().join("cache")).with_tier0_dir(resources.clone());

            let pinned = CatalogStore::new(roots.clone());
            let set = offline_tiers(&pinned, &config);
            assert_eq!(set.labels, [TIER1_SIRIL_LABEL], "{:?}", set.notes);
            assert_eq!(set.chain.tiers[0].0, CatalogTier::Tier1);
            let actual: String = Sha256::digest(&tier0).iter().map(|b| format!("{b:02x}")).collect();
            let note = set.notes.iter().find(|n| n.starts_with(TIER0_LABEL)).expect("a note for the refused tier 0");
            assert!(note.contains("bf01b058e7c9babe03d38d367c98d6a0dbb0e9785741a9c842d053574fa17076") && note.contains(&actual), "{note}");
            assert!(!set.notes.iter().any(|n| n.contains(TIER1_2MASS_LABEL)), "a missing tier 1 is not a problem: {:?}", set.notes);
            let (rows, label) = set.chain.tiers[0].1.cone(56.75, 24.12, 1.0, None).unwrap();
            assert_eq!((rows.len(), label), (1, TIER1_SIRIL_LABEL));
            drop(set);

            let mut spec = pinned.spec(TYCHO2_ID).unwrap().clone();
            spec.sha256 = Some(Sha256::digest(&tier0).into());
            let verified = CatalogStore::new(roots).with_dataset(spec);
            let set = offline_tiers(&verified, &config);
            assert_eq!(set.labels, [TIER0_LABEL, TIER1_SIRIL_LABEL], "{:?}", set.notes);
            assert!(set.notes.is_empty(), "{:?}", set.notes);
            let (rows, label) = set.chain.tiers[0].1.cone(56.75, 24.12, 1.0, None).unwrap();
            assert_eq!((rows.len(), label), (1, TIER0_LABEL));

            let bare = CatalogStore::new(StoreRoots::under(&dir.path().join("bare"), &dir.path().join("bare-cache")));
            let set = offline_tiers(&bare, &AppConfig::default());
            assert!(set.labels.is_empty());
            assert_eq!(set.notes.len(), 1, "{:?}", set.notes);
            assert!(set.notes[0].contains("bundled Tycho-2"), "{:?}", set.notes);
        }

        #[test]
        fn solve_hints_cmd_reports_each_hint_with_its_source() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_noise_frame(dir.path(), "hinted.fits", true);
            let hints = header_solve_hints(&path).unwrap();
            assert_eq!(hints["centre"]["ra"].as_f64(), Some(SKY_CENTRE.0), "{hints}");
            assert_eq!(hints["centre"]["dec"].as_f64(), Some(SKY_CENTRE.1), "{hints}");
            assert_eq!(hints["centre"]["source"], "RA/DEC", "{hints}");
            let (low, high) = (hints["scale"]["low_arcsec"].as_f64().unwrap(), hints["scale"]["high_arcsec"].as_f64().unwrap());
            assert!(low < SKY_SCALE_ARCSEC && SKY_SCALE_ARCSEC < high, "{hints}");
            assert!(hints["scale"]["source"].as_str().unwrap().contains("XPIXSZ"), "{hints}");
            assert!(hints["fov_deg"].as_f64().unwrap() > 0.9, "{hints}");
            assert!((hints["epoch_year"].as_f64().unwrap() - 2026.04).abs() < 0.01, "{hints}");
            for key in ["cfa", "object", "expected_tier0_stars"] {
                assert_eq!(hints.get(key), Some(&Value::Null), "{key}: {hints}");
            }
            assert_eq!(hints["warnings"], serde_json::json!([]));

            let blind = write_noise_frame(dir.path(), "blind.fits", false);
            let hints = header_solve_hints(&blind).unwrap();
            assert_eq!(hints["centre"], Value::Null, "{hints}");
            assert!(hints["scale"].is_object(), "{hints}");
        }

        struct StoreEnv {
            store: CatalogStore,
        }

        impl SolveEnv for StoreEnv {
            fn tiers(&self, _request: &TierRequest<'_>) -> TierSet {
                offline_tiers(&self.store, &AppConfig::default())
            }

            fn stored_api_key(&self) -> Option<String> {
                None
            }

            fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
                anyhow::bail!("this test never reaches astrometry.net")
            }

            fn has_nova(&self) -> bool {
                false
            }

            fn nova(&self, _upload: NovaUpload) -> Option<NovaFuture> {
                None
            }

            fn cancelled(&self) -> bool {
                false
            }

            fn stage(&self, _stage: &str) {}
        }

        fn wcs_of_cards(cards: &[(String, String)], (width, height): (usize, usize)) -> WcsTransform {
            let mut header = crate::types::header::HduHeader::empty();
            for (key, value) in cards {
                header.set(key, value.clone());
            }
            header.set("NAXIS1", width.to_string());
            header.set("NAXIS2", height.to_string());
            WcsTransform::from_header(&header).unwrap()
        }

        fn fits_in(dir: &Path) -> Vec<std::path::PathBuf> {
            let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "fits")).collect())
                .unwrap_or_default();
            files.sort();
            files
        }

        #[tokio::test]
        #[ignore]
        async fn real_wcs_less_frames_solve_offline_through_the_shipped_tier_chain() {
            let (Some(corpus), Some(tier0)) = (std::env::var_os("ASTROBURST_SOLVE_CORPUS"), std::env::var_os("ASTROBURST_TIER0_PATH")) else {
                eprintln!("skipped: set ASTROBURST_SOLVE_CORPUS and ASTROBURST_TIER0_PATH (ASTROBURST_TIER1_PATH optional)");
                return;
            };
            let corpus = std::path::PathBuf::from(corpus);
            let dir = tempfile::tempdir().unwrap();
            let roots = StoreRoots::under(&dir.path().join("data"), &dir.path().join("cache")).with_tier0_dir(tier0.into());
            if let Some(tier1) = std::env::var_os("ASTROBURST_TIER1_PATH") {
                let installed = dir.path().join("data").join("catalogs").join("2mass-l8-v1").join("2mass-l8-v1.dat");
                std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
                std::fs::copy(&tier1, &installed).unwrap();
            }
            let env = Arc::new(StoreEnv { store: CatalogStore::new(roots) });
            let set = offline_tiers(&env.store, &AppConfig::default());
            println!("tiers {:?}, notes {:?}", set.labels, set.notes);
            assert!(set.labels.contains(&TIER0_LABEL), "the real tier 0 must verify against its pin: {:?}", set.notes);
            drop(set);

            let mut failures = Vec::new();
            let mut solved = 0usize;
            let frames: Vec<(std::path::PathBuf, std::path::PathBuf)> = ["c1", "mosaic-v3"]
                .iter()
                .flat_map(|set| {
                    let base = corpus.join(set);
                    fits_in(&base.join("nowcs")).into_iter().map(move |f| (f.clone(), base.join(f.file_name().unwrap())))
                })
                .collect();
            assert!(!frames.is_empty(), "no WCS-less frames under {}", corpus.display());
            for (frame, truth_path) in &frames {
                let name = frame.file_name().unwrap().to_string_lossy().to_string();
                let path = frame.to_string_lossy().to_string();
                let started = std::time::Instant::now();
                let request = PlateSolveRequest { solver: Some("local".into()), ..request(&path) };
                let result = match run_plate_solve(env.clone(), request).await {
                    Ok(result) => result,
                    Err(error) => {
                        println!("{name}: no solution in {:.2} s: {}", started.elapsed().as_secs_f64(), error["message"]);
                        failures.push(format!("{name}: unsolved"));
                        continue;
                    }
                };
                solved += 1;
                let truth_header = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&truth_path.to_string_lossy())).unwrap();
                let truth = crate::infra::wcs_source::load_wcs(&truth_path.to_string_lossy(), &truth_header).unwrap();
                let (width, height) = super::super::image_dims(&truth_header);
                let cards: Vec<(String, String)> = serde_json::from_value(result["wcs_cards"].clone()).unwrap();
                let solution = wcs_of_cards(&cards, (width, height));
                let scale = truth.pixel_scale_arcsec();
                let mut points: Vec<(f64, f64)> = crate::core::astrometry::wcs::pixel_edge_corners(width, height).to_vec();
                points.push(crate::core::astrometry::wcs::pixel_center(width, height));
                let worst = points
                    .iter()
                    .map(|&(x, y)| {
                        let (a, b) = (truth.pixel_to_world(x, y), solution.pixel_to_world(x, y));
                        angular_separation(a.ra, a.dec, b.ra, b.dec) * 3600.0
                    })
                    .fold(0.0f64, f64::max);
                let tolerance = 2.0f64.max(scale);
                let out = dir.path().join("out").to_string_lossy().to_string();
                let written = super::super::write_solved_wcs(&path, &cards, &out).unwrap();
                let info = super::super::get_wcs_info(written["fits_path"].as_str().unwrap().to_string()).await.unwrap();
                let (ra, dec) = (result["center_ra"].as_f64().unwrap(), result["center_dec"].as_f64().unwrap());
                let round_trip = angular_separation(ra, dec, info["center_ra"].as_f64().unwrap(), info["center_dec"].as_f64().unwrap()) * 3600.0;
                println!(
                    "{name}: {} {}/{} rms {:.3} px sip {} sipmxerr {} worst corner {:.3}\" ({:.3} px) round trip {:.5}\" hint [{}] {:.2} s",
                    result["catalog"].as_str().unwrap_or("?"),
                    result["matched"],
                    result["predicted"],
                    result["rms_px"].as_f64().unwrap_or(f64::NAN),
                    result["sip_order"],
                    result["sip_max_err_px"],
                    worst,
                    worst / scale,
                    round_trip,
                    result["hint_source"].as_str().unwrap_or(""),
                    started.elapsed().as_secs_f64()
                );
                if worst > tolerance {
                    failures.push(format!("{name}: worst corner {worst:.3}\" > {tolerance:.3}\""));
                }
                if round_trip > 0.01 {
                    failures.push(format!("{name}: Write WCS round trip {round_trip}\""));
                }
                if result["sip_max_err_px"].as_f64().is_some_and(|e| e > 0.01) {
                    failures.push(format!("{name}: SIPMXERR {}", result["sip_max_err_px"]));
                }
            }
            println!("{solved} of {} WCS-less frames solved offline", frames.len());
            assert!(failures.is_empty(), "{}", failures.join("\n"));
        }
    }

    mod gwcs {
        use std::path::PathBuf;

        use serde_json::Value;

        use super::write_north_up_fits;
        use crate::cmd::common::image_ref;
        use crate::core::astrometry::gwcs::test_support::{
            assert_close, assert_close_slice, documents_fits_dir, fixture_pipeline, heavy_test_dir, mast_dir, real_data_dir,
            skip_if_absent,
        };
        use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
        use crate::infra::asdf::converter::test_fixtures::nircam_gwcs_with_inline_data;
        use crate::infra::image_source::load_plane_header;
        use crate::infra::wcs_source::gwcs_for_path;

        const GWCS_NULL_KEYS: [&str; 6] = [
            "gwcs_steps",
            "gwcs_frames",
            "gwcs_source",
            "gwcs_vs_header_sip_max_mas",
            "gwcs_vs_header_sip_max_px",
            "gwcs_refusal",
        ];

        fn gwcs_asdf(dir: &tempfile::TempDir) -> String {
            let path = dir.path().join("nircam_gwcs.asdf");
            std::fs::write(&path, nircam_gwcs_with_inline_data()).unwrap();
            path.to_string_lossy().into_owned()
        }

        fn pair(v: &Value) -> [f64; 2] {
            [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
        }

        async fn points_of(path: &str, pixels: &[(f64, f64)]) -> Vec<Option<[f64; 2]>> {
            let out = super::super::pixel_to_world_cmd(path.to_string(), pixels.to_vec(), None).await.unwrap();
            out["points"].as_array().unwrap().iter().map(|p| (!p.is_null()).then(|| pair(p))).collect()
        }

        async fn pixels_of(path: &str, sky: &[(f64, f64)]) -> Vec<Option<[f64; 2]>> {
            let out = super::super::world_to_pixel_cmd(path.to_string(), sky.to_vec(), None).await.unwrap();
            out["points"].as_array().unwrap().iter().map(|p| (!p.is_null()).then(|| pair(p))).collect()
        }

        fn real_file(dir: PathBuf, name: &str) -> Option<String> {
            let path = dir.join(name);
            (!skip_if_absent(&path)).then(|| path.to_string_lossy().into_owned())
        }

        fn header_only_wcs(path: &str) -> WcsTransform {
            WcsTransform::from_header(&load_plane_header(&image_ref(path)).unwrap()).unwrap()
        }

        #[tokio::test]
        async fn get_wcs_info_reports_the_gwcs_mode() {
            let dir = tempfile::tempdir().unwrap();
            let path = gwcs_asdf(&dir);
            let info = super::super::get_wcs_info(path.clone()).await.unwrap();
            assert_eq!(info["wcs_kind"], "gwcs", "{info}");
            assert_eq!(info["gwcs_steps"], 3, "{info}");
            assert_eq!(info["gwcs_frames"], serde_json::json!(["detector", "v2v3", "v2v3vacorr", "world"]), "{info}");
            assert_eq!(info["gwcs_source"], "wcs", "{info}");
            assert_eq!(info["projection"], "gWCS", "{info}");
            assert_eq!(info["sip_present"], false, "{info}");
            assert_eq!(info["gwcs_refusal"], Value::Null, "{info}");
            let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
            let centre = pipeline.forward(1.5, 1.5, false);
            assert_close(info["center_ra"].as_f64().unwrap(), centre[0], 1e-9, "center_ra is the pipeline at the 4x4 image centre");
            assert_close(info["center_dec"].as_f64().unwrap(), centre[1], 1e-9, "center_dec");
            let px = info["gwcs_vs_header_sip_max_px"].as_f64().expect("the synthesised header carries the fitted SIP");
            assert!(px < 0.001, "fitted header SIP vs gWCS: {px} px");
            assert!(info["gwcs_vs_header_sip_max_mas"].as_f64().unwrap() < 0.05, "{info}");
            assert!(info["sip_max_err_px"].as_f64().unwrap() < 1e-6, "SIPMXERR of the fitted header: {info}");

            let plain = dir.path().join("plain_tan.fits").to_string_lossy().to_string();
            write_north_up_fits(&plain, 50);
            let info = super::super::get_wcs_info(plain).await.unwrap();
            assert_eq!(info["wcs_kind"], "header", "{info}");
            assert_eq!(info["projection"], "TAN");
            for key in GWCS_NULL_KEYS {
                assert_eq!(info.get(key), Some(&Value::Null), "{key}: {info}");
            }
        }

        #[tokio::test]
        async fn pixel_to_world_cmd_uses_the_gwcs() {
            let dir = tempfile::tempdir().unwrap();
            let path = gwcs_asdf(&dir);
            let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
            let corner = pipeline.forward(0.0, 0.0, false);
            let got = points_of(&path, &[(0.0, 0.0), (-1.0, 0.0), (3.0, 3.0)]).await;
            assert_close_slice(&got[0].expect("inside"), &corner, 1e-9, "(0, 0) equals pipeline.forward");
            assert_close_slice(&got[0].unwrap(), &[274.7253135310639, -13.875903855344635], 1e-9, "(0, 0) equals the gwcs truth");
            assert!(got[1].is_none(), "the readout applies the bounding box: {got:?}");
            assert_close_slice(&got[2].expect("inside"), &pipeline.forward(3.0, 3.0, false), 1e-9, "(3, 3)");
            let unbounded = super::super::load_wcs_cached(&path).unwrap().pixel_to_world(-1.0, 0.0);
            assert!(unbounded.ra.is_finite() && unbounded.dec.is_finite(), "internal consumers extrapolate: {unbounded:?}");

            let back = pixels_of(&path, &[(corner[0], corner[1])]).await;
            let back = back[0].expect("finite");
            assert_close_slice(&back, &[0.0, 0.0], 1e-6, "world_to_pixel_cmd is the exact gWCS inverse");
            let (hx, hy) = header_only_wcs(&path).world_to_pixel(corner[0], corner[1]);
            let header_err = hx.hypot(hy);
            assert!(header_err > back[0].hypot(back[1]), "the fitted AP/BP inverse ({hx}, {hy}) is less exact than Newton {back:?}");
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_nrca1_cal_info_and_points() {
            let Some(path) = real_file(real_data_dir(), "jw02739001001_02105_00001_nrca1_cal.fits") else { return };
            let info = super::super::get_wcs_info(path.clone()).await.unwrap();
            assert_eq!(info["wcs_kind"], "gwcs", "{info}");
            assert_eq!(info["gwcs_source"], "ASDF HDU 8", "{info}");
            assert_eq!(info["gwcs_steps"], 3, "{info}");
            assert_eq!(info["gwcs_frames"], serde_json::json!(["detector", "v2v3", "v2v3vacorr", "world"]));
            assert_eq!(info["sip_max_err_px"].as_f64(), Some(0.008736956112743353), "{info}");
            assert_eq!(info["sip_inv_err_px"].as_f64(), Some(0.008801265065873224), "{info}");
            assert_close(info["gwcs_vs_header_sip_max_mas"].as_f64().unwrap(), 0.2662, 0.005, "9x9 header SIP residual");
            assert_eq!(info["gwcs_refusal"], Value::Null);
            assert_eq!(info["projection"], "gWCS");
            assert_close_slice(
                &[info["center_ra"].as_f64().unwrap(), info["center_dec"].as_f64().unwrap()],
                &[274.7348047011612, -13.867263013140171],
                1e-9,
                "centre (1023.5, 1023.5)",
            );
            let expected = [
                [274.7253135310639, -13.875903855344635],
                [274.73891366393576, -13.871908419682919],
                [274.7348093260082, -13.867258830394992],
                [274.7442624159364, -13.858765184824815],
                [274.72589718810605, -13.858073848486038],
            ];
            let got = points_of(&path, &[(0.0, 0.0), (500.0, 1500.0), (1024.0, 1024.0), (2047.0, 2047.0), (2047.0, 0.0)]).await;
            for (g, e) in got.iter().zip(expected) {
                assert_close_slice(&g.expect("finite"), &e, 1e-9, "nrca1 CDP point");
            }
            println!("nrca1 cal: {} 9x9 max {} mas", info["gwcs_source"], info["gwcs_vs_header_sip_max_mas"]);
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_mirimage_cal_points_and_exact_inverse() {
            let Some(path) = real_file(real_data_dir(), "jw02739002001_02101_00001_mirimage_cal.fits") else { return };
            let got = points_of(&path, &[(0.0, 0.0), (515.0, 511.0)]).await;
            assert_close_slice(&got[0].unwrap(), &[274.7271082813119, -13.841811218259549], 1e-9, "mirimage (0, 0)");
            assert_close_slice(&got[1].unwrap(), &[274.74503682314577, -13.828111050526784], 1e-9, "mirimage (515, 511)");
            let back = pixels_of(&path, &[(274.7271082813119, -13.841811218259549)]).await;
            assert_close_slice(&back[0].unwrap(), &[0.0, 0.0], 1e-6, "exact inverse (header AP/BP: 0.014 px, analytic: 0.11 px)");
            let (hx, hy) = header_only_wcs(&path).world_to_pixel(274.7271082813119, -13.841811218259549);
            println!("mirimage (0,0) inverse: newton {:?}, header AP/BP ({hx}, {hy})", back[0]);
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_roman_cal_info_points_inverse_and_grid() {
            let Some(path) = real_file(real_data_dir(), "r9999901001001001001_0001_wfi01_f129_cal.asdf") else { return };
            let info = super::super::get_wcs_info(path.clone()).await.unwrap();
            assert_eq!(info["wcs_kind"], "gwcs", "{info}");
            assert_eq!(info["gwcs_source"], "roman.meta.wcs", "{info}");
            assert_eq!(info["gwcs_steps"], 3);
            assert_eq!(info["projection"], "gWCS");
            assert_eq!(info["gwcs_refusal"], Value::Null);
            assert_close(info["pixel_scale_arcsec"].as_f64().unwrap(), 0.1093, 0.0005, "mean of the anisotropic surrogate");
            assert_close(info["pixel_scale_x_arcsec"].as_f64().unwrap(), 0.1103, 0.0005, "x scale");
            assert_close(info["pixel_scale_y_arcsec"].as_f64().unwrap(), 0.1083, 0.0005, "y scale");
            assert!(info["gwcs_vs_header_sip_max_px"].as_f64().unwrap() < 0.001, "fitted degree-5 header vs gWCS: {info}");
            let expected = [
                [72.49379934026463, -30.725719347990214],
                [72.45811479570528, -30.665770933732336],
                [72.42090111505738, -30.66436058048687],
                [72.34786329185872, -30.602393450898834],
                [72.34851066377523, -30.72522315122843],
                [72.48302693211511, -30.608447954482045],
            ];
            let pixels = [(0.0, 0.0), (1000.0, 2000.0), (2044.0, 2044.0), (4087.0, 4087.0), (4087.0, 0.0), (300.0, 3900.0)];
            let got = points_of(&path, &pixels).await;
            for (g, e) in got.iter().zip(expected) {
                assert_close_slice(&g.expect("finite"), &e, 1e-9, "roman CDP point");
            }
            let far = super::super::world_to_pixel_cmd(path.clone(), vec![(72.83404951829527, -31.252367086918532)], None).await.unwrap();
            let margin = crate::core::astrometry::wcs::GWCS_NEWTON_SKIP_MARGIN_PX;
            if let Some(p) = far["points"][0].as_array() {
                let [x, y] = pair(&far["points"][0]);
                assert!(x < -0.5 - margin || x > 4087.5 + margin || y < -0.5 - margin || y > 4087.5 + margin, "ghost at ({x}, {y}): {p:?}");
                assert_eq!(far["on_image"][0], false);
            }
            let grid = super::super::grid_lines_cmd(path.clone(), Some("icrs".into()), None).await.unwrap();
            assert!(grid.get("error").is_none(), "{grid}");
            let lines = grid["lines"].as_array().unwrap();
            let lon = lines.iter().filter(|l| l["kind"] == "lon").count();
            let lat = lines.iter().filter(|l| l["kind"] == "lat").count();
            assert!(lon >= 2 && lat >= 2, "lon {lon} lat {lat}");
            println!("roman cal: {} lon + {} lat grid lines, scale {}", lon, lat, info["pixel_scale_arcsec"]);
        }

        fn i2d_files(dir: &PathBuf) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(dir) else {
                eprintln!("skipped: {} absent", dir.display());
                return Vec::new();
            };
            let mut files: Vec<String> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("_i2d.fits")))
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            files.sort();
            files
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_i2d_products_stay_in_header_mode() {
            let heavy = i2d_files(&heavy_test_dir());
            let mast = i2d_files(&mast_dir());
            if !heavy.is_empty() {
                assert_eq!(heavy.len(), 6, "{heavy:?}");
            }
            if !mast.is_empty() {
                assert_eq!(mast.len(), 22, "{mast:?}");
            }
            let files: Vec<String> = heavy.into_iter().chain(mast).collect();
            if files.is_empty() {
                return;
            }
            for path in &files {
                let lookup = gwcs_for_path(path);
                assert!(matches!(lookup, Ok(None)), "{path}: {:?}", lookup.map(|s| s.map(|_| ())));
                let info = super::super::get_wcs_info(path.clone()).await.unwrap();
                assert_eq!(info["wcs_kind"], "header", "{path}: {info}");
                for key in GWCS_NULL_KEYS {
                    assert_eq!(info.get(key), Some(&Value::Null), "{path} {key}: {info}");
                }
                let header_only = header_only_wcs(path);
                let (n1, n2) = (info["naxis1"].as_u64().unwrap() as f64, info["naxis2"].as_u64().unwrap() as f64);
                let corners = [(0.0, 0.0), (n1 - 1.0, 0.0), (0.0, n2 - 1.0), (n1 - 1.0, n2 - 1.0)];
                let got = points_of(path, &corners).await;
                for (g, (x, y)) in got.iter().zip(corners) {
                    let h = header_only.pixel_to_world(x, y);
                    assert_close_slice(&g.expect("finite"), &[h.ra, h.dec], 1e-12, &format!("{path} corner ({x}, {y})"));
                }
            }
            println!("{} i2d products in header mode", files.len());
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_rate_and_s3d_keep_their_header_result() {
            if let Some(s3d) = real_file(documents_fits_dir(), "jw01266005001_02103_00001_nrs1_s3d.fits") {
                assert!(matches!(gwcs_for_path(&s3d), Ok(None)));
                let err = super::super::get_wcs_info(s3d.clone()).await.unwrap_err();
                assert!(err.contains("3D cube"), "a cube is not a plane for get_wcs_info, before and after: {err}");
                let file = std::fs::File::open(&s3d).unwrap();
                let header = crate::infra::fits::reader::extract_header_by_index(&file, 1).unwrap();
                let wcs = crate::infra::wcs_source::load_wcs(&s3d, &header).unwrap();
                assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (crate::core::astrometry::wcs::WcsKind::Header, None));
                let (a, b) = (wcs.pixel_to_world(10.0, 12.0), WcsTransform::from_header(&header).unwrap().pixel_to_world(10.0, 12.0));
                assert_close_slice(&[a.ra, a.dec], &[b.ra, b.dec], 1e-12, "s3d header TAN through load_wcs");
            }
            if let Some(rate) = real_file(documents_fits_dir(), "jw01266005001_02103_00001_nrs1_rate.fits") {
                assert!(matches!(gwcs_for_path(&rate), Ok(None)));
                let err = super::super::get_wcs_info(rate.clone()).await.unwrap_err();
                let header = load_plane_header(&image_ref(&rate)).unwrap();
                let before = WcsTransform::from_header(&header).unwrap_err().to_string();
                assert!(err.ends_with(&before), "the rate SCI (no CRPIX, no celestial CTYPE) refuses as before: {err} vs {before}");
            }
        }

        #[tokio::test]
        #[ignore]
        async fn gwcs_real_crf_reference_exposure_uses_its_gwcs() {
            let Some(path) = real_file(mast_dir(), "jw01475001001_02101_00001_nrca1_o001_crf.fits") else { return };
            let info = super::super::get_wcs_info(path.clone()).await.unwrap();
            assert_eq!(info["wcs_kind"], "gwcs", "{info}");
            assert!(info["gwcs_source"].as_str().unwrap().starts_with("ASDF HDU "), "{info}");
            assert_eq!(info["gwcs_steps"], 3);
            let mas = info["gwcs_vs_header_sip_max_mas"].as_f64().unwrap();
            assert!(mas < 0.01, "cal_ver 2.0.1 degree-5 header SIP vs gWCS: {mas} mas");
            let header_only = header_only_wcs(&path);
            let got = points_of(&path, &[(0.0, 0.0)]).await;
            let h = header_only.pixel_to_world(0.0, 0.0);
            let sep_mas = angular_separation(got[0].unwrap()[0], got[0].unwrap()[1], h.ra, h.dec) * 3.6e6;
            assert!(sep_mas <= mas + 1e-9, "corner separation {sep_mas} mas within the 9x9 maximum {mas}");
        }
    }
}
