use axum::extract::State;
use axum::http::HeaderMap;
use eyre::WrapErr;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::state::AppState;
use crate::error::AppError;
use crate::webhooks::types::JiraWebhookPayload;

type HmacSha256 = Hmac<Sha256>;

pub async fn handle_jira_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<JiraWebhookPayload>,
) -> Result<axum::http::StatusCode, AppError> {
    let body = serde_json::to_string(&payload).unwrap_or_default();
    verify_webhook_signature(&headers, &body, &state.config.server.webhook_secret)?;

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

    if let Some(existing) = state.db.find_active_task_by_jira_key(&issue.key).await? {
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
            &issue.key,
            &issue.fields.summary,
            issue.fields.description.as_deref(),
            repo_url,
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

fn verify_webhook_signature(
    headers: &HeaderMap,
    body: &str,
    secret: &str,
) -> Result<(), AppError> {
    let signature = headers
        .get("x-hub-signature-256")
        .or_else(|| headers.get("x-hub-signature"))
        .ok_or(AppError::WebhookVerification)?
        .to_str()
        .with_context(|| "reading webhook signature header")
        .map_err(AppError::from)?;

    let signature = signature
        .strip_prefix("sha256=")
        .unwrap_or(signature);

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body.as_bytes());

    let result = mac.finalize();
    let computed = hex::encode(result.into_bytes());

    if !timing_safe_eq(&computed, signature) {
        return Err(AppError::WebhookVerification);
    }

    Ok(())
}

fn timing_safe_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0, |acc, (x, y)| acc | (x ^ y))
        == 0
}
