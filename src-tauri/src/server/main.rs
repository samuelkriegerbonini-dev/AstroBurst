// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
mod app;
mod handlers;
mod http_layer;
mod science;
mod v2;

use app::{config, job, router, session, state};
use http_layer::{error, extractors};

#[cfg(test)]
mod api_docs;
#[cfg(test)]
use api_docs::route_docs;
#[cfg(test)]
use app::tests;

use std::sync::Arc;

use config::ServerConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Arc::new(ServerConfig::from_env());

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &cfg.log_level);
    }
    env_logger::init();
    astroburst_lib::core::astrometry::catalog_disk::init_from_env();
    astroburst_lib::core::astrometry::catalog::init_vizier_url_from_env();
    #[cfg(feature = "mast")]
    astroburst_lib::infra::mast::settings::init_from_env();

    log::info!(
        "AstroBurst Headless Server v{}",
        env!("CARGO_PKG_VERSION")
    );
    log::info!("Listening on {}", cfg.bind);
    log::info!(
        "Session TTL: {}s, Max sessions: {}",
        cfg.session_ttl.as_secs(),
        cfg.session_max
    );
    log::info!(
        "Per-session cache: {} entries / {} MiB",
        cfg.cache_max_entries,
        cfg.cache_max_bytes / (1024 * 1024)
    );
    log::info!("Cleanup interval: {}s", cfg.cleanup_interval.as_secs());

    let state = state::AppState::new(Arc::clone(&cfg));

    session::SessionManager::start_ttl_cleaner(
        state.sessions.clone(),
        Arc::clone(&cfg),
    );

    let app = router::build_router(state);
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
