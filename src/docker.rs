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

const AGENT_IMAGE: &str = "autodev-agent";
const PR_URL_PATTERN: &str = r"https://github\.com/[^/]+/[^/]+/pull/\d+";

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
    let repo_url_with_token = inject_token_in_repo_url(&task.repo_url, &config.github.token);
    let branch_name = format!("autodev/{}", task.jira_key);

    let env = vec![
        format!("REPO_URL={}", repo_url_with_token),
        format!("JIRA_KEY={}", task.jira_key),
        format!("BRANCH_NAME={}", branch_name),
        format!("OPENCODE_PROMPT={}", prompt),
        format!("OPENCODE_CONFIG_CONTENT={}", opencode_config_content),
        format!("GITHUB_TOKEN={}", config.github.token),
        format!("OPENCODE_MODEL={}", config.opencode.model),
    ];

    let container_name = format!("autodev-{}", task.id);
    let create_options = CreateContainerOptionsBuilder::new()
        .name(&container_name)
        .build();

    let container_config = ContainerCreateBody {
        image: Some(AGENT_IMAGE.to_string()),
        env: Some(env),
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

    db.update_task_status(&task.id, "running", Some(&container_id), None, None)
        .await?;

    docker
        .start_container(&container_id, None)
        .await
        .with_context(|| "starting container")
        .map_err(AppError::from)?;

    tracing::info!(container_id = %container_id, "container started");

    let task_id = task.id.clone();
    let jira_key = task.jira_key.clone();
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

        let outcome = wait_for_container_and_parse_result(&docker, &container_id_clone).await;

        let (status, pr_url, error): (&str, Option<String>, Option<String>) = match outcome {
            Ok(Some(pr)) => ("done", Some(pr), None),
            Ok(None) => (
                "failed",
                None,
                Some("container exited but no PR URL found in logs".into()),
            ),
            Err(e) => ("failed", None, Some(format!("{e}"))),
        };

        if let Err(e) = db_clone
            .update_task_status(&task_id, status, None, pr_url.as_deref(), error.as_deref())
            .await
        {
            tracing::error!(error = %e, "failed to update task status");
        }

        let jira_client = JiraClient::new(&jira_config.base_url, &jira_config.pat);
        match status {
            "done" => {
                if let Some(ref pr) = pr_url {
                    let comment = format!(
                        "Autodev has implemented this ticket and opened a PR: {}",
                        pr
                    );
                    if let Err(e) = jira_client.add_comment(&jira_key, &comment).await {
                        tracing::error!(error = %e, "failed to add Jira comment");
                    }
                }
                if let Err(e) = jira_client
                    .transition_issue(&jira_key, &jira_config.transition_to)
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
                if let Err(e) = jira_client.add_comment(&jira_key, &comment).await {
                    tracing::error!(error = %e, "failed to add failure comment to Jira");
                }
            }
            _ => {}
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

async fn wait_for_container_and_parse_result(
    docker: &Docker,
    container_id: &str,
) -> Result<Option<String>, AppError> {
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

    let re = regex::Regex::new(PR_URL_PATTERN).expect("invalid PR URL regex");
    let pr_url = collected_logs
        .iter()
        .find_map(|line| re.find(line).map(|m| m.as_str().to_string()));

    Ok(pr_url)
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
        jira_key = task.jira_key,
        summary = task.summary,
        description = description,
    )
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
