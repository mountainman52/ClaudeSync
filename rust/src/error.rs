use thiserror::Error;

/// Mirrors the Python `ConfigurationError` / `ProviderError` exceptions.
#[derive(Error, Debug)]
pub enum CsError {
    #[error("{0}")]
    Configuration(String),

    #[error("{0}")]
    Provider(String),

    #[error("{0}")]
    Other(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CsError>;

impl CsError {
    /// Errors that the Python CLI's `handle_errors` decorator would catch
    /// and print as `Error: ...` instead of propagating.
    pub fn is_handled(&self) -> bool {
        matches!(self, CsError::Configuration(_) | CsError::Provider(_))
    }
}
