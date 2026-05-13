# autodev

A Rust server that receives Jira and GitHub webhooks and spawns Docker containers running [opencode](https://opencode.ai) to implement tickets, open pull requests, and respond to PR reviews automatically.

## How it works

1. A Jira issue transitions to "ready-to-dev" (or similar status)
2. autodev receives the webhook, finds the matching GitHub repo by label, and spawns a Docker container
3. The container clones the repo, creates a branch, and runs `opencode run` to implement the ticket
4. On completion, the container pushes the branch and opens a pull request
5. autodev updates the Jira issue with the PR link and transitions it to "In Review"
6. When a reviewer submits changes or comments on the PR, autodev spawns a review agent that resumes the session and addresses the feedback

Review agents for the same PR are serialized via an in-memory queue to avoid conflicts.

## Prerequisites

- Rust (edition 2024)
- Docker (with access to the daemon socket)
- A Jira instance with webhook support
- A GitHub account with a personal access token (repo scope)
- An opencode-compatible API key (Anthropic, OpenAI, etc.)

## Configuration

Copy the example config and fill in your values:

```sh
cp config.example.toml config.toml
```

```toml
[server]
bind = "0.0.0.0:3000"
webhook_secret = "your-jira-webhook-secret"
storage_path = "./data"

[jira]
base_url = "https://myorg.atlassian.net"
pat = "your-jira-personal-access-token"
transition_to = "In Review"
ready_to_dev_status = "ready-to-dev"

[github]
token = "ghp_your-github-personal-access-token"
webhook_secret = "your-github-webhook-secret"

[opencode]
model = "anthropic/claude-sonnet-4-20250514"

[[mapping]]
label = "backend"
repo = "https://github.com/myorg/backend.git"

[[mapping]]
label = "frontend"
repo = "https://github.com/myorg/frontend.git"
```

The config path can be overridden with the `AUTODEV_CONFIG` environment variable.

### `[[mapping]]`

Each entry maps a Jira label to a GitHub repository. When a Jira issue has the label `backend`, autodev targets the corresponding repo URL.

### `[opencode]`

The `model` field sets the LLM model. Additional opencode configuration can be passed through `opencode.extra`, which is serialized to JSON and injected into the agent container via the `OPENCODE_CONFIG_CONTENT` environment variable.

## Running

```sh
cargo run
```

The server binds to `0.0.0.0:3000` by default. Storage (SQLite database and opencode session data) is written to the configured `storage_path`.

## API

| Method | Path              | Description                            |
|--------|-------------------|----------------------------------------|
| POST   | `/webhooks/jira`   | Jira webhook receiver (HMAC-verified)  |
| POST   | `/webhooks/github` | GitHub webhook receiver (HMAC-verified)|
| GET    | `/tasks`          | List all tasks                         |
| GET    | `/tasks/{id}`     | Get task by ID                         |
| GET    | `/health`         | Health check                           |

## GitHub webhook events

| Event                 | Trigger                                      | Behavior                                    |
|-----------------------|----------------------------------------------|---------------------------------------------|
| `pull_request_review` | Reviewer submits (not approved)              | Implements requested changes or answers     |
| `issue_comment`       | PR comment that @mentions the bot            | Answers questions or implements changes     |

The bot filters out its own comments using the authenticated GitHub username (fetched on startup).

## Architecture

```
Jira webhook → Axum handler → SQLite (task row) → Docker container (agent)
                                                           │
                                               opencode run (headless, full permissions)
                                                           │
                                               git push + gh pr create
                                                           │
                                               Monitor task → Jira comment + transition

GitHub webhook → Axum handler → find original task by PR → SQLite (review task)
                                                           │
                                               Docker container (review agent)
                                                           │
                                               opencode run --session <id> (resumes session)
                                                           │
                                               gh pr comment / git push
                                                           │
                                               Notify queue processor → dequeue next review
```

## Tech stack

- **Runtime**: Tokio
- **Web framework**: Axum 0.8
- **Docker API**: bollard 0.21
- **Database**: SQLite via sqlx 0.8
- **HTTP client**: reqwest 0.13
- **Errors**: eyre + thiserror

## License

All rights reserved.
