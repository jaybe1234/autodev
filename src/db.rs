use std::str::FromStr;

use chrono::Utc;
use eyre::WrapErr;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Task {
    pub id: String,
    pub jira_key: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub repo_url: String,
    pub status: String,
    pub container_id: Option<String>,
    pub pr_url: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub session_id: Option<String>,
    pub pr_repo: Option<String>,
    pub pr_number: Option<i64>,
    pub parent_task_id: Option<String>,
    pub source: String,
}

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn new(database_url: &str) -> eyre::Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?.create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .with_context(|| "connecting to SQLite")?;

        let db = Self { pool };
        db.run_migrations().await?;
        Ok(db)
    }

    async fn run_migrations(&self) -> eyre::Result<()> {
        let migration_001 = include_str!("../migrations/001_init.sql");
        sqlx::raw_sql(migration_001)
            .execute(&self.pool)
            .await
            .with_context(|| "running migration 001")?;

        let migration_002 = include_str!("../migrations/002_github_webhook.sql");
        sqlx::raw_sql(migration_002)
            .execute(&self.pool)
            .await
            .with_context(|| "running migration 002")?;

        Ok(())
    }

    pub async fn insert_task(
        &self,
        jira_key: Option<&str>,
        summary: &str,
        description: Option<&str>,
        repo_url: &str,
        source: &str,
        parent_task_id: Option<&str>,
    ) -> Result<Task, AppError> {
        let now = Utc::now().to_rfc3339();
        let id = Uuid::now_v7().to_string();

        sqlx::query(
            "INSERT INTO tasks (id, jira_key, summary, description, repo_url, status,
                                created_at, updated_at, source, parent_task_id)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(jira_key)
        .bind(summary)
        .bind(description)
        .bind(repo_url)
        .bind(&now)
        .bind(&now)
        .bind(source)
        .bind(parent_task_id)
        .execute(&self.pool)
        .await
        .with_context(|| "inserting new task")?;

        self.get_task(&id)
            .await?
            .ok_or_else(|| AppError::TaskNotFound(id))
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>, AppError> {
        sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| "fetching task")
            .map_err(AppError::from)
    }

    pub async fn list_tasks(&self) -> Result<Vec<Task>, AppError> {
        sqlx::query_as::<_, Task>("SELECT * FROM tasks ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
            .with_context(|| "listing tasks")
            .map_err(AppError::from)
    }

    pub async fn find_active_task_by_jira_key(
        &self,
        jira_key: &str,
    ) -> Result<Option<Task>, AppError> {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks
             WHERE jira_key = ? AND status IN ('pending', 'running')
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(jira_key)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| "finding active task by jira key")
        .map_err(AppError::from)
    }

    pub async fn find_original_task_by_pr(
        &self,
        pr_repo: &str,
        pr_number: i64,
    ) -> Result<Option<Task>, AppError> {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks
             WHERE pr_repo = ? AND pr_number = ? AND source = 'jira'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(pr_repo)
        .bind(pr_number)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| "finding original task by PR")
        .map_err(AppError::from)
    }

    pub async fn find_active_review_for_task(
        &self,
        parent_task_id: &str,
    ) -> Result<Option<Task>, AppError> {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks
             WHERE parent_task_id = ? AND status IN ('pending', 'running')
               AND source IN ('github_review', 'github_comment')
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(parent_task_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| "finding active review task")
        .map_err(AppError::from)
    }

    pub async fn update_task_status(
        &self,
        id: &str,
        status: &str,
        container_id: Option<&str>,
        pr_url: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE tasks SET status = ?, container_id = COALESCE(?, container_id),
             pr_url = COALESCE(?, pr_url), error = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(container_id)
        .bind(pr_url)
        .bind(error)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| "updating task status")
        .map_err(AppError::from)?;

        Ok(())
    }

    pub async fn update_task_pr_info(
        &self,
        id: &str,
        pr_url: &str,
        pr_repo: &str,
        pr_number: i64,
        session_id: &str,
    ) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE tasks SET pr_url = ?, pr_repo = ?, pr_number = ?,
             session_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(pr_url)
        .bind(pr_repo)
        .bind(pr_number)
        .bind(session_id)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| "updating task PR info")
        .map_err(AppError::from)?;

        Ok(())
    }
}
