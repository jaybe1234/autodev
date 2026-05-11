use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("docker error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),

    #[error("webhook verification failed")]
    WebhookVerification,

    #[error("no matching repo for ticket labels")]
    NoMatchingRepo,

    #[error("ticket already has an active task")]
    DuplicateTask,

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("{0}")]
    Internal(#[from] eyre::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::WebhookVerification => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::NoMatchingRepo => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::DuplicateTask => (StatusCode::CONFLICT, self.to_string()),
            AppError::TaskNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            _ => {
                tracing::error!(error = %self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".into())
            }
        };
        (status, message).into_response()
    }
}
