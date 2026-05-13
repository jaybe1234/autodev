---
name: create-webhook
description: Scaffold a new webhook integration in the autodev server. Use when adding a new external service webhook (e.g., GitLab, Bitbucket, Slack, Linear) or when the user mentions "new webhook", "add webhook", or "webhook integration".
---

# Create a New Webhook Integration

## Directory convention

```
src/webhooks/<name>/
├── mod.rs      # Handler function(s)
└── types.rs    # Serde payload structs
```

Register the submodule in `src/webhooks/mod.rs`:
```rust
pub mod <name>;
```

## Step-by-step workflow

### 1. Define payload types (`types.rs`)

- Use `serde::Deserialize` for inbound webhooks.
- Mirror the external service's JSON structure with flat, named structs.
- Use `#[serde(rename = "...")]` for fields whose JSON names are Rust reserved words or use different casing.

```rust
// src/webhooks/<name>/types.rs
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    pub action: String,
    pub repository: Repository,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    pub full_name: String,
}
```

### 2. Implement the handler (`mod.rs`)

Follow the established pattern:

```rust
// src/webhooks/<name>/mod.rs
mod types;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use eyre::WrapErr;

use crate::error::AppError;
use crate::state::AppState;
use crate::webhooks::crypto::verify_webhook_signature;
use types::WebhookPayload;

pub async fn handle_<name>_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::http::StatusCode, AppError> {
    // 1. Verify HMAC signature against the raw body bytes
    verify_webhook_signature(&headers, &body, &/* config secret field */)?;

    // 2. Parse JSON from the verified raw bytes
    let payload: WebhookPayload = serde_json::from_slice(&body)
        .with_context(|| "parsing <name> webhook payload")
        .map_err(AppError::from)?;

    // 3. Filter — return 200 OK early for events you don't care about
    // 4. Look up matching repo/task, deduplicate, insert task
    // 5. Return ACCEPTED (202) when spawning work, OK (200) when ignoring
    Ok(axum::http::StatusCode::ACCEPTED)
}
```

Key rules:
- **Always** receive `Bytes` (not `Json<T>`) to preserve raw bytes for signature verification.
- **Always** call `verify_webhook_signature` before parsing.
- Use `.with_context()` before every `?` propagation (eyre convention).
- Return `StatusCode::OK` for ignored events, `StatusCode::ACCEPTED` when work is queued.

### 3. Add route in `src/main.rs`

```rust
.route("/webhooks/<name>", post(webhooks::<name>::handle_<name>_webhook))
```

### 4. Add config

Add a new section to `config.toml` and the corresponding struct in `src/config.rs`:

```toml
[<name>]
webhook_secret = "..."
```

Add a `webhook_secret` field to the config struct. If the service uses label/tag → repo mapping, add a `[[mapping]]` entry or a service-specific mapping array.

### 5. Register the module

In `src/webhooks/mod.rs`:
```rust
pub mod <name>;
```

### 6. Verify

Run `cargo check` to confirm compilation. The handler path visible from `main.rs` is `webhooks::<name>::handle_<name>_webhook`.

## Shared utilities

- **Signature verification**: `crate::webhooks::crypto::verify_webhook_signature(headers, body, secret)` — checks `x-hub-signature-256` or `x-hub-signature` headers with HMAC-SHA256.
- **Error type**: `crate::error::AppError` — has variants for `WebhookVerification`, `NoMatchingRepo`, `DuplicateTask`, `TaskNotFound`, etc.
- **State**: `crate::state::AppState` — holds config, DB pool, GitHub username, review queue.

## Checklist

- [ ] `src/webhooks/<name>/types.rs` — payload structs with serde
- [ ] `src/webhooks/<name>/mod.rs` — handler with signature verification, parsing, filtering
- [ ] `src/webhooks/mod.rs` — `pub mod <name>;` added
- [ ] `src/main.rs` — route registered
- [ ] `src/config.rs` — config struct + TOML field for webhook secret
- [ ] `cargo check` passes
