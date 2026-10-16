//! Error type shared by every command.
//!
//! Errors cross the IPC boundary as a plain string so the webview gets a message a human can read.
//! Internal detail (OS error codes) is preserved in the message; nothing about file *contents* is
//! ever included.

use serde::ser::{Serialize, Serializer};

#[derive(Debug, Clone, thiserror::Error)]
pub enum AppError {
    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("The path is too long for the Windows API: {0}")]
    LongPath(String),

    #[error("Symlink loop detected at {0}")]
    SymlinkCycle(String),

    #[error("This operation is not permitted: {0}")]
    NotPermitted(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("The item changed since the scan, so it was skipped: {0}")]
    Changed(String),

    #[error("IO error at {path}: {message}")]
    Io { path: String, message: String },

    #[error("Database error: {0}")]
    Database(String),

    #[error("Licence error: {0}")]
    Licence(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn io(path: impl Into<String>, error: &std::io::Error) -> Self {
        let path = path.into();
        let message = error.to_string();
        match error.kind() {
            std::io::ErrorKind::PermissionDenied => AppError::AccessDenied(path),
            std::io::ErrorKind::NotFound => AppError::NotFound(path),
            _ => AppError::Io { path, message },
        }
    }

    /// Machine-readable kind, used for the `ScanErrorInfo.kind` field.
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::AccessDenied(_) => "access-denied",
            AppError::NotFound(_) => "not-found",
            AppError::LongPath(_) => "long-path",
            AppError::SymlinkCycle(_) => "symlink-cycle",
            AppError::NotPermitted(_) | AppError::InvalidArgument(_) | AppError::Changed(_) => "other",
            AppError::Io { .. } => "io",
            AppError::Database(_) | AppError::Licence(_) | AppError::Internal(_) => "other",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        AppError::Io {
            path: String::new(),
            message: error.to_string(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        AppError::Database(error.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        AppError::Internal(error.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl From<String> for AppError {
    fn from(message: String) -> Self {
        AppError::Internal(message)
    }
}
