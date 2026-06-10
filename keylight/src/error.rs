use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeylightError {
    #[error("client error (HTTP {status}): {message}")]
    ClientError { status: u16, message: String },
    #[error("server error (HTTP {status})")]
    ServerError { status: u16 },
    #[error("rate limited; retry after {retry_after}s")]
    RateLimited { retry_after: u64 },
    #[error("request timed out")]
    Timeout,
    #[error("network failure: {0}")]
    NetworkFailure(String),
    #[error("invalid server response")]
    InvalidResponse,
    #[error("lease signature verification failed")]
    LeaseVerificationFailed,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("no stored license")]
    NoStoredLicense,
}

pub type Result<T> = std::result::Result<T, KeylightError>;
