use std::path::Path;

use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    BuildImageOptionsBuilder, CreateContainerOptionsBuilder, LogsOptions,
    RemoveContainerOptionsBuilder, WaitContainerOptions,
};
use bollard::{body_full, Docker};
use eyre::WrapErr;
use futures_util::StreamExt;

use crate::config::AppConfig;
use crate::db::{Db, Task};
use crate::error::AppError;
use crate::jira::JiraClient;
use crate::state::AppState;

const AGENT_IMAGE: &str = "autodev-agent";
const PR_URL_PATTERN: &str = r"https://github\.com/([^/]+/[^/]+)/pull/(\d+)";
const SESSION_ID_PATTERN: &str = r"OPENCODE_SESSION_ID=([a-f0-9-]+)";

struct ContainerResult {
    pr_url: Option<String>,
    pr_repo: Option<String>,
    pr_number: Option<i64>,
    session_id: Option<String>,
}

pub async fn spawn_agent(
    config: &AppConfig,
    db: &Db,
    task: &Task,
) -> Result<String, AppError> {
    let docker = Docker::connect_with_local_defaults()
        .with_context(|| "connecting to Docker daemon")
        .map_err(AppError::from)?;

    ensure_agent_image(&docker).await?;

    let session_dir = config
        .server
        .storage_path
        .join("sessions")
        .join(&task.id);
    tokio::fs::create_dir_all(&session_dir)
        .await
        .with_context(|| "creating session directory")
        .map_err(AppError::from)?;

    let prompt = build_prompt(task);
    let opencode_config_content = build_opencode_config(config);
    let repo_url_with_token =
        inject_token_in_repo_url(&task.repo_url, &config.github.token);
    let jira_key = task.jira_key.as_deref().ok_or_else(|| {
        AppError::Internal(eyre::eyre!("jira_key missing for implementation task"))
    })?;
    let branch_name = format!("autodev/{jira_key}");

    let env = vec![
        format!("REPO_URL={}", repo_url_with_token),
        format!("JIRA_KEY={}", jira_key),
        format!("BRANCH_NAME={}", branch_name),
        format!("OPENCODE_PROMPT={}", prompt),
        format!(
            "OPENCODE_CONFIG_CONTENT={}",
            opencode_config_content
        ),
        format!("GITHUB_TOKEN={}", config.github.token),
        format!("OPENCODE_MODEL={}", config.opencode.model),
    ];

    let container_id =
        create_and_start_container(&docker, &task.id, &env, &session_dir).await?;

    db.update_task_status(&task.id, "running", Some(&container_id), None, None)
        .await?;

    let task_id = task.id.clone();
    let jira_key_owned = jira_key.to_string();
    let jira_config = config.jira.clone();
    let db_clone = db.clone();
    let container_id_clone = container_id.clone();

    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "failed to connect to Docker in monitor task");
                return;
            }
        };

        let result = wait_for_container_and_parse_result(&docker, &container_id_clone).await;

        match result {
            Ok(container_result) => {
                let (status, error): (&str, Option<String>) =
                    match &container_result.pr_url {
                        Some(_) => ("done", None),
                        None => (
                            "failed",
                            Some("container exited but no PR URL found in logs".into()),
                        ),
                    };

                if let Err(e) = db_clone
                    .update_task_status(
                        &task_id,
                        status,
                        None,
                        container_result.pr_url.as_deref(),
                        error.as_deref(),
                    )
                    .await
                {
                    tracing::error!(error = %e, "failed to update task status");
                }

                if let (Some(pr_url), Some(pr_repo), Some(pr_number), Some(session_id)) = (
                    &container_result.pr_url,
                    &container_result.pr_repo,
                    &container_result.pr_number,
                    &container_result.session_id,
                ) && let Err(e) = db_clone
                    .update_task_pr_info(&task_id, pr_url, pr_repo, *pr_number, session_id)
                    .await
                {
                    tracing::error!(error = %e, "failed to update task PR info");
                }

                let jira_client =
                    JiraClient::new(&jira_config.base_url, &jira_config.pat);
                match status {
                    "done" => {
                        if let Some(ref pr) = container_result.pr_url {
                            let comment = format!(
                                "Autodev has implemented this ticket and opened a PR: {}",
                                pr
                            );
                            if let Err(e) =
                                jira_client.add_comment(&jira_key_owned, &comment).await
                            {
                                tracing::error!(error = %e, "failed to add Jira comment");
                            }
                        }
                        if let Err(e) = jira_client
                            .transition_issue(
                                &jira_key_owned,
                                &jira_config.transition_to,
                            )
                            .await
                        {
                            tracing::error!(error = %e, "failed to transition Jira issue");
                        }
                    }
                    "failed" => {
                        let comment = format!(
                            "Autodev failed to implement this ticket. Error: {}",
                            error.unwrap_or_default()
                        );
                        if let Err(e) = jira_client
                            .add_comment(&jira_key_owned, &comment)
                            .await
                        {
                            tracing::error!(error = %e, "failed to add failure comment to Jira");
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                if let Err(e) = db_clone
                    .update_task_status(&task_id, "failed", None, None, Some(&format!("{e}")))
                    .await
                {
                    tracing::error!(error = %e, "failed to update task status on error");
                }
            }
        }

        let remove_opts = RemoveContainerOptionsBuilder::new()
            .force(true)
            .build();
        let _ = docker
            .remove_container(&container_id_clone, Some(remove_opts))
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to remove container"));
    });

    Ok(container_id)
}

