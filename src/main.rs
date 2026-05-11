use std::path::Path;

use axum::routing::{get, post};
use axum::Router;
use eyre::WrapErr;
use tracing_subscriber::EnvFilter;

mod api;
mod config;
mod db;
mod docker;
mod error;
mod jira;
mod state;
mod webhooks;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::var("AUTODEV_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let config = config::AppConfig::load(Path::new(&config_path))
        .with_context(|| "loading configuration")?;

    tokio::fs::create_dir_all(&config.server.storage_path)
        .await
        .with_context(|| "creating storage directory")?;

    let db_url = format!(
        "sqlite:{}/autodev.db",
        config.server.storage_path.display()
    );
    let db = db::Db::new(&db_url).await?;

    let state = state::AppState {
        config: config.clone(),
        db,
    };

    let app = Router::new()
        .route("/webhook/jira", post(webhooks::jira::handle_jira_webhook))
        .route("/tasks", get(api::list_tasks))
        .route("/tasks/{id}", get(api::get_task))
        .route("/health", get(api::health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .with_context(|| format!("binding to {}", config.server.bind))?;

    tracing::info!(addr = %config.server.bind, "autodev server starting");

    axum::serve(listener, app)
        .await
        .with_context(|| "server error")?;

    Ok(())
}
