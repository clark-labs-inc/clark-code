use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::fingerprint::canonical_sha256;
use crate::validate::{
    validate_digest, validate_field_name, validate_fields, validate_identifier, validate_safe_text,
    validate_string_set,
};
use crate::{
    AdapterId, AuthContextHandle, AuthContextId, CursorHandle, ProtocolError, ProtocolResult,
    RequestId, SafeFieldValue, TargetId,
};

pub const ADAPTER_PROTOCOL_VERSION: u16 = 3;

const MAX_QUERY_PAGE_SIZE: u32 = 1_000;
const MAX_RECORD_LIMIT: u32 = 10_000;
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DURATION_MS: u64 = 5 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetIdentity {
    pub protocol_version: u16,
    pub target_id: TargetId,
    pub identity_key_sha256: String,
    pub session_nonce_sha256: String,
    pub root_sha256: String,
    pub adapter_host_sha256: String,
    pub platform: String,
    pub architecture: String,
}

impl TargetIdentity {
    pub fn new(
        identity_key_sha256: String,
        session_nonce_sha256: String,
        root_sha256: String,
        adapter_host_sha256: String,
        platform: String,
        architecture: String,
    ) -> ProtocolResult<Self> {
        validate_digest("identity_key_sha256", &identity_key_sha256)?;
        let target_id = TargetId::from_digest(identity_key_sha256.clone())?;
        let identity = Self {
            protocol_version: ADAPTER_PROTOCOL_VERSION,
            target_id,
            identity_key_sha256,
            session_nonce_sha256,
            root_sha256,
            adapter_host_sha256,
            platform,
            architecture,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> ProtocolResult<()> {
        validate_version(self.protocol_version)?;
        self.target_id.validate()?;
        validate_digest("identity_key_sha256", &self.identity_key_sha256)?;
        validate_digest("session_nonce_sha256", &self.session_nonce_sha256)?;
        validate_digest("root_sha256", &self.root_sha256)?;
        validate_digest("adapter_host_sha256", &self.adapter_host_sha256)?;
        validate_identifier("platform", &self.platform, 64)?;
        validate_identifier("architecture", &self.architecture, 64)?;
        let expected = TargetId::from_digest(self.identity_key_sha256.clone())?;
        if self.target_id != expected {
            return Err(ProtocolError::invalid(
                "target_id",
                "does not match identity_key_sha256",
            ));
        }
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate()?;
        canonical_sha256(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSourceKind {
    EnvironmentReference,
    CliProfile,
    WorkloadIdentity,
    InstanceMetadata,
    OsCredentialStore,
    BrokeredSession,
    Anonymous,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthContextDescriptor {
    pub context_id: AuthContextId,
    pub handle: AuthContextHandle,
    pub target_id: TargetId,
    pub adapter_id: AdapterId,
    pub provider: String,
    pub authority_scope: String,
    pub principal_native_id: String,
    pub source_kind: AuthSourceKind,
    pub grant_boundary_sha256: String,
    pub verified_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

impl AuthContextDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: AuthContextHandle,
        target_id: TargetId,
        adapter_id: AdapterId,
        provider: String,
        authority_scope: String,
        principal_native_id: String,
        source_kind: AuthSourceKind,
        grant_boundary_sha256: String,
        verified_at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> ProtocolResult<Self> {
        let context_id = derive_auth_context_id(
            &target_id,
            &adapter_id,
            &provider,
            &authority_scope,
            &principal_native_id,
            &grant_boundary_sha256,
        )?;
        let descriptor = Self {
            context_id,
            handle,
            target_id,
            adapter_id,
            provider,
            authority_scope,
            principal_native_id,
            source_kind,
            grant_boundary_sha256,
            verified_at_ms,
            expires_at_ms,
        };
        descriptor.validate_at(verified_at_ms)?;
        Ok(descriptor)
    }

    pub fn validate_at(&self, now_ms: u64) -> ProtocolResult<()> {
        self.context_id.validate()?;
        self.handle.validate()?;
        self.target_id.validate()?;
        self.adapter_id.validate()?;
        validate_identifier("provider", &self.provider, 64)?;
        validate_safe_text("authority_scope", &self.authority_scope, 512)?;
        validate_safe_text("principal_native_id", &self.principal_native_id, 512)?;
        validate_digest("grant_boundary_sha256", &self.grant_boundary_sha256)?;
        if self.verified_at_ms > now_ms {
            return Err(ProtocolError::invalid(
                "verified_at_ms",
                "cannot be in the future",
            ));
        }
        if self
            .expires_at_ms
            .is_some_and(|expires_at| expires_at <= now_ms || expires_at <= self.verified_at_ms)
        {
            return Err(ProtocolError::invalid(
                "expires_at_ms",
                "must be after verification and current time",
            ));
        }
        let expected = derive_auth_context_id(
            &self.target_id,
            &self.adapter_id,
            &self.provider,
            &self.authority_scope,
            &self.principal_native_id,
            &self.grant_boundary_sha256,
        )?;
        if self.context_id != expected {
            return Err(ProtocolError::invalid(
                "context_id",
                "does not match the target-bound authorization descriptor",
            ));
        }
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate_at(self.verified_at_ms)?;
        canonical_sha256(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageBinding {
    pub enterprise_id: String,
    pub charter_id: String,
    pub discovery_epoch: u64,
    pub sequence: u64,
    pub adapter_id: AdapterId,
    pub auth_context_id: AuthContextId,
    pub tenant: String,
    pub region_or_project: String,
    pub resource_kind: String,
}

impl CoverageBinding {
    pub fn validate(&self) -> ProtocolResult<()> {
        self.adapter_id.validate()?;
        self.auth_context_id.validate()?;
        if self.discovery_epoch == 0 || self.sequence == 0 {
            return Err(ProtocolError::invalid(
                "coverage",
                "discovery epoch and sequence must be positive",
            ));
        }
        validate_safe_text("enterprise_id", &self.enterprise_id, 256)?;
        validate_safe_text("charter_id", &self.charter_id, 256)?;
        validate_safe_text("tenant", &self.tenant, 512)?;
        validate_safe_text("region_or_project", &self.region_or_project, 512)?;
        validate_identifier("resource_kind", &self.resource_kind, 128)?;
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate()?;
        canonical_sha256(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterQuery {
    pub operation: String,
    pub authority_scope: String,
    pub provider_resource_type: String,
    pub filters: BTreeMap<String, SafeFieldValue>,
    pub projection: BTreeSet<String>,
    pub page_size: u32,
}

impl AdapterQuery {
    pub fn validate(&self) -> ProtocolResult<()> {
        validate_identifier("operation", &self.operation, 128)?;
        validate_safe_text("authority_scope", &self.authority_scope, 512)?;
        validate_safe_text("provider_resource_type", &self.provider_resource_type, 256)?;
        validate_fields(&self.filters, 64)?;
        if self.projection.is_empty() {
            return Err(ProtocolError::invalid(
                "projection",
                "must explicitly allow at least one safe field",
            ));
        }
        validate_string_set("projection", &self.projection, 128, 128)?;
        for field in &self.projection {
            validate_field_name(field)?;
        }
        if self.page_size == 0 || self.page_size > MAX_QUERY_PAGE_SIZE {
            return Err(ProtocolError::invalid(
                "page_size",
                format!("must be between 1 and {MAX_QUERY_PAGE_SIZE}"),
            ));
        }
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate()?;
        canonical_sha256(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPageLimits {
    pub max_records: u32,
    pub max_response_bytes: u64,
    pub max_duration_ms: u64,
}

impl AdapterPageLimits {
    pub fn validate(&self) -> ProtocolResult<()> {
        if self.max_records == 0 || self.max_records > MAX_RECORD_LIMIT {
            return Err(ProtocolError::invalid(
                "max_records",
                format!("must be between 1 and {MAX_RECORD_LIMIT}"),
            ));
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_RESPONSE_BYTES {
            return Err(ProtocolError::invalid(
                "max_response_bytes",
                format!("must be between 1 and {MAX_RESPONSE_BYTES}"),
            ));
        }
        if self.max_duration_ms == 0 || self.max_duration_ms > MAX_DURATION_MS {
            return Err(ProtocolError::invalid(
                "max_duration_ms",
                format!("must be between 1 and {MAX_DURATION_MS}"),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPageRequest {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub target_id: TargetId,
    pub target_identity_sha256: String,
    pub adapter_id: AdapterId,
    pub auth_context_handle: AuthContextHandle,
    pub auth_context_id: AuthContextId,
    pub coverage: CoverageBinding,
    pub query: AdapterQuery,
    pub page_ordinal: u32,
    pub cursor_handle: Option<CursorHandle>,
    pub limits: AdapterPageLimits,
    pub requested_at_ms: u64,
}

impl AdapterPageRequest {
    pub fn validate(
        &self,
        target: &TargetIdentity,
        auth: &AuthContextDescriptor,
        now_ms: u64,
    ) -> ProtocolResult<()> {
        validate_version(self.protocol_version)?;
        self.request_id.validate()?;
        self.target_id.validate()?;
        validate_digest("target_identity_sha256", &self.target_identity_sha256)?;
        self.adapter_id.validate()?;
        self.auth_context_handle.validate()?;
        self.auth_context_id.validate()?;
        if let Some(cursor_handle) = &self.cursor_handle {
            cursor_handle.validate()?;
        }
        target.validate()?;
        auth.validate_at(now_ms)?;
        self.coverage.validate()?;
        self.query.validate()?;
        self.limits.validate()?;
        if self.requested_at_ms > now_ms {
            return Err(ProtocolError::invalid(
                "requested_at_ms",
                "cannot be in the future",
            ));
        }
        if self.target_id != target.target_id
            || self.target_identity_sha256 != target.fingerprint_sha256()?
            || self.target_id != auth.target_id
            || self.adapter_id != auth.adapter_id
            || self.auth_context_handle != auth.handle
            || self.auth_context_id != auth.context_id
            || self.coverage.adapter_id != self.adapter_id
            || self.coverage.auth_context_id != self.auth_context_id
            || self.coverage.tenant != self.query.authority_scope
            || self.query.authority_scope != auth.authority_scope
        {
            return Err(ProtocolError::invalid(
                "request_binding",
                "target, adapter, authorization, coverage, and query must agree",
            ));
        }
        match (self.page_ordinal, &self.cursor_handle) {
            (0, None) | (1.., Some(_)) => Ok(()),
            (0, Some(_)) => Err(ProtocolError::invalid(
                "cursor_handle",
                "must be absent on the first page",
            )),
            (_, None) => Err(ProtocolError::invalid(
                "cursor_handle",
                "is required after the first page",
            )),
        }
    }

    pub fn binding_fingerprint_sha256(&self) -> ProtocolResult<String> {
        #[derive(Serialize)]
        struct Binding<'a> {
            protocol_version: u16,
            target_id: &'a TargetId,
            target_identity_sha256: &'a str,
            adapter_id: &'a AdapterId,
            auth_context_id: &'a AuthContextId,
            coverage_sha256: String,
            query_sha256: String,
        }
        canonical_sha256(&Binding {
            protocol_version: self.protocol_version,
            target_id: &self.target_id,
            target_identity_sha256: &self.target_identity_sha256,
            adapter_id: &self.adapter_id,
            auth_context_id: &self.auth_context_id,
            coverage_sha256: self.coverage.fingerprint_sha256()?,
            query_sha256: self.query.fingerprint_sha256()?,
        })
    }
}

fn derive_auth_context_id(
    target_id: &TargetId,
    adapter_id: &AdapterId,
    provider: &str,
    authority_scope: &str,
    principal_native_id: &str,
    grant_boundary_sha256: &str,
) -> ProtocolResult<AuthContextId> {
    validate_digest("grant_boundary_sha256", grant_boundary_sha256)?;
    let digest = canonical_sha256(&(
        target_id,
        adapter_id,
        provider,
        authority_scope,
        principal_native_id,
        grant_boundary_sha256,
    ))?;
    AuthContextId::from_digest(digest)
}

fn validate_version(version: u16) -> ProtocolResult<()> {
    if version != ADAPTER_PROTOCOL_VERSION {
        return Err(ProtocolError::invalid(
            "protocol_version",
            format!("must equal {ADAPTER_PROTOCOL_VERSION}"),
        ));
    }
    Ok(())
}
