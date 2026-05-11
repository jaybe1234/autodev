use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::db::Task;
use crate::error::AppError;
use crate::state::AppState;

pub async fn list_tasks(
    State(state): State<AppState>,
) -> Result<Json<Vec<Task>>, AppError> {
    let tasks = state.db.list_tasks().await?;
    Ok(Json(tasks))
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>, AppError> {
    let task = state
        .db
        .get_task(&id)
        .await?
        .ok_or_else(|| AppError::TaskNotFound(id))?;
    Ok(Json(task))
}

pub async fn health() -> StatusCode {
    StatusCode::OK
}
