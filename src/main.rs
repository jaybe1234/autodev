use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};
use eyre::WrapErr;
use tracing_subscriber::EnvFilter;

mod api;
mod config;
mod db;
mod docker;
mod error;
mod figma_mcp;
mod github;
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

    let db_url = format!("sqlite:{}/autodev.db", config.server.storage_path.display());
    let db = db::Db::new(&db_url).await?;

    let github_client = github::GitHubClient::new(&config.github.token);
    let github_username = github_client
        .get_authenticated_user()
        .await
        .with_context(|| "fetching GitHub authenticated user (check your token)")?;

    tracing::info!(github_user = %github_username, "authenticated with GitHub");

    let mut figma_mcp_process = if let Some(ref figma_config) = config.figma {
        match figma_mcp::FigmaMcpProcess::start(figma_config).await {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!(error = %e, "failed to start figma MCP server, continuing without it");
                None
            }
        }
    } else {
        None
    };

    let (review_notify_tx, mut review_notify_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let state = state::AppState {
        config: config.clone(),
        db,
        github_username,
        review_queue: Arc::new(Mutex::new(Default::default())),
        review_notify: review_notify_tx,
    };

    let queue_state = state.clone();
    tokio::spawn(async move {
        while let Some(queue_key) = review_notify_rx.recv().await {
            docker::process_next_queued_review(&queue_state, &queue_key).await;
        }
    });

    let app = Router::new()
        .route("/webhooks/jira", post(webhooks::jira::handle_jira_webhook))
        .route(
            "/webhooks/github",
            post(webhooks::github::handle_github_webhook),
        )
        .route("/tasks", get(api::list_tasks))
        .route("/tasks/{id}", get(api::get_task))
        .route("/health", get(api::health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .with_context(|| format!("binding to {}", config.server.bind))?;

    tracing::info!(addr = %config.server.bind, "autodev server starting");

    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
        tracing::info!("received shutdown signal");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .with_context(|| "server error")?;

    if let Some(ref mut proc) = figma_mcp_process {
        if let Err(e) = proc.shutdown().await {
            tracing::error!(error = %e, "error shutting down figma MCP process");
        }
    }

    Ok(())
}
