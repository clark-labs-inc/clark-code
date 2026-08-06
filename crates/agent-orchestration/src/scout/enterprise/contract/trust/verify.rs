use std::collections::{BTreeMap, BTreeSet};

use super::crypto::{authenticated_batch_payload_id, verify_signature, AuthTranscript};
use super::model::{
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseTrustChain,
    EnterpriseTrustManifest, ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION,
};
use crate::scout::enterprise::contract::{EnterpriseBatch, EnterpriseFact, EnterpriseId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEnterpriseBatch {
    envelope: EnterpriseSignedBatch,
}

impl VerifiedEnterpriseBatch {
    pub fn batch(&self) -> &EnterpriseBatch {
        &self.envelope.batch
    }

    pub fn envelope(&self) -> &EnterpriseSignedBatch {
        &self.envelope
    }

    pub fn into_envelope(self) -> EnterpriseSignedBatch {
        self.envelope
    }
}

impl EnterpriseTrustChain {
    pub fn verify(&self, enterprise_id: &EnterpriseId) -> Result<&EnterpriseTrustManifest, String> {
        if self.manifests.is_empty() {
            return Err("enterprise trust chain is empty".into());
        }
        let root = &self.manifests[0];
        if root.enterprise_id != *enterprise_id {
            return Err("trust chain belongs to another enterprise".into());
        }
        if root.manifest_id != self.anchor_manifest_id {
            return Err("trust root does not match the externally pinned anchor".into());
        }
        root.validate_shape()?;
        if root.generation != 1 || root.previous_manifest_id.is_some() {
            return Err("pinned trust root must be generation one".into());
        }
        verify_approvals(
            root,
            &root.coordinators,
            &root.revoked_signer_ids.keys().cloned().collect(),
            root.coordinator_threshold,
            &root.manifest_id,
        )?;

        let mut seen = BTreeSet::from([root.manifest_id.clone()]);
        let mut parent = root;
        for manifest in self.manifests.iter().skip(1) {
            manifest.validate_shape()?;
            if manifest.enterprise_id != *enterprise_id
                || manifest.trust_domain_id != root.trust_domain_id
            {
                return Err("trust manifest crosses an enterprise or trust domain".into());
            }
            if manifest.generation != parent.generation + 1
                || manifest.previous_manifest_id.as_deref() != Some(&parent.manifest_id)
            {
                return Err("trust manifest chain skips or names the wrong parent".into());
            }
            if !preserves_revocations(&parent.revoked_signer_ids, &manifest.revoked_signer_ids)
                || !preserves_revocations(&parent.revoked_grant_ids, &manifest.revoked_grant_ids)
            {
                return Err("trust revocations cannot be removed by a successor".into());
            }
            verify_approvals(
                manifest,
                &parent.coordinators,
                &parent.revoked_signer_ids.keys().cloned().collect(),
                parent.coordinator_threshold,
                &parent.manifest_id,
            )?;
            if !seen.insert(manifest.manifest_id.clone()) {
                return Err("trust chain repeats a manifest".into());
            }
            parent = manifest;
        }
        Ok(parent)
    }

    pub fn detect_forks(
        manifests: impl IntoIterator<Item = EnterpriseTrustManifest>,
    ) -> Result<(), String> {
        let mut successors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for manifest in manifests {
            if let Some(parent) = manifest.previous_manifest_id {
                successors
                    .entry(parent)
                    .or_default()
                    .insert(manifest.manifest_id);
            }
        }
        if successors.values().any(|children| children.len() > 1) {
            return Err(
                "enterprise trust fork detected; out-of-band resolution is required".into(),
            );
        }
        Ok(())
    }

    pub fn verify_signed_batch(
        &self,
        envelope: EnterpriseSignedBatch,
    ) -> Result<VerifiedEnterpriseBatch, String> {
        let current = self.verify(&envelope.batch.enterprise_id)?;
        let signing_manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.manifest_id == envelope.manifest_id)
            .ok_or_else(|| {
                "signed batch references a manifest outside the pinned chain".to_string()
            })?;
        verify_envelope(signing_manifest, current, &envelope)?;
        Ok(VerifiedEnterpriseBatch { envelope })
    }

    pub fn verify_grant_at(
        &self,
        grant: &EnterpriseSignerGrant,
        now_ms: u64,
    ) -> Result<(), String> {
        let current = self.verify(&grant.enterprise_id)?;
        let signing_manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.manifest_id == grant.manifest_id)
            .ok_or_else(|| {
                "signer grant references a manifest outside the pinned chain".to_string()
            })?;
        verify_grant(signing_manifest, grant)?;
        if now_ms < grant.not_before_ms
            || now_ms > grant.expires_at_ms
            || now_ms < signing_manifest.issued_at_ms
            || now_ms > signing_manifest.expires_at_ms
        {
            return Err("signer grant is outside its authenticated validity interval".into());
        }
        let signer_revoked = current
            .revoked_signer_ids
            .get(&grant.signer_id)
            .is_some_and(|effective| now_ms >= *effective);
        let grant_revoked = current
            .revoked_grant_ids
            .get(&grant.grant_id)
            .is_some_and(|effective| now_ms >= *effective);
        if signer_revoked || grant_revoked {
            return Err("signer grant is revoked at the requested time".into());
        }
        Ok(())
    }
}