pub struct ReviewParams {
    pub branch_name: String,
    pub reviewer_login: String,
    pub review_body: String,
    pub review_state: String,
    pub pr_repo: String,
    pub pr_number: i64,
}

pub async fn spawn_review_agent(
    state: &AppState,
    review_task: &Task,
    parent_task: &Task,
    params: &ReviewParams,
) -> Result<String, AppError> {
    let session_id = parent_task
        .session_id
        .as_deref()
        .ok_or_else(|| AppError::SessionNotFound(parent_task.id.clone()))?;

    let docker = Docker::connect_with_local_defaults()
        .with_context(|| "connecting to Docker daemon")
        .map_err(AppError::from)?;

    ensure_agent_image(&docker).await?;

    let session_dir = state
        .config
        .server
        .storage_path
        .join("sessions")
        .join(&parent_task.id);

    let prompt = build_review_prompt(
        params.pr_number,
        &params.pr_repo,
        &params.reviewer_login,
        &params.review_body,
        &params.review_state,
    );
    let opencode_config_content = build_opencode_config(&state.config);
    let repo_url_with_token =
        inject_token_in_repo_url(&parent_task.repo_url, &state.config.github.token);

    let env = vec![
        format!("REPO_URL={}", repo_url_with_token),
        format!("BRANCH_NAME={}", params.branch_name),
        format!("MODE=review"),
        format!("OPENCODE_SESSION_ID={}", session_id),
        format!("OPENCODE_PROMPT={}", prompt),
        format!(
            "OPENCODE_CONFIG_CONTENT={}",
            opencode_config_content
        ),
        format!("GITHUB_TOKEN={}", state.config.github.token),
        format!("OPENCODE_MODEL={}", state.config.opencode.model),
        format!("PR_NUMBER={}", params.pr_number),
    ];

    let container_id =
        create_and_start_container(&docker, &review_task.id, &env, &session_dir).await?;

    state
        .db
        .update_task_status(&review_task.id, "running", Some(&container_id), None, None)
        .await?;

    let task_id = review_task.id.clone();
    let db_clone = state.db.clone();
    let container_id_clone = container_id.clone();
    let notify = state.review_notify.clone();
    let queue_key = format!("{}#{}", params.pr_repo, params.pr_number);

    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "failed to connect to Docker in review monitor");
                return;
            }
        };

        let result =
            wait_for_container_and_parse_result(&docker, &container_id_clone).await;

        match result {
            Ok(_) => {
                if let Err(e) = db_clone
                    .update_task_status(&task_id, "done", None, None, None)
                    .await
                {
                    tracing::error!(error = %e, "failed to update review task status");
                }
            }
            Err(e) => {
                if let Err(e) = db_clone
                    .update_task_status(
                        &task_id,
                        "failed",
                        None,
                        None,
                        Some(&format!("{e}")),
                    )
                    .await
                {
                    tracing::error!(error = %e, "failed to update review task status on error");
                }
            }
        }

        let remove_opts = RemoveContainerOptionsBuilder::new()
            .force(true)
            .build();
        let _ = docker
            .remove_container(&container_id_clone, Some(remove_opts))
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to remove review container"));

        if let Err(e) = notify.send(queue_key) {
            tracing::warn!(error = %e, "failed to send review queue notification");
        }
    });

    Ok(container_id)
}

