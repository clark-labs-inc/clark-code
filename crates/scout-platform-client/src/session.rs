use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use scout_ingest_protocol::cartography::{
    BatchAcceptance, BatchEnvelope, CartographyChangePage, CartographyChangeQuery,
    EvidenceCommitRequest, EvidenceObjectRef, EvidenceStatus, EvidenceUploadRequest,
    GraphDeltaCursor, GraphDeltaPage, GraphDeltaQuery, GraphObjectKind, GraphSnapshotCursor,
    GraphSnapshotPage, GraphSnapshotQuery, ObservationEvent, SimulationOverlayCursor,
    SimulationOverlayPage, SimulationOverlayQuery, TaskClaimRequest, TaskClaimResponse,
    TaskCompletion,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::{
    enroll_machine, hex_lower, now_ms, ClarkCartographyClient, ClarkCartographyEnrollmentConfig,
    CollectorMachineIdentity, MachineEnrollment, MachineEnrollmentRequest,
};

/// Host-owned binding for a protected collector identity.
///
/// This type intentionally does not implement `Debug`, `Clone`, or
/// serialization because it contains the Platform API credential.
pub struct ScoutCartographySessionConfig {
    enrollment: ClarkCartographyEnrollmentConfig,
    identity_root: PathBuf,
    organization_id: Uuid,
    workspace_id: Uuid,
    platform: String,
    architecture: String,
}

impl ScoutCartographySessionConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform_base_url: impl AsRef<str>,
        platform_api_key: impl AsRef<str>,
        identity_root: impl AsRef<Path>,
        organization_id: Uuid,
        workspace_id: Uuid,
        platform: impl Into<String>,
        architecture: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            enrollment: ClarkCartographyEnrollmentConfig::new(platform_base_url, platform_api_key)?,
            identity_root: identity_root.as_ref().to_path_buf(),
            organization_id,
            workspace_id,
            platform: platform.into(),
            architecture: architecture.into(),
        })
    }
}

/// Enrolled sensor session pinned to one exact Clark organization/workspace.
///
/// The protected signing key never enters model arguments, graph rows, logs,
/// or HTTP responses. A session cannot submit or retrieve another tenant.
pub struct ScoutCartographySession {
    client: ClarkCartographyClient,
    identity: CollectorMachineIdentity,
    enrollment: MachineEnrollment,
}

impl ScoutCartographySession {
    pub async fn enroll(config: ScoutCartographySessionConfig) -> Result<Self, String> {
        let origin = config.enrollment.base_url.origin().ascii_serialization();
        let binding = format!(
            "{origin}|{}|{}",
            config.organization_id, config.workspace_id
        );
        let identity = CollectorMachineIdentity::load_or_create(&config.identity_root, &binding)?;
        let request = MachineEnrollmentRequest {
            organization_id: config.organization_id,
            workspace_id: config.workspace_id,
            public_key: identity.public_key_hex(),
            platform: config.platform,
            architecture: config.architecture,
        };
        let enrolled = enroll_machine(config.enrollment, &request).await?;
        Ok(Self {
            client: enrolled.client,
            identity,
            enrollment: enrolled.enrollment,
        })
    }

    pub fn enrollment(&self) -> &MachineEnrollment {
        &self.enrollment
    }

    pub fn identity_path(&self) -> &Path {
        self.identity.key_path()
    }

    pub async fn claim_next_task(
        &self,
        run_id: Uuid,
        lease_seconds: i32,
    ) -> Result<TaskClaimResponse, String> {
        let request = TaskClaimRequest::sign(
            self.enrollment.organization_id,
            self.enrollment.workspace_id,
            run_id,
            self.enrollment.id,
            random_nonce("task-claim")?,
            now_ms()?,
            lease_seconds,
            self.identity.signing_key(),
        )?;
        self.client.claim_task(&request).await
    }

