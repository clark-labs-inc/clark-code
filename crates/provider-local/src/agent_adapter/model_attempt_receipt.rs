use agent_core::domain as desktop;
use agent_core::ids::RunId;
use async_channel::Sender;
use serde_json::json;

use crate::llm::{ProviderFailureContext, ProviderResponseMetadata};

/// Emit only the model-attempt fields safe for the public event stream.
///
/// Full response metadata remains private: it contains request/session
/// identities and transport details needed for recovery, while callers need a
/// truthful route receipt even when the provider response is discarded.
pub(super) fn emit_model_attempt_receipt(
    events: &Sender<desktop::AgentEvent>,
    run: &RunId,
    metadata: Option<&ProviderResponseMetadata>,
    failure: Option<&ProviderFailureContext>,
    outcome: &'static str,
) {
    let requested_model = metadata
        .and_then(|value| value.requested_model.as_deref())
        .or_else(|| failure.map(|value| value.model.as_str()));
    let resolved_model = metadata
        .and_then(|value| value.resolved_model.as_deref())
        .or_else(|| failure.map(|value| value.model.as_str()));
    let provider = metadata
        .and_then(|value| value.provider.as_deref())
        .or_else(|| failure.map(|value| value.provider_route.as_str()));
    let provider_route = metadata
        .and_then(|value| value.provider_route.as_deref())
        .or_else(|| failure.map(|value| value.provider_route.as_str()));
    let attempt = metadata
        .and_then(|value| value.request_attempts)
        .or_else(|| failure.map(|value| value.attempts));

    let _ = events.try_send(desktop::AgentEvent::Trace {
        run: Some(run.clone()),
        // Keep the established public provenance source so CLI, remote
        // workers, and benchmark collectors see failed attempts too.
        source: "model_response".to_string(),
        payload: json!({
            "requested_model": requested_model,
            "fallback_model": metadata.and_then(|value| value.fallback_model.as_deref()),
            "fallback_reason": metadata.and_then(|value| value.fallback_reason.as_deref()),
            "resolved_model": resolved_model,
            "provider": provider,
            "provider_route": provider_route,
            "request_attempts": attempt,
            "rate_limit_retries": metadata.and_then(|value| value.rate_limit_retries),
            "outcome": outcome,
            "attempt": attempt,
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::recovery::{ProviderIncidentCategory, ProviderRetryCounts};

    #[test]
    fn failure_receipt_projects_route_without_private_recovery_fields() {
        let (events, receiver) = async_channel::unbounded();
        let failure = ProviderFailureContext {
            category: ProviderIncidentCategory::ConnectionLost,
            message: "private provider detail".into(),
            model: "requested/model".into(),
            provider_route: "gateway.test/v1".into(),
            provider_status: None,
            provider_error_type: Some("private_error_type".into()),
            idempotency_key: "private-idempotency-key".into(),
            provider_request_id: Some("private-request-id".into()),
            attempts: 2,
            max_attempts: 4,
            retries: ProviderRetryCounts {
                transient: 1,
                ..Default::default()
            },
            output_started: false,
            request_started_at_ms: 1,
            observed_at_ms: 2,
        };

        emit_model_attempt_receipt(
            &events,
            &RunId::new("run-1"),
            None,
            Some(&failure),
            "failed",
        );

        let desktop::AgentEvent::Trace {
            source, payload, ..
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected model attempt receipt");
        };
        assert_eq!(source, "model_response");
        assert_eq!(payload["requested_model"], "requested/model");
        assert_eq!(payload["resolved_model"], "requested/model");
        assert_eq!(payload["provider"], "gateway.test/v1");
        assert_eq!(payload["outcome"], "failed");
        assert_eq!(payload["attempt"], 2);
        assert!(payload.get("message").is_none());
        assert!(!payload.to_string().contains("private-idempotency-key"));
        assert!(!payload.to_string().contains("private-request-id"));
        assert!(!payload.to_string().contains("private_error_type"));
    }
}
