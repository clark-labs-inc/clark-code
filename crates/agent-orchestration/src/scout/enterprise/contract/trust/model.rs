use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::crypto::{
    authenticated_batch_payload_id, validate_public_key, validate_signature, AuthTranscript,
    EnterpriseSigningKey,
};
use crate::scout::enterprise::contract::{
    canonical_digest, EnterpriseBatch, EnterpriseId, EnterpriseProvenance,
};

pub const ENTERPRISE_TRUST_SCHEMA_VERSION: u16 = 1;
pub const ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION: u16 = 2;
const MAX_COORDINATORS: usize = 64;
const MAX_APPROVALS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseSignerRole {
    Coordinator,
    Collector,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseTrustManifest {
    pub schema_version: u16,
    pub manifest_id: String,
    pub enterprise_id: EnterpriseId,
    pub trust_domain_id: String,
    pub generation: u64,
    pub previous_manifest_id: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub coordinator_threshold: u16,
    pub coordinators: BTreeMap<String, String>,
    #[serde(default)]
    pub revoked_signer_ids: BTreeMap<String, u64>,
    #[serde(default)]
    pub revoked_grant_ids: BTreeMap<String, u64>,
    pub approvals: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterpriseTrustPolicy {
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub coordinator_threshold: u16,
    pub coordinators: BTreeMap<String, String>,
    pub revoked_signer_ids: BTreeMap<String, u64>,
    pub revoked_grant_ids: BTreeMap<String, u64>,
}

#[derive(Serialize)]
struct ManifestContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    trust_domain_id: &'a str,
    generation: u64,
    previous_manifest_id: &'a Option<String>,
    issued_at_ms: u64,
    expires_at_ms: u64,
    coordinator_threshold: u16,
    coordinators: &'a BTreeMap<String, String>,
    revoked_signer_ids: &'a BTreeMap<String, u64>,
    revoked_grant_ids: &'a BTreeMap<String, u64>,
}

impl EnterpriseTrustManifest {
    pub fn initial(
        enterprise_id: EnterpriseId,
        trust_domain_id: String,
        issued_at_ms: u64,
        expires_at_ms: u64,
        coordinator: &EnterpriseSigningKey,
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            manifest_id: String::new(),
            enterprise_id,
            trust_domain_id,
            generation: 1,
            previous_manifest_id: None,
            issued_at_ms,
            expires_at_ms,
            coordinator_threshold: 1,
            coordinators: BTreeMap::from([(coordinator.signer_id(), coordinator.public_key_hex())]),
            revoked_signer_ids: BTreeMap::new(),
            revoked_grant_ids: BTreeMap::new(),
            approvals: BTreeMap::new(),
        };
        value.manifest_id = value.content_id()?;
        let signer_id = coordinator.signer_id();
        value.approvals.insert(
            signer_id.clone(),
            coordinator.sign(&AuthTranscript {
                kind: "trust_manifest",
                enterprise_id: value.enterprise_id.as_str(),
                payload_id: &value.manifest_id,
                manifest_id: &value.manifest_id,
                grant_id: "",
                signer_id: &signer_id,
            }),
        );
        value.validate_shape()?;
        Ok(value)
    }

    pub fn successor(
        parent: &Self,
        policy: EnterpriseTrustPolicy,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            manifest_id: String::new(),
            enterprise_id: parent.enterprise_id.clone(),
            trust_domain_id: parent.trust_domain_id.clone(),
            generation: parent.generation + 1,
            previous_manifest_id: Some(parent.manifest_id.clone()),
            issued_at_ms: policy.issued_at_ms,
            expires_at_ms: policy.expires_at_ms,
            coordinator_threshold: policy.coordinator_threshold,
            coordinators: policy.coordinators,
            revoked_signer_ids: policy.revoked_signer_ids,
            revoked_grant_ids: policy.revoked_grant_ids,
            approvals: BTreeMap::new(),
        };
        value.manifest_id = value.content_id()?;
        for key in approvers {
            let signer_id = key.signer_id();
            value.approvals.insert(
                signer_id.clone(),
                key.sign(&AuthTranscript {
                    kind: "trust_manifest",
                    enterprise_id: value.enterprise_id.as_str(),
                    payload_id: &value.manifest_id,
                    manifest_id: &parent.manifest_id,
                    grant_id: "",
                    signer_id: &signer_id,
                }),
            );
        }
        value.validate_shape()?;
        Ok(value)
    }

    pub(super) fn content_id(&self) -> Result<String, String> {
        Ok(format!(
            "trust-manifest:{}",
            canonical_digest(&ManifestContent {
                schema_version: self.schema_version,
                enterprise_id: &self.enterprise_id,
                trust_domain_id: &self.trust_domain_id,
                generation: self.generation,
                previous_manifest_id: &self.previous_manifest_id,
                issued_at_ms: self.issued_at_ms,
                expires_at_ms: self.expires_at_ms,
                coordinator_threshold: self.coordinator_threshold,
                coordinators: &self.coordinators,
                revoked_signer_ids: &self.revoked_signer_ids,
                revoked_grant_ids: &self.revoked_grant_ids,
            })?
        ))
    }

    pub(super) fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_TRUST_SCHEMA_VERSION {
            return Err(format!(
                "unsupported enterprise trust schema {}",
                self.schema_version
            ));
        }
        if self.content_id()? != self.manifest_id {
            return Err("trust manifest content digest mismatch".into());
        }
        validate_prefixed_digest("trust manifest", &self.manifest_id, "trust-manifest:")?;
        if !self.trust_domain_id.starts_with("trust:") || self.trust_domain_id.len() > 128 {
            return Err("trust domain id must use the trust: namespace".into());
        }
        if self.generation == 0 || self.issued_at_ms == 0 || self.expires_at_ms <= self.issued_at_ms
        {
            return Err("trust manifest generation and validity interval are invalid".into());
        }
        if self.coordinators.is_empty() || self.coordinators.len() > MAX_COORDINATORS {
            return Err("trust manifest coordinator count is outside 1..=64".into());
        }
        if self.coordinator_threshold == 0
            || usize::from(self.coordinator_threshold) > self.coordinators.len()
        {
            return Err("trust manifest coordinator threshold is invalid".into());
        }
        if self.approvals.len() > MAX_APPROVALS {
            return Err("trust manifest has too many approvals".into());
        }
        for (signer, key) in &self.coordinators {
            validate_prefixed_digest("signer", signer, "signer:")?;
            validate_public_key(key)?;
        }
        for (signer, signature) in &self.approvals {
            validate_prefixed_digest("approval signer", signer, "signer:")?;
            validate_signature(signature)?;
        }
        for (signer, effective_at_ms) in &self.revoked_signer_ids {
            validate_prefixed_digest("revoked signer", signer, "signer:")?;
            if *effective_at_ms == 0 {
                return Err("signer revocation time must be positive".into());
            }
        }
        for (grant, effective_at_ms) in &self.revoked_grant_ids {
            validate_prefixed_digest("revoked signer grant", grant, "grant:")?;
            if *effective_at_ms == 0 {
                return Err("grant revocation time must be positive".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseGrantScope {
    pub machine_id: String,
    pub run_id: String,
    pub adapter_instance_id: String,
    pub auth_context_id: String,
    pub discovery_epoch: String,
    pub discovery_epoch_sequence: u64,
    pub first_source_sequence: u64,
    pub last_source_sequence: u64,
}

impl EnterpriseGrantScope {
    pub(super) fn authorizes(&self, provenance: &EnterpriseProvenance) -> bool {
        self.machine_id == provenance.machine_id
            && self.run_id == provenance.run_id
            && self.adapter_instance_id == provenance.adapter_instance_id
            && self.auth_context_id == provenance.auth_context_id
            && self.discovery_epoch == provenance.discovery_epoch
            && self.discovery_epoch_sequence == provenance.discovery_epoch_sequence
            && (self.first_source_sequence..=self.last_source_sequence)
                .contains(&provenance.source_sequence)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.first_source_sequence == 0 || self.last_source_sequence < self.first_source_sequence
        {
            return Err("signer grant source-sequence bounds are invalid".into());
        }
        let probe = EnterpriseProvenance {
            machine_id: self.machine_id.clone(),
            run_id: self.run_id.clone(),
            adapter_instance_id: self.adapter_instance_id.clone(),
            auth_context_id: self.auth_context_id.clone(),
            discovery_epoch: self.discovery_epoch.clone(),
            discovery_epoch_sequence: self.discovery_epoch_sequence,
            source_sequence: self.first_source_sequence,
            observed_at_ms: 1,
            source_fingerprint: "0".repeat(64),
        };
        probe.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseSignerGrant {
    pub schema_version: u16,
    pub grant_id: String,
    pub enterprise_id: EnterpriseId,
    pub manifest_id: String,
    pub signer_id: String,
    pub signer_public_key: String,
    pub roles: BTreeSet<EnterpriseSignerRole>,
    pub scope: EnterpriseGrantScope,
    pub not_before_ms: u64,
    pub expires_at_ms: u64,
    pub approvals: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct GrantContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    manifest_id: &'a str,
    signer_id: &'a str,
    signer_public_key: &'a str,
    roles: &'a BTreeSet<EnterpriseSignerRole>,
    scope: &'a EnterpriseGrantScope,
    not_before_ms: u64,
    expires_at_ms: u64,
}

impl EnterpriseSignerGrant {
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        manifest: &EnterpriseTrustManifest,
        signer_id: String,
        signer_public_key: String,
        roles: BTreeSet<EnterpriseSignerRole>,
        scope: EnterpriseGrantScope,
        not_before_ms: u64,
        expires_at_ms: u64,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            grant_id: String::new(),
            enterprise_id: manifest.enterprise_id.clone(),
            manifest_id: manifest.manifest_id.clone(),
            signer_id,
            signer_public_key,
            roles,
            scope,
            not_before_ms,
            expires_at_ms,
            approvals: BTreeMap::new(),
        };
        value.grant_id = value.content_id()?;
        for key in approvers {
            let signer_id = key.signer_id();
            value.approvals.insert(
                signer_id.clone(),
                key.sign(&AuthTranscript {
                    kind: "signer_grant",
                    enterprise_id: value.enterprise_id.as_str(),
                    payload_id: &value.grant_id,
                    manifest_id: &value.manifest_id,
                    grant_id: "",
                    signer_id: &signer_id,
                }),
            );
        }
        value.validate_shape()?;
        Ok(value)
    }

    pub(super) fn content_id(&self) -> Result<String, String> {
        Ok(format!(
            "grant:{}",
            canonical_digest(&GrantContent {
                schema_version: self.schema_version,
                enterprise_id: &self.enterprise_id,
                manifest_id: &self.manifest_id,
                signer_id: &self.signer_id,
                signer_public_key: &self.signer_public_key,
                roles: &self.roles,
                scope: &self.scope,
                not_before_ms: self.not_before_ms,
                expires_at_ms: self.expires_at_ms,
            })?
        ))
    }

    pub(super) fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_TRUST_SCHEMA_VERSION {
            return Err("unsupported enterprise signer grant schema".into());
        }
        if self.content_id()? != self.grant_id {
            return Err("signer grant content digest mismatch".into());
        }
        validate_prefixed_digest("signer grant", &self.grant_id, "grant:")?;
        validate_prefixed_digest("signer", &self.signer_id, "signer:")?;
        validate_public_key(&self.signer_public_key)?;
        if super::crypto::signer_id(&hex_public_key(&self.signer_public_key)?) != self.signer_id {
            return Err("signer grant id does not match its public key".into());
        }
        if self.roles.is_empty() {
            return Err("signer grant requires at least one role".into());
        }
        self.scope.validate()?;
        if self.not_before_ms == 0 || self.expires_at_ms <= self.not_before_ms {
            return Err("signer grant validity interval is invalid".into());
        }
        if self.approvals.len() > MAX_APPROVALS {
            return Err("signer grant has too many approvals".into());
        }
        for signature in self.approvals.values() {
            validate_signature(signature)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseSignedBatch {
    pub schema_version: u16,
    pub batch: EnterpriseBatch,
    pub manifest_id: String,
    pub grant: EnterpriseSignerGrant,
    pub signer_id: String,
    pub signed_at_ms: u64,
    pub signature: String,
}

impl EnterpriseSignedBatch {
    pub fn sign(
        batch: EnterpriseBatch,
        manifest: &EnterpriseTrustManifest,
        grant: EnterpriseSignerGrant,
        signed_at_ms: u64,
        signer: &EnterpriseSigningKey,
    ) -> Result<Self, String> {
        let signer_id = signer.signer_id();
        let payload_id = authenticated_batch_payload_id(batch.batch_id.as_str(), signed_at_ms);
        let signature = signer.sign(&AuthTranscript {
            kind: "enterprise_batch_v2",
            enterprise_id: batch.enterprise_id.as_str(),
            payload_id: &payload_id,
            manifest_id: &manifest.manifest_id,
            grant_id: &grant.grant_id,
            signer_id: &signer_id,
        });
        Ok(Self {
            schema_version: ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION,
            batch,
            manifest_id: manifest.manifest_id.clone(),
            grant,
            signer_id,
            signed_at_ms,
            signature,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseTrustChain {
    pub anchor_manifest_id: String,
    pub manifests: Vec<EnterpriseTrustManifest>,
}

pub(super) fn validate_prefixed_digest(
    label: &str,
    value: &str,
    prefix: &str,
) -> Result<(), String> {
    let Some(digest) = value.strip_prefix(prefix) else {
        return Err(format!("{label} id must use the {prefix} namespace"));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} id has an invalid digest"));
    }
    Ok(())
}

fn hex_public_key(value: &str) -> Result<[u8; 32], String> {
    let mut output = [0_u8; 32];
    if value.len() != 64 {
        return Err("signer public key has an invalid length".into());
    }
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
        output[index] =
            u8::from_str_radix(text, 16).map_err(|_| "invalid signer public key".to_string())?;
    }
    Ok(output)
}
