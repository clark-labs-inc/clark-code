//! Shared classification for provider access failures.
//!
//! Providers retain transport-specific retry and recovery logic, but billing,
//! authentication, and rate-limit evidence must map to the same typed run
//! failure everywhere. Presentation consumes the type and never parses prose.

use crate::RunFailureKind;

fn contains_status_code(message: &str, expected: &str) -> bool {
    message
        .split(|character: char| !character.is_ascii_digit())
        .any(|token| token == expected)
}

/// Classify only provider access failures. `None` means the caller must apply
/// its transport-specific provider, schema, or runtime classification.
pub fn classify_provider_access_failure(
    status: Option<u16>,
    message: &str,
) -> Option<RunFailureKind> {
    let lower = message.to_ascii_lowercase();
    if status == Some(429)
        || contains_status_code(&lower, "429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        return Some(RunFailureKind::RateLimited);
    }
    if status == Some(402)
        || contains_status_code(&lower, "402")
        || lower.contains("payment required")
        || lower.contains("insufficient_credit")
        || lower.contains("insufficient credit")
        || lower.contains("usage_limit_reached")
        || lower.contains("usage limit reached")
        || lower.contains("credit balance exhausted")
        || lower.contains("out of credit")
    {
        return Some(RunFailureKind::InsufficientCredits);
    }
    if matches!(status, Some(401 | 403))
        || contains_status_code(&lower, "401")
        || contains_status_code(&lower, "403")
        || lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("credential")
    {
        return Some(RunFailureKind::PlatformKeyRejected);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_billing_evidence_wins_over_a_forbidden_status() {
        assert_eq!(
            classify_provider_access_failure(
                Some(403),
                "usage_limit_reached: insufficient credits"
            ),
            Some(RunFailureKind::InsufficientCredits),
        );
    }

    #[test]
    fn credentials_are_not_mistaken_for_credit_exhaustion() {
        assert_eq!(
            classify_provider_access_failure(Some(403), "credit service credentials were rejected"),
            Some(RunFailureKind::PlatformKeyRejected),
        );
        assert_eq!(
            classify_provider_access_failure(None, "credit service temporarily unavailable"),
            None,
        );
    }

    #[test]
    fn recognizes_typed_access_failures_without_substring_status_collisions() {
        assert_eq!(
            classify_provider_access_failure(None, "HTTP 429: too many requests"),
            Some(RunFailureKind::RateLimited),
        );
        assert_eq!(
            classify_provider_access_failure(None, "HTTP 1402 upstream failure"),
            None,
        );
    }
}
