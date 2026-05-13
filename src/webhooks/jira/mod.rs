mod types;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use eyre::WrapErr;

use crate::error::AppError;
use crate::state::AppState;
use crate::webhooks::crypto::verify_webhook_signature;
use types::JiraWebhookPayload;

pub async fn handle_jira_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::http::StatusCode, AppError> {
    verify_webhook_signature(&headers, &body, &state.config.server.webhook_secret)?;

    let payload: JiraWebhookPayload = serde_json::from_slice(&body)
        .with_context(|| "parsing Jira webhook payload")
        .map_err(AppError::from)?;

    let issue = payload
        .issue
        .ok_or_else(|| AppError::Internal(eyre::eyre!("no issue in webhook payload")))?;

    let is_transition_to_ready = payload
        .changelog
        .as_ref()
        .map(|cl| {
            cl.items.iter().any(|item| {
                item.field == "status"
                    && item
                        .to_string
                        .eq_ignore_ascii_case(
                            state
                                .config
                                .jira
                                .ready_to_dev_status
                                .as_deref()
                                .unwrap_or("ready-to-dev"),
                        )
            })
        })
        .unwrap_or(false);

    if !is_transition_to_ready {
        tracing::debug!(
            jira_key = %issue.key,
            event = ?payload.webhook_event,
            "ignoring non-transition or non-ready-to-dev event"
        );
        return Ok(axum::http::StatusCode::OK);
    }

    let labels = &issue.fields.labels;
    let repo_url = labels
        .iter()
        .find_map(|label| state.config.find_repo_for_label(label))
        .ok_or(AppError::NoMatchingRepo)?;

    if let Some(existing) = state
        .db
        .find_active_task_by_jira_key(&issue.key)
        .await?
    {
        tracing::warn!(
            jira_key = %issue.key,
            task_id = %existing.id,
            "ticket already has an active task, skipping"
        );
        return Err(AppError::DuplicateTask);
    }

    let task = state
        .db
        .insert_task(
            Some(&issue.key),
            &issue.fields.summary,
            issue.fields.description.as_deref(),
            repo_url,
            "jira",
            None,
        )
        .await?;

    tracing::info!(
        task_id = %task.id,
        jira_key = %issue.key,
        repo = repo_url,
        "created task, spawning agent container"
    );

    crate::docker::spawn_agent(&state.config, &state.db, &task).await?;

    Ok(axum::http::StatusCode::ACCEPTED)
}
