//! Decide whether/how long to retry, given an attempt outcome. Pure logic so the
//! backoff policy is unit-tested without sleeping or networking.

#[derive(Debug, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAfter(u64 /* millis */),
    Stop,
}

pub const MAX_ATTEMPTS: u32 = 3;
const BASE_MS: u64 = 500;
const CAP_MS: u64 = 4000;
const MAX_SLEEP_MS: u64 = 3_600_000;

/// Backoff for a given attempt (1-based), without jitter (jitter added by caller).
pub fn backoff_ms(attempt: u32) -> u64 {
    let exp = BASE_MS.saturating_mul(1u64 << (attempt.saturating_sub(1)));
    exp.min(CAP_MS)
}

/// Clamp a server- or backoff-derived sleep to a safe range.
pub fn clamp_sleep_ms(ms: u64) -> u64 {
    ms.min(MAX_SLEEP_MS)
}

/// Is this HTTP status retryable?
pub fn status_retryable(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

/// Decide next step for an HTTP status on a given attempt.
/// `retry_after_secs` is the parsed Retry-After (429 only).
pub fn decide(status: u16, attempt: u32, retry_after_secs: Option<u64>) -> RetryDecision {
    if attempt >= MAX_ATTEMPTS || !status_retryable(status) {
        return RetryDecision::Stop;
    }
    let ms = if status == 429 {
        clamp_sleep_ms(
            retry_after_secs
                .map(|s| s * 1000)
                .unwrap_or_else(|| backoff_ms(attempt)),
        )
    } else {
        clamp_sleep_ms(backoff_ms(attempt))
    };
    RetryDecision::RetryAfter(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_capped() {
        assert_eq!(backoff_ms(1), 500);
        assert_eq!(backoff_ms(2), 1000);
        assert_eq!(backoff_ms(10), 4000);
    }

    #[test]
    fn stops_at_max_attempts() {
        assert_eq!(decide(500, MAX_ATTEMPTS, None), RetryDecision::Stop);
    }

    #[test]
    fn non_retryable_status_stops() {
        assert_eq!(decide(404, 1, None), RetryDecision::Stop);
    }

    #[test]
    fn honors_retry_after_on_429() {
        assert_eq!(decide(429, 1, Some(2)), RetryDecision::RetryAfter(2000));
    }

    #[test]
    fn retries_5xx_with_backoff() {
        assert_eq!(decide(503, 1, None), RetryDecision::RetryAfter(500));
    }
}
