CREATE TABLE IF NOT EXISTS tasks_new (
    id             TEXT PRIMARY KEY,
    jira_key       TEXT,
    summary        TEXT NOT NULL,
    description    TEXT,
    repo_url       TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending',
    container_id   TEXT,
    pr_url         TEXT,
    error          TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    session_id     TEXT,
    pr_repo        TEXT,
    pr_number      INTEGER,
    parent_task_id TEXT,
    source         TEXT NOT NULL DEFAULT 'jira'
);

INSERT INTO tasks_new
    (id, jira_key, summary, description, repo_url, status, container_id,
     pr_url, error, created_at, updated_at, session_id, pr_repo, pr_number,
     parent_task_id, source)
SELECT
    id, jira_key, summary, description, repo_url, status, container_id,
    pr_url, error, created_at, updated_at,
    NULL, NULL, NULL, NULL, 'jira'
FROM tasks;

DROP TABLE tasks;

ALTER TABLE tasks_new RENAME TO tasks;
