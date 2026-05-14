use crate::error::AppError;

pub struct ContainerError {
    pub exit_code: i64,
    pub logs: String,
}

impl std::fmt::Display for ContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "container exited with code {}", self.exit_code)
    }
}

impl std::fmt::Debug for ContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerError")
            .field("exit_code", &self.exit_code)
            .field("logs", &self.logs)
            .finish()
    }
}

impl std::error::Error for ContainerError {}

impl From<ContainerError> for AppError {
    fn from(e: ContainerError) -> Self {
        AppError::Internal(eyre::eyre!("{e}"))
    }
}
