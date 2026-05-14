# AGENTS.md

Autodev is a Rust server that receives Jira and GitHub webhooks and spawns Docker
containers running [opencode](https://opencode.ai) to implement tickets, open pull
requests, and respond to PR reviews automatically.

## Architecture

```
Jira webhook → Axum handler → SQLite (task row) → Docker container (agent)
                                                           ↓
                                               opencode run (headless, full permissions)
                                                           ↓
                                               git push + gh pr create
                                                           ↓
                                               Monitor task → Jira comment + transition

GitHub webhook → Axum handler → find original task by PR → SQLite (review task)
                                                           ↓
                                               Docker container (review agent)
                                                           ↓
                                               opencode run --session <id> (resumes session)
                                                           ↓
                                               gh pr comment / git push
                                                           ↓
                                               Notify queue processor → dequeue next review
```

A single `AppState` struct holds the config, database pool, GitHub username, and
review queue. It is shared across all axum handlers via `State<AppState>`.

## Tech stack

- **Runtime**: Tokio (full features)
- **Web framework**: Axum 0.8
- **Docker API**: bollard 0.21 (uses builder pattern for query params —
  `CreateContainerOptionsBuilder`, `BuildImageOptionsBuilder`, etc.)
- **Database**: SQLite via sqlx 0.8 (runtime-tokio, no macros — raw SQL queries)
- **HTTP client**: reqwest 0.13 (for Jira and GitHub REST API calls)
- **Errors**: `eyre` for context-propagation with `.with_context()`, `thiserror`
  for the `AppError` enum that implements `IntoResponse`

## Key conventions

### Error handling

All fallible operations use `eyre::WrapErr::with_context()` before `?`.  The
`AppError` enum converts to HTTP responses via `IntoResponse`:
- `WebhookVerification` → 401
- `NoMatchingRepo` / `DuplicateTask` → 400 / 409
- `TaskNotFound` → 404
- `SessionNotFound` → 409
- `NoOriginalTask` → 404
- Everything else → 500 (logged at error level)

### Bollard API (Docker)

bollard 0.21 uses the builder pattern for all query-parameter structs.  Do not
construct them directly — use the `*Builder` types from
`bollard::query_parameters`.  Container config uses
`bollard::models::ContainerCreateBody` (not `Config`).  Use `bollard::body_full()`
to wrap tarball bytes for `build_image`.

### Database

SQLite schema lives in `migrations/001_init.sql` and `migrations/002_github_webhook.sql`.
The `Db` type wraps a `SqlitePool` and exposes async CRUD methods.  Migrations run on
startup via `sqlx::raw_sql`.  Task IDs are UUID v7 (time-sortable).  Task statuses:
`pending` → `running` → `done` | `failed`.

The `tasks` table has a `source` column: `'jira'` for implementation tasks,
`'github_review'` for review responses, `'github_comment'` for comment responses.
Review tasks have `parent_task_id` pointing to the original implementation task.

### Webhook verification

Both Jira and GitHub webhooks are verified using HMAC-SHA256 against the **raw
request body bytes**. The handlers receive `axum::body::Bytes` directly (not
`Json<T>`) to preserve the exact bytes for signature verification. JSON is parsed
after verification.

### Config

TOML file loaded at startup from `config.toml` (or `AUTODEV_CONFIG` env var).
The `[opencode]` section is serialized to JSON and passed to the agent container
via the `OPENCODE_CONFIG_CONTENT` env var.  The `[[mapping]]` array maps Jira
labels to GitHub repo URLs.  The `[github]` section includes both `token` and
`webhook_secret`.

### Agent container

Defined in `docker/Dockerfile.agent` (based on `ghcr.io/anomalyco/opencode`).
The entrypoint script (`docker/entrypoint.sh`) supports two modes:

- **Implementation mode** (default): clones repo, creates branch, runs
  `opencode run` with `--dangerously-skip-permissions`.
- **Review mode** (`MODE=review`): clones repo, fetches and checks out existing
  branch, runs `opencode run --session <id>` to resume the session.

Session data is mounted to `<storage_path>/sessions/<task_id>/` on the host.
Review containers mount the **parent** task's session directory to access the
original session.

### Review queue

Concurrent reviews for the same PR are serialized via an in-memory queue
(`Mutex<HashMap<String, VecDeque<QueuedReview>>>` keyed by `"{repo}#{pr_number}"`).
When a review container finishes, it sends a notification on an unbounded mpsc
channel. A background task receives notifications and processes the next queued
review (if any). This avoids recursive async function cycles.

### Bot identity

On startup, the server calls `GET /user` on the GitHub API to determine the
authenticated user's login. This is stored in `AppState.github_username` and used
to filter out the bot's own comments and detect @mentions.

## Running

```sh
cargo run  # reads config.toml, binds to 0.0.0.0:3000
```

Config path override: `AUTODEV_CONFIG=./path/to/config.toml cargo run`

## API endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/webhooks/jira` | Jira webhook receiver (HMAC-verified) |
| POST | `/webhooks/github` | GitHub webhook receiver (HMAC-verified) |
| GET | `/tasks` | List all tasks |
| GET | `/tasks/{id}` | Get task by ID |
| GET | `/health` | Health check |

### GitHub webhook events handled

| Event | Action | Filter | Behavior |
|-------|--------|--------|----------|
| `pull_request_review` | `submitted` | reviewer ≠ bot, state ≠ approved | `changes_requested`: implement changes; `commented`: answer questions |
| `issue_comment` | `created` | on PR, commenter ≠ bot, @mentions bot | Answer questions or implement requested changes |

## Post-edit verification

After making any changes to Rust source files, always run:

```sh
cargo fmt
cargo check
```

Fix any errors or warnings before considering the task complete.

## Testing

No test suite yet.  When adding tests, prefer `insta` for snapshot testing and
`tokio::test` for async tests.  The SQLite database can be created in-memory with
`sqlite::memory:` for test isolation.