    pub async fn ingest(
        &self,
        run_id: Uuid,
        events: Vec<ObservationEvent>,
        completions: Vec<TaskCompletion>,
    ) -> Result<BatchAcceptance, String> {
        let envelope = BatchEnvelope::sign(
            self.enrollment.organization_id,
            self.enrollment.workspace_id,
            run_id,
            self.enrollment.id,
            random_nonce("attempt")?,
            now_ms()?,
            events,
            completions,
            self.identity.signing_key(),
        )?;
        self.client.ingest_batch(&envelope).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upload_evidence(
        &self,
        run_id: Uuid,
        source_id: Uuid,
        task_id: Uuid,
        fence: i64,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<EvidenceObjectRef, String> {
        self.upload_evidence_signed(
            run_id,
            source_id,
            task_id,
            fence,
            random_nonce("evidence-upload")?,
            now_ms()?,
            content_type,
            bytes,
        )
        .await
    }

    /// Upload evidence under a stable signed authorization request.
    ///
    /// Retrying the same binding, bytes, key, and timestamp resolves to the
    /// same backend evidence object and immutable S3 version.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_evidence_idempotent(
        &self,
        run_id: Uuid,
        source_id: Uuid,
        task_id: Uuid,
        fence: i64,
        idempotency_key: &str,
        requested_at_ms: u64,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<EvidenceObjectRef, String> {
        if idempotency_key.is_empty() || idempotency_key.len() > 256 {
            return Err("evidence idempotency key must contain 1 to 256 characters".into());
        }
        let sha256 = hex_lower(&Sha256::digest(bytes));
        let mut nonce_material = Vec::with_capacity(idempotency_key.len() + 160);
        nonce_material.extend_from_slice(b"clark.scout-evidence-idempotency/v1\0");
        nonce_material.extend_from_slice(run_id.as_bytes());
        nonce_material.extend_from_slice(source_id.as_bytes());
        nonce_material.extend_from_slice(task_id.as_bytes());
        nonce_material.extend_from_slice(&fence.to_le_bytes());
        nonce_material.extend_from_slice(content_type.as_bytes());
        nonce_material.extend_from_slice(sha256.as_bytes());
        nonce_material.extend_from_slice(idempotency_key.as_bytes());
        let nonce = format!(
            "evidence-upload:{}",
            hex_lower(&Sha256::digest(&nonce_material))
        );
        self.upload_evidence_signed(
            run_id,
            source_id,
            task_id,
            fence,
            nonce,
            requested_at_ms,
            content_type,
            bytes,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_evidence_signed(
        &self,
        run_id: Uuid,
        source_id: Uuid,
        task_id: Uuid,
        fence: i64,
        nonce: String,
        requested_at_ms: u64,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<EvidenceObjectRef, String> {
        let sha256 = hex_lower(&Sha256::digest(bytes));
        let upload = EvidenceUploadRequest::sign(
            self.enrollment.organization_id,
            self.enrollment.workspace_id,
            run_id,
            source_id,
            self.enrollment.id,
            task_id,
            fence,
            nonce,
            requested_at_ms,
            content_type.to_owned(),
            bytes.len() as u64,
            sha256,
            self.identity.signing_key(),
        )?;
        let grant = self.client.authorize_evidence(&upload).await?;
        if grant.authorization.status == EvidenceStatus::Verified {
            return verified_evidence_ref(grant.authorization);
        }
        self.client.upload_evidence(&grant, bytes).await?;
        let commit = EvidenceCommitRequest::sign(
            self.enrollment.organization_id,
            self.enrollment.workspace_id,
            run_id,
            self.enrollment.id,
            task_id,
            fence,
            upload.evidence_id,
            random_nonce("evidence-commit")?,
            now_ms()?,
            self.identity.signing_key(),
        )?;
        let outcome = self.client.commit_evidence(&commit).await?;
        if outcome.evidence.status != EvidenceStatus::Verified || outcome.rejection_reason.is_some()
        {
            return Err(outcome
                .rejection_reason
                .unwrap_or_else(|| "Clark did not verify the evidence object".into()));
        }
        verified_evidence_ref(outcome.evidence)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn query_snapshot(
        &self,
        effective_at_ms: Option<u64>,
        known_at_ms: Option<u64>,
        object_kinds: BTreeSet<GraphObjectKind>,
        limit: u16,
        cursor: Option<GraphSnapshotCursor>,
    ) -> Result<GraphSnapshotPage, String> {
        self.client
            .query_snapshot(&GraphSnapshotQuery {
                organization_id: self.enrollment.organization_id,
                workspace_id: self.enrollment.workspace_id,
                effective_at_ms,
                known_at_ms,
                object_kinds,
                limit,
                cursor,
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn query_delta(
        &self,
        from_effective_at_ms: u64,
        from_known_at_ms: Option<u64>,
        to_effective_at_ms: Option<u64>,
        to_known_at_ms: Option<u64>,
        object_kinds: BTreeSet<GraphObjectKind>,
        include_unchanged: bool,
        limit: u16,
        cursor: Option<GraphDeltaCursor>,
    ) -> Result<GraphDeltaPage, String> {
        self.client
            .query_delta(&GraphDeltaQuery {
                organization_id: self.enrollment.organization_id,
                workspace_id: self.enrollment.workspace_id,
                from_effective_at_ms,
                from_known_at_ms,
                to_effective_at_ms,
                to_known_at_ms,
                object_kinds,
                include_unchanged,
                limit,
                cursor,
            })
            .await
    }

    pub async fn query_simulation_overlay(
        &self,
        stable_key: String,
        version: Option<u64>,
        limit: u16,
        cursor: Option<SimulationOverlayCursor>,
    ) -> Result<SimulationOverlayPage, String> {
        self.client
            .query_simulation_overlay(&SimulationOverlayQuery {
                organization_id: self.enrollment.organization_id,
                workspace_id: self.enrollment.workspace_id,
                stable_key,
                version,
                limit,
                cursor,
            })
            .await
    }

    pub async fn query_changes(
        &self,
        after_sequence: u64,
        limit: u16,
    ) -> Result<CartographyChangePage, String> {
        self.client
            .query_changes(&CartographyChangeQuery {
                organization_id: self.enrollment.organization_id,
                workspace_id: self.enrollment.workspace_id,
                after_sequence,
                limit,
            })
            .await
    }
}

fn verified_evidence_ref(
    authorization: scout_ingest_protocol::cartography::EvidenceUploadAuthorization,
) -> Result<EvidenceObjectRef, String> {
    if authorization.status != EvidenceStatus::Verified {
        return Err("Clark evidence object is not verified".into());
    }
    let version_id = authorization.version_id.ok_or_else(|| {
        "Clark verified evidence without returning its immutable S3 version id".to_string()
    })?;
    Ok(EvidenceObjectRef {
        evidence_id: authorization.evidence_id,
        bucket: authorization.bucket,
        key: authorization.key,
        sha256: authorization.sha256,
        size_bytes: authorization.size_bytes,
        version_id: Some(version_id),
    })
}

fn random_nonce(namespace: &str) -> Result<String, String> {
    let mut random = [0_u8; 24];
    getrandom::fill(&mut random)
        .map_err(|_| "failed to generate a Scout request nonce".to_string())?;
    Ok(format!("{namespace}:{}", hex_lower(&random)))
}