pub async fn process_next_queued_review(state: &AppState, queue_key: &str) {
    let review = {
        let mut queue = state.review_queue.lock().expect("review queue lock poisoned");
        queue.get_mut(queue_key).and_then(|q| q.pop_front())
    };

    let Some(review) = review else {
        tracing::debug!(queue_key = %queue_key, "no more queued reviews");
        return;
    };

    tracing::info!(
        queue_key = %queue_key,
        reviewer = %review.reviewer_login,
        "processing queued review"
    );

    let parent_task = match state.db.get_task(&review.parent_task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::error!(task_id = %review.parent_task_id, "parent task not found for queued review");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch parent task for queued review");
            return;
        }
    };

    let review_task = match state
        .db
        .insert_task(
            None,
            &format!(
                "Review response to @{} on PR #{}",
                review.reviewer_login, review.pr_number
            ),
            Some(&review.review_body),
            &review.repo_url,
            &review.source,
            Some(&review.parent_task_id),
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to create queued review task");
            return;
        }
    };

    let params = ReviewParams {
        branch_name: review.branch_name,
        reviewer_login: review.reviewer_login,
        review_body: review.review_body,
        review_state: review.review_state,
        pr_repo: review.pr_repo,
        pr_number: review.pr_number,
    };

    if let Err(e) = spawn_review_agent(state, &review_task, &parent_task, &params).await {
        tracing::error!(error = %e, "failed to spawn queued review agent");
    }
}

