# AGENTS.md

Autodev is a Rust server that receives Jira webhooks and spawns Docker containers
running [opencode](https://opencode.ai) to implement tickets and open GitHub pull
requests automatically.

## Architecture

```
Jira webhook → Axum handler → SQLite (task row) → Docker container (agent)
                                                          ↓
                                              opencode run (headless, full permissions)
                                                          ↓
                                              git push + gh pr create
                                                          ↓
                                              Monitor task → Jira comment + transition
```

A single `AppState` struct holds the config and database pool, shared across all
axum handlers via `State<AppState>`.

## Tech stack

- **Runtime**: Tokio (full features)
- **Web framework**: Axum 0.8
- **Docker API**: bollard 0.21 (uses builder pattern for query params —
  `CreateContainerOptionsBuilder`, `BuildImageOptionsBuilder`, etc.)
- **Database**: SQLite via sqlx 0.8 (runtime-tokio, no macros — raw SQL queries)
- **HTTP client**: reqwest 0.13 (for Jira REST API calls)
- **Errors**: `eyre` for context-propagation with `.with_context()`, `thiserror`
  for the `AppError` enum that implements `IntoResponse`

## Key conventions

### Error handling

All fallible operations use `eyre::WrapErr::with_context()` before `?`.  The
`AppError` enum converts to HTTP responses via `IntoResponse`:
- `WebhookVerification` → 401
- `NoMatchingRepo` / `DuplicateTask` → 400 / 409
- `TaskNotFound` → 404
- Everything else → 500 (logged at error level)

### Bollard API (Docker)

bollard 0.21 uses the builder pattern for all query-parameter structs.  Do not
construct them directly — use the `*Builder` types from
`bollard::query_parameters`.  Container config uses
`bollard::models::ContainerCreateBody` (not `Config`).  Use `bollard::body_full()`
to wrap tarball bytes for `build_image`.

### Database

SQLite schema lives in `migrations/001_init.sql`.  The `Db` type wraps a
`SqlitePool` and exposes async CRUD methods.  Migrations run on startup via
`sqlx::raw_sql`.  Task IDs are UUID v7 (time-sortable).  Task statuses:
`pending` → `running` → `done` | `failed`.

### Config

TOML file loaded at startup from `config.toml` (or `AUTODEV_CONFIG` env var).
The `[opencode]` section is serialized to JSON and passed to the agent container
via the `OPENCODE_CONFIG_CONTENT` env var.  The `[[mapping]]` array maps Jira
labels to GitHub repo URLs.

### Agent container

Defined in `docker/Dockerfile.agent` (based on `ghcr.io/anomalyco/opencode`).
The entrypoint script (`docker/entrypoint.sh`) clones the repo, creates a branch,
and runs `opencode run` with `--dangerously-skip-permissions`.  Session data is
mounted to `<storage_path>/sessions/<task_id>/` on the host.

## Running

```sh
cargo run  # reads config.toml, binds to 0.0.0.0:3000
```

Config path override: `AUTODEV_CONFIG=./path/to/config.toml cargo run`

## API endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/webhook/jira` | Jira webhook receiver (HMAC-verified) |
| GET | `/tasks` | List all tasks |
| GET | `/tasks/{id}` | Get task by ID |
| GET | `/health` | Health check |

## Testing

No test suite yet.  When adding tests, prefer `insta` for snapshot testing and
`tokio::test` for async tests.  The SQLite database can be created in-memory with
`sqlite::memory:` for test isolation.
