use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value as JsonValue;

use super::{EvidenceObjectRef, ObservationFact, MAX_BATCH_BYTES};

pub(super) fn validate_evidence_ref(evidence: &EvidenceObjectRef) -> Result<(), String> {
    validate_prefixed_digest("evidence id", &evidence.evidence_id, "evidence:")?;
    validate_digest("evidence SHA-256", &evidence.sha256)?;
    if evidence.bucket.is_empty()
        || evidence.key.is_empty()
        || evidence.size_bytes == 0
        || evidence.size_bytes > MAX_BATCH_BYTES as u64
        || evidence
            .version_id
            .as_deref()
            .is_none_or(|version| version.trim().is_empty())
    {
        return Err("evidence object reference is incomplete".into());
    }
    Ok(())
}

pub(super) fn validate_fact(fact: &ObservationFact) -> Result<(), String> {
    fact.subject.validate()?;
    if !fact.attributes.is_object() {
        return Err("observation attributes must be a JSON object".into());
    }
    if fact.evidence_digests.len() > 128 {
        return Err("one observation may cite at most 128 evidence digests".into());
    }
    for digest in &fact.evidence_digests {
        validate_digest("observation evidence digest", digest)?;
    }
    reject_secret_canaries(&fact.attributes)
}

pub(super) fn validate_nonce_time(nonce: &str, requested_at_ms: u64) -> Result<(), String> {
    if nonce.len() < 16 || nonce.len() > 128 || requested_at_ms == 0 {
        return Err("request nonce or time is invalid".into());
    }
    Ok(())
}

pub(super) fn validate_prefixed_digest(
    label: &str,
    value: &str,
    prefix: &str,
) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has the wrong namespace"))?;
    validate_digest(label, digest)
}

pub(super) fn validate_digest(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} must contain 64 hexadecimal characters"));
    }
    Ok(())
}

pub(super) fn validate_namespace(label: &str, value: &str) -> Result<(), String> {
    validate_text(label, value, 1, 128)?;
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
    }) {
        return Err(format!("{label} must use a lowercase portable namespace"));
    }
    Ok(())
}

pub(super) fn validate_text(
    label: &str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), String> {
    if value.len() < minimum
        || value.len() > maximum
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(format!(
            "{label} must contain {minimum}..={maximum} trimmed, non-control bytes"
        ));
    }
    Ok(())
}

fn reject_secret_canaries(value: &JsonValue) -> Result<(), String> {
    let lower = serde_json::to_string(value)
        .map_err(|error| error.to_string())?
        .to_ascii_lowercase();
    const CANARIES: &[&str] = &[
        "-----begin private key-----",
        "aws_secret_access_key=",
        "github_token=",
        "authorization: bearer ",
        "xoxb-",
        "ghp_",
        "sk_live_",
    ];
    if CANARIES.iter().any(|canary| lower.contains(canary)) {
        return Err("observation attributes appear to contain a secret value".into());
    }
    Ok(())
}

pub(super) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let value = serde_json::to_value(value).map_err(|error| error.to_string())?;
    serde_json::to_vec(&sort_json(value)).map_err(|error| error.to_string())
}

fn sort_json(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(values) => JsonValue::Array(values.into_iter().map(sort_json).collect()),
        JsonValue::Object(values) => JsonValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}
