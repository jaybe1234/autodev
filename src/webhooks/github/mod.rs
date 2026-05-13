mod types;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;

use crate::error::AppError;
use crate::state::AppState;
use crate::webhooks::crypto::verify_webhook_signature;
use types::{IssueCommentEvent, PullRequestReviewEvent};

pub async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::http::StatusCode, AppError> {
    verify_webhook_signature(&headers, &body, &state.config.github.webhook_secret)?;

    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    match event {
        "pull_request_review" => handle_pull_request_review(&state, &body).await,
        "issue_comment" => handle_issue_comment(&state, &body).await,
        _ => {
            tracing::debug!(event = %event, "ignoring unhandled GitHub event");
            Ok(axum::http::StatusCode::OK)
        }
    }
}

async fn handle_pull_request_review(
    state: &AppState,
    body: &[u8],
) -> Result<axum::http::StatusCode, AppError> {
    let payload: PullRequestReviewEvent = serde_json::from_slice(body)
        .map_err(|e| AppError::Internal(eyre::eyre!("parsing pull_request_review payload: {e}")))?;

    if payload.action != "submitted" {
        tracing::debug!(action = %payload.action, "ignoring non-submitted review action");
        return Ok(axum::http::StatusCode::OK);
    }

    if payload.review.state == "approved" {
        tracing::debug!(pr = %payload.pull_request.number, "ignoring approved review");
        return Ok(axum::http::StatusCode::OK);
    }

    let reviewer = &payload.review.user.login;
    let pr_opener = &payload.pull_request.user.login;
    if reviewer == pr_opener {
        tracing::debug!(reviewer = %reviewer, "ignoring review from PR opener");
        return Ok(axum::http::StatusCode::OK);
    }

    let pr_number = payload.pull_request.number;
    let pr_repo = &payload.repository.full_name;
    let branch_name = &payload.pull_request.head.ref_name;
    let review_body = payload
        .review
        .body
        .as_deref()
        .unwrap_or("(no review body)");
    let review_state = &payload.review.state;

    tracing::info!(
        pr_repo = %pr_repo,
        pr_number = pr_number,
        reviewer = %reviewer,
        state = %review_state,
        "received pull request review"
    );

    let original_task = state
        .db
        .find_original_task_by_pr(pr_repo, pr_number)
        .await?
        .ok_or_else(|| AppError::NoOriginalTask(pr_repo.clone(), pr_number))?;

    spawn_or_queue_review(
        state,
        &original_task,
        &ReviewInfo {
            pr_repo: pr_repo.clone(),
            pr_number,
            branch_name: branch_name.to_string(),
            reviewer_login: reviewer.to_string(),
            review_body: review_body.to_string(),
            review_state: review_state.to_string(),
            source: "github_review".to_string(),
        },
    )
    .await
}

async fn handle_issue_comment(
    state: &AppState,
    body: &[u8],
) -> Result<axum::http::StatusCode, AppError> {
    let payload: IssueCommentEvent = serde_json::from_slice(body)
        .map_err(|e| AppError::Internal(eyre::eyre!("parsing issue_comment payload: {e}")))?;

    if payload.action != "created" {
        return Ok(axum::http::StatusCode::OK);
    }

    if payload.issue.pull_request.is_none() {
        tracing::debug!("ignoring comment on non-PR issue");
        return Ok(axum::http::StatusCode::OK);
    }

    let commenter = &payload.comment.user.login;
    if commenter == &state.github_username {
        tracing::debug!(commenter = %commenter, "ignoring comment from bot");
        return Ok(axum::http::StatusCode::OK);
    }

    let mention = format!("@{}", state.github_username);
    if !payload.comment.body.contains(&mention) {
        tracing::debug!(commenter = %commenter, "ignoring comment without bot mention");
        return Ok(axum::http::StatusCode::OK);
    }

    let pr_number = payload.issue.number;
    let pr_repo = &payload.repository.full_name;

    tracing::info!(
        pr_repo = %pr_repo,
        pr_number = pr_number,
        commenter = %commenter,
        "received issue comment mentioning bot"
    );

    let original_task = state
        .db
        .find_original_task_by_pr(pr_repo, pr_number)
        .await?
        .ok_or_else(|| AppError::NoOriginalTask(pr_repo.clone(), pr_number))?;

    let branch_name = original_task
        .jira_key
        .as_deref()
        .map(|k| format!("autodev/{k}"))
        .unwrap_or_else(|| "main".to_string());

    spawn_or_queue_review(
        state,
        &original_task,
        &ReviewInfo {
            pr_repo: pr_repo.clone(),
            pr_number,
            branch_name,
            reviewer_login: commenter.to_string(),
            review_body: payload.comment.body.clone(),
            review_state: "commented".to_string(),
            source: "github_comment".to_string(),
        },
    )
    .await
}

struct ReviewInfo {
    pr_repo: String,
    pr_number: i64,
    branch_name: String,
    reviewer_login: String,
    review_body: String,
    review_state: String,
    source: String,
}

async fn spawn_or_queue_review(
    state: &AppState,
    original_task: &crate::db::Task,
    info: &ReviewInfo,
) -> Result<axum::http::StatusCode, AppError> {
    let active_review = state
        .db
        .find_active_review_for_task(&original_task.id)
        .await?;

    if active_review.is_some() {
        tracing::info!(
            pr_repo = %info.pr_repo,
            pr_number = info.pr_number,
            reviewer = %info.reviewer_login,
            "active review exists, queuing"
        );

        let session_id = original_task
            .session_id
            .clone()
            .ok_or_else(|| AppError::SessionNotFound(original_task.id.clone()))?;

        let queued = crate::state::QueuedReview {
            pr_repo: info.pr_repo.clone(),
            pr_number: info.pr_number,
            branch_name: info.branch_name.clone(),
            reviewer_login: info.reviewer_login.clone(),
            review_body: info.review_body.clone(),
            review_state: info.review_state.clone(),
            source: info.source.clone(),
            parent_task_id: original_task.id.clone(),
            session_id,
            repo_url: original_task.repo_url.clone(),
        };

        let queue_key = format!("{}#{}", info.pr_repo, info.pr_number);
        let mut queue = state.review_queue.lock().expect("review queue lock poisoned");
        queue
            .entry(queue_key)
            .or_default()
            .push_back(queued);

        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let summary = format!(
        "Review response to @{} on PR #{}",
        info.reviewer_login, info.pr_number
    );

    let review_task = state
        .db
        .insert_task(
            None,
            &summary,
            Some(&info.review_body),
            &original_task.repo_url,
            &info.source,
            Some(&original_task.id),
        )
        .await?;

    tracing::info!(
        task_id = %review_task.id,
        parent_task_id = %original_task.id,
        reviewer = %info.reviewer_login,
        "created review task, spawning review agent"
    );

    let params = crate::docker::ReviewParams {
        branch_name: info.branch_name.clone(),
        reviewer_login: info.reviewer_login.clone(),
        review_body: info.review_body.clone(),
        review_state: info.review_state.clone(),
        pr_repo: info.pr_repo.clone(),
        pr_number: info.pr_number,
    };

    crate::docker::spawn_review_agent(state, &review_task, original_task, &params).await?;

    Ok(axum::http::StatusCode::ACCEPTED)
}
