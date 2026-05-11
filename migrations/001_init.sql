CREATE TABLE IF NOT EXISTS tasks (
    id           TEXT PRIMARY KEY,
    jira_key     TEXT NOT NULL,
    summary      TEXT NOT NULL,
    description  TEXT,
    repo_url     TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    container_id TEXT,
    pr_url       TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
