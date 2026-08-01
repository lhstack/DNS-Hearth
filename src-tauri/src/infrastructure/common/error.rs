//! Application error contract shared by the business layer.
//!
//! The business layer returns `AppError`. Only the Tauri adapter maps it onto
//! HTTP status codes, so business code never depends on axum.

use thiserror::Error;

/// Main error type for the application
#[derive(Error, Debug)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),

    #[error("{0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("DNS error: {0}")]
    Dns(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}

/// Result type alias for the application
pub type AppResult<T> = Result<T, AppError>;
