use serde::{Deserialize, Serialize};

use crate::fingerprint::canonical_sha256;
use crate::validate::validate_digest;
use crate::{
    AdapterPageRequest, AuthContextDescriptor, CursorHandle, NormalizedRecord, ProtocolError,
    ProtocolResult, ReceiptId, TargetIdentity, ADAPTER_PROTOCOL_VERSION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    AccessDenied,
    AuthenticationExpired,
    RateLimited,
    ServiceUnavailable,
    InvalidScope,
    PolicyRestricted,
    ProviderFailure,
    ProtocolViolation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    RecordLimit,
    ByteLimit,
    Deadline,
    ProviderLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdapterPageOutcome {
    Succeeded {
        final_page: bool,
    },
    Denied {
        reason: FailureReason,
    },
    Unreachable {
        reason: FailureReason,
    },
    Unsupported {
        reason: FailureReason,
    },
    Unsafe {
        reason: FailureReason,
    },
    Stale {
        reason: FailureReason,
    },
    Truncated {
        reason: TruncationReason,
        continuation_available: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionSummary {
    pub source_records_seen: u64,
    pub records_emitted: u64,
    pub fields_omitted: u64,
    pub values_rejected: u64,
}

impl RedactionSummary {
    fn validate(&self, actual_records: usize) -> ProtocolResult<()> {
        if self.records_emitted != actual_records as u64 {
            return Err(ProtocolError::invalid(
                "redaction_summary.records_emitted",
                "must equal the number of normalized records",
            ));
        }
        if self.source_records_seen < self.records_emitted {
            return Err(ProtocolError::invalid(
                "redaction_summary.source_records_seen",
                "cannot be smaller than records_emitted",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPageReceipt {
    pub protocol_version: u16,
    pub receipt_id: ReceiptId,
    pub request: AdapterPageRequest,
    pub target: TargetIdentity,
    pub auth_context: AuthContextDescriptor,
    pub adapter_build_sha256: String,
    pub observed_at_ms: u64,
    pub outcome: AdapterPageOutcome,
    pub records: Vec<NormalizedRecord>,
    pub next_cursor_handle: Option<CursorHandle>,
    pub safe_page_sha256: String,
    pub redaction_summary: RedactionSummary,
}

impl AdapterPageReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: AdapterPageRequest,
        target: TargetIdentity,
        auth_context: AuthContextDescriptor,
        adapter_build_sha256: String,
        observed_at_ms: u64,
        outcome: AdapterPageOutcome,
        records: Vec<NormalizedRecord>,
        next_cursor_handle: Option<CursorHandle>,
        redaction_summary: RedactionSummary,
    ) -> ProtocolResult<Self> {
        let safe_page_sha256 =
            derive_safe_page_sha256(&outcome, &records, &next_cursor_handle, &redaction_summary)?;
        let receipt_id = derive_receipt_id(
            &request,
            &target,
            &auth_context,
            &adapter_build_sha256,
            observed_at_ms,
            &safe_page_sha256,
        )?;
        let receipt = Self {
            protocol_version: ADAPTER_PROTOCOL_VERSION,
            receipt_id,
            request,
            target,
            auth_context,
            adapter_build_sha256,
            observed_at_ms,
            outcome,
            records,
            next_cursor_handle,
            safe_page_sha256,
            redaction_summary,
        };
        receipt.validate_at(observed_at_ms)?;
        Ok(receipt)
    }

    pub fn validate_at(&self, now_ms: u64) -> ProtocolResult<()> {
        if self.protocol_version != ADAPTER_PROTOCOL_VERSION {
            return Err(ProtocolError::invalid(
                "protocol_version",
                format!("must equal {ADAPTER_PROTOCOL_VERSION}"),
            ));
        }
        self.receipt_id.validate()?;
        if let Some(cursor_handle) = &self.next_cursor_handle {
            cursor_handle.validate()?;
        }
        self.request
            .validate(&self.target, &self.auth_context, now_ms)?;
        validate_digest("adapter_build_sha256", &self.adapter_build_sha256)?;
        validate_digest("safe_page_sha256", &self.safe_page_sha256)?;
        if self.request.requested_at_ms > self.observed_at_ms || self.observed_at_ms > now_ms {
            return Err(ProtocolError::invalid(
                "observed_at_ms",
                "must be between request time and current time",
            ));
        }
        if self.records.len() > self.request.limits.max_records as usize {
            return Err(ProtocolError::invalid(
                "records",
                "exceeds the request record limit",
            ));
        }
        for record in &self.records {
            record.validate()?;
            if record.adapter_id != self.request.adapter_id
                || record.provider_type != self.request.query.provider_resource_type
            {
                return Err(ProtocolError::invalid(
                    "record_binding",
                    "record must match the request adapter and provider type",
                ));
            }
            if record
                .fields
                .keys()
                .any(|field| !self.request.query.projection.contains(field))
            {
                return Err(ProtocolError::invalid(
                    "record.fields",
                    "contains a field outside the explicit projection",
                ));
            }
        }
        let serialized_records = serde_json::to_vec(&self.records)?;
        if serialized_records.len() as u64 > self.request.limits.max_response_bytes {
            return Err(ProtocolError::invalid(
                "records",
                "exceeds the request byte limit",
            ));
        }
        self.redaction_summary.validate(self.records.len())?;
        validate_outcome(
            &self.outcome,
            self.records.len(),
            self.next_cursor_handle.is_some(),
        )?;

        let expected_page = derive_safe_page_sha256(
            &self.outcome,
            &self.records,
            &self.next_cursor_handle,
            &self.redaction_summary,
        )?;
        if self.safe_page_sha256 != expected_page {
            return Err(ProtocolError::invalid(
                "safe_page_sha256",
                "does not match normalized page content",
            ));
        }
        let expected_receipt = derive_receipt_id(
            &self.request,
            &self.target,
            &self.auth_context,
            &self.adapter_build_sha256,
            self.observed_at_ms,
            &self.safe_page_sha256,
        )?;
        if self.receipt_id != expected_receipt {
            return Err(ProtocolError::invalid(
                "receipt_id",
                "does not match the target-bound receipt",
            ));
        }
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate_at(self.observed_at_ms)?;
        Ok(self
            .receipt_id
            .as_str()
            .strip_prefix("receipt:")
            .expect("validated receipt identifiers have a fixed prefix")
            .to_owned())
    }
}

fn validate_outcome(
    outcome: &AdapterPageOutcome,
    record_count: usize,
    has_next_cursor: bool,
) -> ProtocolResult<()> {
    let valid = match outcome {
        AdapterPageOutcome::Succeeded { final_page } => {
            (*final_page && !has_next_cursor) || (!*final_page && has_next_cursor)
        }
        AdapterPageOutcome::Truncated {
            continuation_available,
            ..
        } => *continuation_available == has_next_cursor,
        AdapterPageOutcome::Denied { .. }
        | AdapterPageOutcome::Unreachable { .. }
        | AdapterPageOutcome::Unsupported { .. }
        | AdapterPageOutcome::Unsafe { .. }
        | AdapterPageOutcome::Stale { .. } => record_count == 0 && !has_next_cursor,
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::invalid(
            "outcome",
            "is inconsistent with records or continuation handle",
        ))
    }
}

fn derive_safe_page_sha256(
    outcome: &AdapterPageOutcome,
    records: &[NormalizedRecord],
    next_cursor_handle: &Option<CursorHandle>,
    redaction_summary: &RedactionSummary,
) -> ProtocolResult<String> {
    canonical_sha256(&(outcome, records, next_cursor_handle, redaction_summary))
}

fn derive_receipt_id(
    request: &AdapterPageRequest,
    target: &TargetIdentity,
    auth_context: &AuthContextDescriptor,
    adapter_build_sha256: &str,
    observed_at_ms: u64,
    safe_page_sha256: &str,
) -> ProtocolResult<ReceiptId> {
    let digest = canonical_sha256(&(
        request,
        target,
        auth_context,
        adapter_build_sha256,
        observed_at_ms,
        safe_page_sha256,
    ))?;
    ReceiptId::from_digest(digest)
}