async fn create_and_start_container(
    docker: &Docker,
    task_id: &str,
    env: &[String],
    session_dir: &Path,
) -> Result<String, AppError> {
    let container_name = format!("autodev-{task_id}");
    let create_options = CreateContainerOptionsBuilder::new()
        .name(&container_name)
        .build();

    let container_config = ContainerCreateBody {
        image: Some(AGENT_IMAGE.to_string()),
        env: Some(env.to_vec()),
        tty: Some(false),
        host_config: Some(HostConfig {
            binds: Some(vec![format!(
                "{}:/root/.local/share/opencode",
                session_dir.display()
            )]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = docker
        .create_container(Some(create_options), container_config)
        .await
        .with_context(|| "creating container")
        .map_err(AppError::from)?;

    let container_id = result.id;
    tracing::info!(container_id = %container_id, "container created");

    docker
        .start_container(&container_id, None)
        .await
        .with_context(|| "starting container")
        .map_err(AppError::from)?;

    tracing::info!(container_id = %container_id, "container started");

    Ok(container_id)
}

async fn wait_for_container_and_parse_result(
    docker: &Docker,
    container_id: &str,
) -> Result<ContainerResult, AppError> {
    let mut logs_stream = docker.logs(
        container_id,
        Some(LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        }),
    );

    let mut collected_logs: Vec<String> = Vec::new();

    while let Some(log_result) = logs_stream.next().await {
        match log_result {
            Ok(LogOutput::StdOut { message }) | Ok(LogOutput::StdErr { message }) => {
                let line = String::from_utf8_lossy(&message).to_string();
                tracing::debug!(container = container_id, %line);
                collected_logs.push(line);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "error reading container logs");
                break;
            }
        }
    }

    let wait_result = docker
        .wait_container(container_id, Some(WaitContainerOptions::default()))
        .next()
        .await;

    let exit_code = match wait_result {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => {
            return Err(AppError::Internal(
                eyre::eyre!("error waiting for container: {e}"),
            ));
        }
        None => {
            return Err(AppError::Internal(
                eyre::eyre!("no wait response from container"),
            ));
        }
    };

    if exit_code != 0 {
        return Err(AppError::Internal(eyre::eyre!(
            "container exited with code {exit_code}"
        )));
    }

    let pr_re = regex::Regex::new(PR_URL_PATTERN).expect("invalid PR URL regex");
    let pr_match = collected_logs.iter().find_map(|line| {
        pr_re.captures(line).map(|caps| {
            let url = caps.get(0).unwrap().as_str().to_string();
            let repo = caps.get(1).unwrap().as_str().to_string();
            let number: i64 = caps
                .get(2)
                .unwrap()
                .as_str()
                .parse()
                .expect("valid PR number");
            (url, repo, number)
        })
    });

    let (pr_url, pr_repo, pr_number) = match pr_match {
        Some((url, repo, number)) => (Some(url), Some(repo), Some(number)),
        None => (None, None, None),
    };

    let session_re =
        regex::Regex::new(SESSION_ID_PATTERN).expect("invalid session ID regex");
    let session_id = collected_logs.iter().find_map(|line| {
        session_re
            .captures(line)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    });

    Ok(ContainerResult {
        pr_url,
        pr_repo,
        pr_number,
        session_id,
    })
}

async fn ensure_agent_image(docker: &Docker) -> Result<(), AppError> {
    let inspect = docker.inspect_image(AGENT_IMAGE).await;

    if inspect.is_err() {
        tracing::info!("agent image not found, building from docker/ directory");
        let tar = build_context_tarball(Path::new("docker"))
            .await
            .with_context(|| "building Docker context tarball")
            .map_err(AppError::from)?;

        let build_options = BuildImageOptionsBuilder::new()
            .dockerfile("Dockerfile.agent")
            .t(AGENT_IMAGE)
            .build();

        let body = body_full(tar.into());

        let mut stream = docker.build_image(build_options, None, Some(body));

        while let Some(result) = stream.next().await {
            result
                .with_context(|| "building agent image")
                .map_err(AppError::from)?;
        }

        tracing::info!(image = AGENT_IMAGE, "agent image built successfully");
    }

    Ok(())
}

async fn build_context_tarball(context_dir: &Path) -> eyre::Result<Vec<u8>> {
    let mut ar = tar::Builder::new(Vec::new());

    ar.append_path_with_name(context_dir.join("Dockerfile.agent"), "Dockerfile.agent")
        .with_context(|| "adding Dockerfile.agent to tarball")?;
    ar.append_path_with_name(context_dir.join("entrypoint.sh"), "entrypoint.sh")
        .with_context(|| "adding entrypoint.sh to tarball")?;

    let data = ar.into_inner().with_context(|| "finalizing tarball")?;
    Ok(data)
}

fn build_prompt(task: &Task) -> String {
    let description = task
        .description
        .as_deref()
        .unwrap_or("(no description provided)");

    let jira_key = task.jira_key.as_deref().unwrap_or("(unknown)");

    format!(
        "You are implementing Jira ticket {jira_key}: {summary}\n\
         \n\
         Description:\n\
         {description}\n\
         \n\
         Instructions:\n\
         1. Analyze the codebase and understand the existing patterns.\n\
         2. Implement the changes described above.\n\
         3. Run any existing tests to verify your changes.\n\
         4. Commit your changes with a descriptive message referencing {jira_key}.\n\
         5. Push to the remote branch.\n\
         6. Open a pull request using:\n\
            gh pr create --title \"{jira_key}: {summary}\" --body \"Implements {jira_key}\"",
        jira_key = jira_key,
        summary = task.summary,
        description = description,
    )
}

fn build_review_prompt(
    pr_number: i64,
    pr_repo: &str,
    reviewer_login: &str,
    review_body: &str,
    review_state: &str,
) -> String {
    match review_state {
        "changes_requested" => format!(
            "You are continuing a previous session for PR #{pr_number} in {pr_repo}.\n\
             \n\
             Reviewer @{reviewer_login} has requested changes:\n\
             \n\
             {review_body}\n\
             \n\
             Instructions:\n\
             - Review the feedback above carefully.\n\
             - If the feedback is valid and makes sense, implement the requested changes.\n\
             - Commit and push the changes to the branch.\n\
             - Reply to the reviewer using: gh pr comment {pr_number} --body \"@{reviewer_login} ...\"\n\
             - Always @mention @{reviewer_login} in your response.\n\
             - If you disagree with the feedback, reply with your reasoning as a PR comment.\n\
             - Do NOT create a new pull request."
        ),
        _ => format!(
            "You are continuing a previous session for PR #{pr_number} in {pr_repo}.\n\
             \n\
             Reviewer @{reviewer_login} left the following feedback:\n\
             \n\
             {review_body}\n\
             \n\
             Instructions:\n\
             - Read and understand the feedback.\n\
             - If the reviewer asked questions, answer them.\n\
             - Post your response using: gh pr comment {pr_number} --body \"@{reviewer_login} ...\"\n\
             - Always @mention @{reviewer_login} in your response.\n\
             - If the feedback requests code changes and they make sense, implement them, commit, and push.\n\
             - Do NOT create a new pull request."
        ),
    }
}

fn build_opencode_config(config: &AppConfig) -> String {
    let mut cfg = serde_json::Map::new();
    cfg.insert(
        "model".into(),
        serde_json::Value::String(config.opencode.model.clone()),
    );
    cfg.insert(
        "permission".into(),
        serde_json::json!({
            "*": "allow",
            "doom_loop": "allow",
        }),
    );

    for (key, value) in &config.opencode.extra {
        cfg.insert(key.clone(), value.clone());
    }

    serde_json::Value::Object(cfg).to_string()
}

fn inject_token_in_repo_url(repo_url: &str, token: &str) -> String {
    if let Some(url) = repo_url.strip_prefix("https://github.com/") {
        format!("https://x-access-token:{token}@github.com/{url}")
    } else {
        repo_url.to_string()
    }
}
