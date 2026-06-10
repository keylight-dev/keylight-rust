pub mod retry;
pub mod ureq_transport;

/// Minimal HTTP response the SDK reasons about.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
}

/// Outcome of a transport attempt: an HTTP response, or a transient/terminal network failure.
pub enum TransportOutcome {
    Response(HttpResponse),
    /// Retryable transport-level failure (timeout, connection lost, DNS, ...).
    Transient(String),
    /// Non-retryable transport failure.
    Terminal(String),
    Timeout,
}

/// A blocking HTTP transport. Implemented by `ureq_transport::UreqTransport`; mocked in tests.
pub trait Transport: Send + Sync {
    fn post_json(&self, url: &str, headers: &[(String, String)], body: &str) -> TransportOutcome;
    fn get(&self, url: &str, headers: &[(String, String)]) -> TransportOutcome;
}