fn verify_envelope(
    signing_manifest: &EnterpriseTrustManifest,
    current_manifest: &EnterpriseTrustManifest,
    envelope: &EnterpriseSignedBatch,
) -> Result<(), String> {
    if envelope.schema_version != ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION {
        return Err("unsupported signed enterprise batch schema".into());
    }
    envelope.batch.validate()?;
    if envelope.manifest_id != signing_manifest.manifest_id
        || envelope.grant.manifest_id != signing_manifest.manifest_id
    {
        return Err("signed batch does not reference the current trust manifest".into());
    }
    verify_grant(signing_manifest, &envelope.grant)?;
    if envelope.signer_id != envelope.grant.signer_id {
        return Err("signed batch signer does not match its grant".into());
    }
    let signer_revoked = current_manifest
        .revoked_signer_ids
        .get(&envelope.signer_id)
        .is_some_and(|effective| envelope.signed_at_ms >= *effective);
    let grant_revoked = current_manifest
        .revoked_grant_ids
        .get(&envelope.grant.grant_id)
        .is_some_and(|effective| envelope.signed_at_ms >= *effective);
    if signer_revoked || grant_revoked {
        return Err("signed batch uses a revoked signer or grant".into());
    }
    if envelope.signed_at_ms < envelope.grant.not_before_ms
        || envelope.signed_at_ms > envelope.grant.expires_at_ms
        || envelope.signed_at_ms < signing_manifest.issued_at_ms
        || envelope.signed_at_ms > signing_manifest.expires_at_ms
    {
        return Err("signed batch falls outside its authenticated validity interval".into());
    }
    for event in &envelope.batch.events {
        if !envelope.grant.scope.authorizes(&event.provenance) {
            return Err("signed batch contains an event outside its signer grant".into());
        }
        let role = match event.fact {
            EnterpriseFact::DiscoveryCharterObserved(_)
            | EnterpriseFact::DiscoveryPassSealed(_)
            | EnterpriseFact::ObservationRetracted { .. } => EnterpriseSignerRole::Coordinator,
            _ => EnterpriseSignerRole::Collector,
        };
        if !envelope.grant.roles.contains(&role)
            && !(role == EnterpriseSignerRole::Collector
                && envelope
                    .grant
                    .roles
                    .contains(&EnterpriseSignerRole::Coordinator))
        {
            return Err(format!("signed batch requires the {role:?} role"));
        }
    }
    let payload_id =
        authenticated_batch_payload_id(envelope.batch.batch_id.as_str(), envelope.signed_at_ms);
    verify_signature(
        &envelope.grant.signer_public_key,
        &envelope.signature,
        &AuthTranscript {
            kind: "enterprise_batch_v2",
            enterprise_id: envelope.batch.enterprise_id.as_str(),
            payload_id: &payload_id,
            manifest_id: &envelope.manifest_id,
            grant_id: &envelope.grant.grant_id,
            signer_id: &envelope.signer_id,
        },
    )
}

fn verify_grant(
    manifest: &EnterpriseTrustManifest,
    grant: &EnterpriseSignerGrant,
) -> Result<(), String> {
    grant.validate_shape()?;
    if grant.enterprise_id != manifest.enterprise_id || grant.manifest_id != manifest.manifest_id {
        return Err("signer grant belongs to another enterprise or manifest".into());
    }
    verify_approvals(
        grant,
        &manifest.coordinators,
        &manifest.revoked_signer_ids.keys().cloned().collect(),
        manifest.coordinator_threshold,
        &manifest.manifest_id,
    )
}

fn preserves_revocations(
    parent: &BTreeMap<String, u64>,
    successor: &BTreeMap<String, u64>,
) -> bool {
    parent
        .iter()
        .all(|(id, effective_at)| successor.get(id) == Some(effective_at))
}

trait ApprovedObject {
    fn enterprise_id(&self) -> &EnterpriseId;
    fn payload_id(&self) -> &str;
    fn approvals(&self) -> &BTreeMap<String, String>;
    fn kind(&self) -> &'static str;
    fn grant_id(&self) -> &str;
}

impl ApprovedObject for EnterpriseTrustManifest {
    fn enterprise_id(&self) -> &EnterpriseId {
        &self.enterprise_id
    }
    fn payload_id(&self) -> &str {
        &self.manifest_id
    }
    fn approvals(&self) -> &BTreeMap<String, String> {
        &self.approvals
    }
    fn kind(&self) -> &'static str {
        "trust_manifest"
    }
    fn grant_id(&self) -> &str {
        ""
    }
}

impl ApprovedObject for EnterpriseSignerGrant {
    fn enterprise_id(&self) -> &EnterpriseId {
        &self.enterprise_id
    }
    fn payload_id(&self) -> &str {
        &self.grant_id
    }
    fn approvals(&self) -> &BTreeMap<String, String> {
        &self.approvals
    }
    fn kind(&self) -> &'static str {
        "signer_grant"
    }
    fn grant_id(&self) -> &str {
        ""
    }
}

fn verify_approvals(
    object: &impl ApprovedObject,
    coordinators: &BTreeMap<String, String>,
    revoked: &BTreeSet<String>,
    threshold: u16,
    manifest_id: &str,
) -> Result<(), String> {
    let mut valid = 0_usize;
    for (signer, signature) in object.approvals() {
        let Some(public_key) = coordinators.get(signer) else {
            continue;
        };
        if revoked.contains(signer) {
            continue;
        }
        verify_signature(
            public_key,
            signature,
            &AuthTranscript {
                kind: object.kind(),
                enterprise_id: object.enterprise_id().as_str(),
                payload_id: object.payload_id(),
                manifest_id,
                grant_id: object.grant_id(),
                signer_id: signer,
            },
        )?;
        valid += 1;
    }
    if valid < usize::from(threshold) {
        return Err(format!(
            "authenticated coordinator threshold not met: {valid}/{threshold}"
        ));
    }
    Ok(())
}
