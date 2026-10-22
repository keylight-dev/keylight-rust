//! The crate error type [`KeylightError`] and its [`Result`] alias.

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
    #[error(
        "no trusted signing keys configured; add .trusted_key(kid, public_key_base64) using your tenant's /.well-known/keylight-keys endpoint"
    )]
    MissingTrustedKeys,
    #[error(
        "no trusted public key configured for signing key ID '{kid}'; add the matching kid and public_key from your tenant's /.well-known/keylight-keys endpoint"
    )]
    UnknownSigningKey { kid: String },
    #[error("lease signature verification failed")]
    LeaseVerificationFailed,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("no stored license")]
    NoStoredLicense,
}

pub type Result<T> = std::result::Result<T, KeylightError>;
