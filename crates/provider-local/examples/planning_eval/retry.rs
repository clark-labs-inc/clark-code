use crate::model::RetryReceipt;
use std::time::{Duration, Instant};

pub const ROUTE_DELAYS: [Duration; 5] = [
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(300),
];
pub const PHASE_DELAYS: [Duration; 2] = [Duration::from_secs(60), Duration::from_secs(300)];
const PROGRESS_INTERVAL: Duration = Duration::from_secs(30);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

pub fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

pub fn retryable_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "429",
        "rate limit",
        "too many requests",
        "capacity",
        "502",
        "503",
        "504",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

pub async fn wait_with_progress(scope: &str, requested: Duration) -> u128 {
    let requested = requested.min(MAX_RETRY_AFTER);
    let started = Instant::now();
    let mut remaining = requested;
    while !remaining.is_zero() {
        let slice = remaining.min(PROGRESS_INTERVAL);
        eprintln!(
            "{scope}: Free-tier capacity backoff, {}s remaining (maximum single wait 300s)",
            remaining.as_secs()
        );
        tokio::time::sleep(slice).await;
        remaining = requested.saturating_sub(started.elapsed());
    }
    started.elapsed().as_millis()
}

pub fn receipt(
    scope: impl Into<String>,
    attempt: usize,
    status: impl Into<String>,
    reason: impl Into<String>,
    requested_wait: Duration,
    actual_wait_ms: u128,
) -> RetryReceipt {
    RetryReceipt {
        scope: scope.into(),
        attempt,
        status: status.into(),
        reason: reason.into(),
        requested_wait_ms: requested_wait.as_millis() as u64,
        actual_wait_ms,
        model_output_observed: false,
        workspace_mutated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_capacity_and_timeout_failures_are_retryable() {
        for status in [429, 502, 503, 504] {
            assert!(retryable_status(status));
        }
        for status in [400, 401, 403, 404, 409, 422, 500] {
            assert!(!retryable_status(status));
        }
        assert!(retryable_error("upstream 429: Too Many Requests"));
        assert!(retryable_error("provider capacity exhausted"));
        assert!(retryable_error("provider turn timed out"));
        assert!(!retryable_error("invalid tool arguments"));
    }
}
