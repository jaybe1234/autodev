use std::str::FromStr;

use chrono::Utc;
use eyre::WrapErr;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Task {
    pub id: String,
    pub jira_key: String,
    pub summary: String,
    pub description: Option<String>,
    pub repo_url: String,
    pub status: String,
    pub container_id: Option<String>,
    pub pr_url: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn new(database_url: &str) -> eyre::Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true);

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
        let migration_sql = include_str!("../migrations/001_init.sql");
        sqlx::raw_sql(migration_sql)
            .execute(&self.pool)
            .await
            .with_context(|| "running migrations")?;
        Ok(())
    }

    pub async fn insert_task(
        &self,
        jira_key: &str,
        summary: &str,
        description: Option<&str>,
        repo_url: &str,
    ) -> Result<Task, AppError> {
        let now = Utc::now().to_rfc3339();
        let id = Uuid::now_v7().to_string();

        sqlx::query(
            "INSERT INTO tasks (id, jira_key, summary, description, repo_url, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)"
        )
        .bind(&id)
        .bind(jira_key)
        .bind(summary)
        .bind(description)
        .bind(repo_url)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| "inserting new task")?;

        self.get_task(&id).await?.ok_or_else(|| AppError::TaskNotFound(id))
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

    pub async fn find_active_task_by_jira_key(&self, jira_key: &str) -> Result<Option<Task>, AppError> {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks
             WHERE jira_key = ? AND status IN ('pending', 'running')
             ORDER BY created_at DESC LIMIT 1"
        )
        .bind(jira_key)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| "finding active task by jira key")
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
             pr_url = COALESCE(?, pr_url), error = ?, updated_at = ? WHERE id = ?"
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
}
