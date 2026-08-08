use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::crypto::{sha256_hex, verify_receipt, CollectorSigningKey};

mod graph;
mod validation;
pub use graph::{ClaimIdentity, ClaimTarget, EdgeIdentity, EntityIdentity, ObservationSubject};
use validation::*;

pub const SYSTEM_CARTOGRAPHY_SCHEMA_VERSION: u16 = 2;
pub const MAX_EVENTS_PER_BATCH: usize = 10_000;
pub const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Public,
    Internal,
    Confidential,
    Restricted,
    SecretReferenceOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceObjectRef {
    pub evidence_id: String,
    pub bucket: String,
    pub key: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub version_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFact {
    pub subject: ObservationSubject,
    pub attributes: JsonValue,
    #[serde(default)]
    pub evidence_digests: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvent {
    pub event_id: String,
    pub source_id: Uuid,
    pub task_id: Uuid,
    pub fence: i64,
    pub source_sequence: i64,
    pub observed_at_ms: u64,
    pub classification: Classification,
    pub evidence: EvidenceObjectRef,
    pub fact: ObservationFact,
}

#[derive(Serialize)]
struct EventContent<'a> {
    source_id: Uuid,
    task_id: Uuid,
    fence: i64,
    source_sequence: i64,
    observed_at_ms: u64,
    classification: Classification,
    evidence: &'a EvidenceObjectRef,
    fact: &'a ObservationFact,
}

impl ObservationEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_id: Uuid,
        task_id: Uuid,
        fence: i64,
        source_sequence: i64,
        observed_at_ms: u64,
        classification: Classification,
        evidence: EvidenceObjectRef,
        fact: ObservationFact,
    ) -> Result<Self, String> {
        if fence <= 0 || source_sequence <= 0 || observed_at_ms == 0 {
            return Err("event fence, sequence, and observation time must be positive".into());
        }
        validate_evidence_ref(&evidence)?;
        validate_fact(&fact)?;
        let mut event = Self {
            event_id: String::new(),
            source_id,
            task_id,
            fence,
            source_sequence,
            observed_at_ms,
            classification,
            evidence,
            fact,
        };
        event.event_id = format!("event:{}", sha256_hex(&event.canonical_content()?));
        Ok(event)
    }

    fn canonical_content(&self) -> Result<Vec<u8>, String> {
        canonical_json(&EventContent {
            source_id: self.source_id,
            task_id: self.task_id,
            fence: self.fence,
            source_sequence: self.source_sequence,
            observed_at_ms: self.observed_at_ms,
            classification: self.classification,
            evidence: &self.evidence,
            fact: &self.fact,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalDisposition {
    Supported,
    Empty,
    Denied,
    Unreachable,
    Unsupported,
    Unsafe,
    Stale,
    Truncated,
    Untested,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCompletion {
    pub task_id: Uuid,
    pub fence: i64,
    pub disposition: TerminalDisposition,
    pub evidence_sha256: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEnvelope {
    pub schema_version: u16,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub machine_id: Uuid,
    pub signer_id: String,
    pub attempt_id: String,
    pub batch_id: String,
    pub signed_at_ms: u64,
    pub events: Vec<ObservationEvent>,
    pub completions: Vec<TaskCompletion>,
    pub signature: String,
}

#[derive(Serialize)]
struct BatchContent<'a> {
    schema_version: u16,
    organization_id: Uuid,
    workspace_id: Uuid,
    run_id: Uuid,
    machine_id: Uuid,
    signer_id: &'a str,
    attempt_id: &'a str,
    signed_at_ms: u64,
    event_ids: Vec<&'a str>,
    completions: &'a [TaskCompletion],
}

impl BatchEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        organization_id: Uuid,
        workspace_id: Uuid,
        run_id: Uuid,
        machine_id: Uuid,
        attempt_id: String,
        signed_at_ms: u64,
        mut events: Vec<ObservationEvent>,
        mut completions: Vec<TaskCompletion>,
        signer: &CollectorSigningKey,
    ) -> Result<Self, String> {
        if attempt_id.is_empty() || signed_at_ms == 0 {
            return Err("batch attempt id and signing time are required".into());
        }
        events.sort_by(|left, right| left.event_id.cmp(&right.event_id));
        events.dedup_by(|left, right| left.event_id == right.event_id);
        completions.sort_by_key(|completion| completion.task_id);
        if events.is_empty() && completions.is_empty() {
            return Err("a batch must contain observations or task completions".into());
        }
        if events.len() > MAX_EVENTS_PER_BATCH || completions.len() > MAX_EVENTS_PER_BATCH {
            return Err("batch exceeds the 10,000-row limit".into());
        }
        let completed = completions
            .iter()
            .map(|completion| (completion.task_id, completion.fence))
            .collect::<BTreeSet<_>>();
        if events
            .iter()
            .any(|event| !completed.contains(&(event.task_id, event.fence)))
        {
            return Err("every observation must complete its exact task fence".into());
        }
        let mut envelope = Self {
            schema_version: SYSTEM_CARTOGRAPHY_SCHEMA_VERSION,
            organization_id,
            workspace_id,
            run_id,
            machine_id,
            signer_id: signer.signer_id(),
            attempt_id,
            batch_id: String::new(),
            signed_at_ms,
            events,
            completions,
            signature: String::new(),
        };
        let content = envelope.canonical_content()?;
        envelope.batch_id = format!("batch:{}", sha256_hex(&content));
        envelope.signature = signer.sign_batch(&content);
        Ok(envelope)
    }

    fn canonical_content(&self) -> Result<Vec<u8>, String> {
        canonical_json(&BatchContent {
            schema_version: self.schema_version,
            organization_id: self.organization_id,
            workspace_id: self.workspace_id,
            run_id: self.run_id,
            machine_id: self.machine_id,
            signer_id: &self.signer_id,
            attempt_id: &self.attempt_id,
            signed_at_ms: self.signed_at_ms,
            event_ids: self
                .events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect(),
            completions: &self.completions,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskClaimRequest {
    pub schema_version: u16,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub machine_id: Uuid,
    pub signer_id: String,
    pub nonce: String,
    pub requested_at_ms: u64,
    pub lease_seconds: i32,
    pub request_id: String,
    pub signature: String,
}

#[derive(Serialize)]
struct TaskClaimContent<'a> {
    schema_version: u16,
    organization_id: Uuid,
    workspace_id: Uuid,
    run_id: Uuid,
    machine_id: Uuid,
    signer_id: &'a str,
    nonce: &'a str,
    requested_at_ms: u64,
    lease_seconds: i32,
}

impl TaskClaimRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        organization_id: Uuid,
        workspace_id: Uuid,
        run_id: Uuid,
        machine_id: Uuid,
        nonce: String,
        requested_at_ms: u64,
        lease_seconds: i32,
        signer: &CollectorSigningKey,
    ) -> Result<Self, String> {
        validate_nonce_time(&nonce, requested_at_ms)?;
        if !(5..=3_600).contains(&lease_seconds) {
            return Err("task lease must be within 5..=3600 seconds".into());
        }
        let mut request = Self {
            schema_version: SYSTEM_CARTOGRAPHY_SCHEMA_VERSION,
            organization_id,
            workspace_id,
            run_id,
            machine_id,
            signer_id: signer.signer_id(),
            nonce,
            requested_at_ms,
            lease_seconds,
            request_id: String::new(),
            signature: String::new(),
        };
        let content = canonical_json(&TaskClaimContent {
            schema_version: request.schema_version,
            organization_id,
            workspace_id,
            run_id,
            machine_id,
            signer_id: &request.signer_id,
            nonce: &request.nonce,
            requested_at_ms,
            lease_seconds,
        })?;
        request.request_id = format!("task-claim:{}", sha256_hex(&content));
        request.signature = signer.sign_task_claim(&content);
        Ok(request)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimedTask {
    pub task_id: Uuid,
    pub source_id: Uuid,
    pub task_kind: String,
    pub scope: JsonValue,
    pub fence: i64,
    pub lease_expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskClaimResponse {
    pub request_id: String,
    pub task: Option<ClaimedTask>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceUploadRequest {
    pub schema_version: u16,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub source_id: Uuid,
    pub machine_id: Uuid,
    pub task_id: Uuid,
    pub fence: i64,
    pub signer_id: String,
    pub nonce: String,
    pub requested_at_ms: u64,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub request_id: String,
    pub evidence_id: String,
    pub signature: String,
}

#[derive(Serialize)]
struct EvidenceUploadContent<'a> {
    schema_version: u16,
    organization_id: Uuid,
    workspace_id: Uuid,
    run_id: Uuid,
    source_id: Uuid,
    machine_id: Uuid,
    task_id: Uuid,
    fence: i64,
    signer_id: &'a str,
    nonce: &'a str,
    requested_at_ms: u64,
    content_type: &'a str,
    size_bytes: u64,
    sha256: &'a str,
}

impl EvidenceUploadRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        organization_id: Uuid,
        workspace_id: Uuid,
        run_id: Uuid,
        source_id: Uuid,
        machine_id: Uuid,
        task_id: Uuid,
        fence: i64,
        nonce: String,
        requested_at_ms: u64,
        content_type: String,
        size_bytes: u64,
        sha256: String,
        signer: &CollectorSigningKey,
    ) -> Result<Self, String> {
        validate_nonce_time(&nonce, requested_at_ms)?;
        if fence <= 0 || size_bytes == 0 || size_bytes > MAX_BATCH_BYTES as u64 {
            return Err("evidence fence or size is invalid".into());
        }
        if !matches!(
            content_type.as_str(),
            "application/json" | "application/octet-stream" | "application/zstd"
        ) {
            return Err("evidence content type is not supported".into());
        }
        validate_digest("evidence SHA-256", &sha256)?;
        let mut request = Self {
            schema_version: SYSTEM_CARTOGRAPHY_SCHEMA_VERSION,
            organization_id,
            workspace_id,
            run_id,
            source_id,
            machine_id,
            task_id,
            fence,
            signer_id: signer.signer_id(),
            nonce,
            requested_at_ms,
            content_type,
            size_bytes,
            sha256,
            request_id: String::new(),
            evidence_id: String::new(),
            signature: String::new(),
        };
        let content = canonical_json(&EvidenceUploadContent {
            schema_version: request.schema_version,
            organization_id,
            workspace_id,
            run_id,
            source_id,
            machine_id,
            task_id,
            fence,
            signer_id: &request.signer_id,
            nonce: &request.nonce,
            requested_at_ms,
            content_type: &request.content_type,
            size_bytes,
            sha256: &request.sha256,
        })?;
        let digest = sha256_hex(&content);
        request.request_id = format!("evidence-upload:{digest}");
        request.evidence_id = format!("evidence:{digest}");
        request.signature = signer.sign_evidence_upload(&content);
        Ok(request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCommitRequest {
    pub schema_version: u16,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub machine_id: Uuid,
    pub task_id: Uuid,
    pub fence: i64,
    pub signer_id: String,
    pub evidence_id: String,
    pub nonce: String,
    pub requested_at_ms: u64,
    pub request_id: String,
    pub signature: String,
}

#[derive(Serialize)]
struct EvidenceCommitContent<'a> {
    schema_version: u16,
    organization_id: Uuid,
    workspace_id: Uuid,
    run_id: Uuid,
    machine_id: Uuid,
    task_id: Uuid,
    fence: i64,
    signer_id: &'a str,
    evidence_id: &'a str,
    nonce: &'a str,
    requested_at_ms: u64,
}

impl EvidenceCommitRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        organization_id: Uuid,
        workspace_id: Uuid,
        run_id: Uuid,
        machine_id: Uuid,
        task_id: Uuid,
        fence: i64,
        evidence_id: String,
        nonce: String,
        requested_at_ms: u64,
        signer: &CollectorSigningKey,
    ) -> Result<Self, String> {
        validate_nonce_time(&nonce, requested_at_ms)?;
        validate_prefixed_digest("evidence id", &evidence_id, "evidence:")?;
        if fence <= 0 {
            return Err("evidence commit fence must be positive".into());
        }
        let mut request = Self {
            schema_version: SYSTEM_CARTOGRAPHY_SCHEMA_VERSION,
            organization_id,
            workspace_id,
            run_id,
            machine_id,
            task_id,
            fence,
            signer_id: signer.signer_id(),
            evidence_id,
            nonce,
            requested_at_ms,
            request_id: String::new(),
            signature: String::new(),
        };
        let content = canonical_json(&EvidenceCommitContent {
            schema_version: request.schema_version,
            organization_id,
            workspace_id,
            run_id,
            machine_id,
            task_id,
            fence,
            signer_id: &request.signer_id,
            evidence_id: &request.evidence_id,
            nonce: &request.nonce,
            requested_at_ms,
        })?;
        request.request_id = format!("evidence-commit:{}", sha256_hex(&content));
        request.signature = signer.sign_evidence_commit(&content);
        Ok(request)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Pending,
    Verified,
    Rejected,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceUploadAuthorization {
    pub evidence_id: String,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub source_id: Uuid,
    pub machine_id: Uuid,
    pub task_id: Uuid,
    pub fence: i64,
    pub bucket: String,
    pub key: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub version_id: Option<String>,
    pub expires_at_ms: u64,
    pub status: EvidenceStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadHeader {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceUploadGrant {
    pub authorization: EvidenceUploadAuthorization,
    pub upload_url: Option<String>,
    pub upload_headers: Vec<UploadHeader>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCommitOutcome {
    pub evidence: EvidenceUploadAuthorization,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchReceipt {
    pub schema_version: u16,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub batch_id: String,
    pub envelope_sha256: String,
    pub sequence: i64,
    pub accepted_at_ms: u64,
    pub previous_receipt_id: Option<String>,
    pub coordinator_id: String,
    pub coordinator_public_key: String,
    pub receipt_id: String,
    pub signature: String,
}

#[derive(Serialize)]
struct ReceiptContent<'a> {
    schema_version: u16,
    organization_id: Uuid,
    workspace_id: Uuid,
    batch_id: &'a str,
    envelope_sha256: &'a str,
    sequence: i64,
    accepted_at_ms: u64,
    previous_receipt_id: &'a Option<String>,
    coordinator_id: &'a str,
    coordinator_public_key: &'a str,
}

impl BatchReceipt {
    pub fn verify(&self, expected_public_key: &str) -> Result<(), String> {
        if self.coordinator_public_key != expected_public_key {
            return Err("receipt is not signed by the pinned backend coordinator".into());
        }
        let content = canonical_json(&ReceiptContent {
            schema_version: self.schema_version,
            organization_id: self.organization_id,
            workspace_id: self.workspace_id,
            batch_id: &self.batch_id,
            envelope_sha256: &self.envelope_sha256,
            sequence: self.sequence,
            accepted_at_ms: self.accepted_at_ms,
            previous_receipt_id: &self.previous_receipt_id,
            coordinator_id: &self.coordinator_id,
            coordinator_public_key: &self.coordinator_public_key,
        })?;
        let expected_id = format!("receipt:{}", sha256_hex(&content));
        if self.receipt_id != expected_id {
            return Err("receipt id does not match its canonical content".into());
        }
        verify_receipt(
            &self.coordinator_public_key,
            &self.coordinator_id,
            &self.signature,
            &content,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchAcceptance {
    pub outcome: ReceiptOutcome,
    pub receipt: BatchReceipt,
    pub inserted_events: usize,
    pub recorded_conflicts: usize,
}
