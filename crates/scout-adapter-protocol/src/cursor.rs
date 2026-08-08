use serde::{Deserialize, Serialize};

use crate::validate::validate_digest;
use crate::{
    AdapterId, AdapterPageReceipt, AdapterPageRequest, AuthContextDescriptor, AuthContextHandle,
    AuthContextId, CursorHandle, ProtocolError, ProtocolResult, ReceiptId, TargetId,
    TargetIdentity, ADAPTER_PROTOCOL_VERSION,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorVaultBinding {
    pub protocol_version: u16,
    pub cursor_handle: CursorHandle,
    pub source_receipt_id: ReceiptId,
    pub target_id: TargetId,
    pub target_identity_sha256: String,
    pub adapter_id: AdapterId,
    pub auth_context_handle: AuthContextHandle,
    pub auth_context_id: AuthContextId,
    pub coverage_sha256: String,
    pub query_sha256: String,
    pub request_binding_sha256: String,
    pub next_page_ordinal: u32,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

impl CursorVaultBinding {
    pub fn for_next_page(
        receipt: &AdapterPageReceipt,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> ProtocolResult<Self> {
        receipt.validate_at(issued_at_ms)?;
        let cursor_handle = receipt.next_cursor_handle.clone().ok_or_else(|| {
            ProtocolError::invalid(
                "next_cursor_handle",
                "a continuation receipt is required to bind a cursor",
            )
        })?;
        let next_page_ordinal = receipt
            .request
            .page_ordinal
            .checked_add(1)
            .ok_or_else(|| ProtocolError::invalid("page_ordinal", "overflowed"))?;
        let binding = Self {
            protocol_version: ADAPTER_PROTOCOL_VERSION,
            cursor_handle,
            source_receipt_id: receipt.receipt_id.clone(),
            target_id: receipt.request.target_id.clone(),
            target_identity_sha256: receipt.request.target_identity_sha256.clone(),
            adapter_id: receipt.request.adapter_id.clone(),
            auth_context_handle: receipt.request.auth_context_handle.clone(),
            auth_context_id: receipt.request.auth_context_id.clone(),
            coverage_sha256: receipt.request.coverage.fingerprint_sha256()?,
            query_sha256: receipt.request.query.fingerprint_sha256()?,
            request_binding_sha256: receipt.request.binding_fingerprint_sha256()?,
            next_page_ordinal,
            issued_at_ms,
            expires_at_ms,
        };
        binding.validate_lifetime(issued_at_ms)?;
        Ok(binding)
    }

    pub fn authorize(
        &self,
        request: &AdapterPageRequest,
        target: &TargetIdentity,
        auth_context: &AuthContextDescriptor,
        now_ms: u64,
    ) -> ProtocolResult<()> {
        self.validate_lifetime(now_ms)?;
        request
            .validate(target, auth_context, now_ms)
            .map_err(|_| ProtocolError::CursorBinding {
                reason: "request failed target and authorization validation".to_owned(),
            })?;
        let presented =
            request
                .cursor_handle
                .as_ref()
                .ok_or_else(|| ProtocolError::CursorBinding {
                    reason: "request does not present a cursor handle".to_owned(),
                })?;
        let coverage_sha256 = request.coverage.fingerprint_sha256()?;
        let query_sha256 = request.query.fingerprint_sha256()?;
        let request_binding_sha256 = request.binding_fingerprint_sha256()?;
        if self.protocol_version != request.protocol_version
            || &self.cursor_handle != presented
            || self.target_id != request.target_id
            || self.target_identity_sha256 != request.target_identity_sha256
            || self.adapter_id != request.adapter_id
            || self.auth_context_handle != request.auth_context_handle
            || self.auth_context_id != request.auth_context_id
            || self.coverage_sha256 != coverage_sha256
            || self.query_sha256 != query_sha256
            || self.request_binding_sha256 != request_binding_sha256
            || self.next_page_ordinal != request.page_ordinal
        {
            return Err(ProtocolError::CursorBinding {
                reason:
                    "cursor is not bound to this target, authorization, coverage, query, and page"
                        .to_owned(),
            });
        }
        Ok(())
    }

    fn validate_lifetime(&self, now_ms: u64) -> ProtocolResult<()> {
        if self.protocol_version != ADAPTER_PROTOCOL_VERSION {
            return Err(ProtocolError::CursorBinding {
                reason: "protocol version mismatch".to_owned(),
            });
        }
        self.cursor_handle.validate()?;
        self.source_receipt_id.validate()?;
        self.target_id.validate()?;
        validate_digest("target_identity_sha256", &self.target_identity_sha256)?;
        self.adapter_id.validate()?;
        self.auth_context_handle.validate()?;
        self.auth_context_id.validate()?;
        validate_digest("coverage_sha256", &self.coverage_sha256)?;
        validate_digest("query_sha256", &self.query_sha256)?;
        validate_digest("request_binding_sha256", &self.request_binding_sha256)?;
        if self.expires_at_ms <= self.issued_at_ms {
            return Err(ProtocolError::CursorBinding {
                reason: "cursor expiry must follow issuance".to_owned(),
            });
        }
        if self.issued_at_ms > now_ms {
            return Err(ProtocolError::CursorBinding {
                reason: "cursor was issued in the future".to_owned(),
            });
        }
        if self.expires_at_ms <= now_ms {
            return Err(ProtocolError::CursorBinding {
                reason: "cursor binding has expired".to_owned(),
            });
        }
        Ok(())
    }
}
