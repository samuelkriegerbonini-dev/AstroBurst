// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
mod config;
mod error;
mod extractors;
mod handlers;
mod job;
mod router;
mod session;
mod state;
mod v2;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use config::ServerConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Arc::new(ServerConfig::from_env());

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &cfg.log_level);
    }
    env_logger::init();

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
