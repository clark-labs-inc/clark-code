use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::crypto::{
    signer_id, validate_public_key, validate_signature, verify_signature, AuthTranscript,
    EnterpriseSigningKey,
};
use super::model::{
    validate_prefixed_digest, EnterpriseGrantScope, EnterpriseSignedBatch, EnterpriseSignerGrant,
    EnterpriseSignerRole, EnterpriseTrustChain, ENTERPRISE_TRUST_SCHEMA_VERSION,
};
use crate::scout::enterprise::contract::{canonical_digest, EnterpriseId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseSignerProposal {
    pub schema_version: u16,
    pub proposal_id: String,
    pub enterprise_id: EnterpriseId,
    pub anchor_manifest_id: String,
    pub signer_id: String,
    pub signer_public_key: String,
    pub roles: BTreeSet<EnterpriseSignerRole>,
    pub scope: EnterpriseGrantScope,
    pub not_before_ms: u64,
    pub expires_at_ms: u64,
    pub proof_of_possession: String,
}

#[derive(Serialize)]
struct ProposalContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    anchor_manifest_id: &'a str,
    signer_id: &'a str,
    signer_public_key: &'a str,
    roles: &'a BTreeSet<EnterpriseSignerRole>,
    scope: &'a EnterpriseGrantScope,
    not_before_ms: u64,
    expires_at_ms: u64,
}

impl EnterpriseSignerProposal {
    pub fn create(
        enterprise_id: EnterpriseId,
        anchor_manifest_id: String,
        roles: BTreeSet<EnterpriseSignerRole>,
        scope: EnterpriseGrantScope,
        not_before_ms: u64,
        expires_at_ms: u64,
        signer: &EnterpriseSigningKey,
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            proposal_id: String::new(),
            enterprise_id,
            anchor_manifest_id,
            signer_id: signer.signer_id(),
            signer_public_key: signer.public_key_hex(),
            roles,
            scope,
            not_before_ms,
            expires_at_ms,
            proof_of_possession: String::new(),
        };
        value.proposal_id = value.content_id()?;
        value.proof_of_possession = signer.sign(&AuthTranscript {
            kind: "signer_proposal",
            enterprise_id: value.enterprise_id.as_str(),
            payload_id: &value.proposal_id,
            manifest_id: &value.anchor_manifest_id,
            grant_id: "",
            signer_id: &value.signer_id,
        });
        value.verify()?;
        Ok(value)
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_TRUST_SCHEMA_VERSION {
            return Err("unsupported enterprise signer proposal schema".into());
        }
        if self.content_id()? != self.proposal_id {
            return Err("signer proposal content digest mismatch".into());
        }
        validate_prefixed_digest("signer proposal", &self.proposal_id, "proposal:")?;
        validate_prefixed_digest("trust anchor", &self.anchor_manifest_id, "trust-manifest:")?;
        validate_prefixed_digest("signer", &self.signer_id, "signer:")?;
        validate_public_key(&self.signer_public_key)?;
        validate_signature(&self.proof_of_possession)?;
        if self.roles.is_empty() {
            return Err("signer proposal requires at least one role".into());
        }
        self.scope.validate()?;
        if self.not_before_ms == 0 || self.expires_at_ms <= self.not_before_ms {
            return Err("signer proposal validity interval is invalid".into());
        }
        let public = decode_public_key(&self.signer_public_key)?;
        if signer_id(&public) != self.signer_id {
            return Err("signer proposal id does not match its public key".into());
        }
        verify_signature(
            &self.signer_public_key,
            &self.proof_of_possession,
            &AuthTranscript {
                kind: "signer_proposal",
                enterprise_id: self.enterprise_id.as_str(),
                payload_id: &self.proposal_id,
                manifest_id: &self.anchor_manifest_id,
                grant_id: "",
                signer_id: &self.signer_id,
            },
        )
    }

    fn content_id(&self) -> Result<String, String> {
        Ok(format!(
            "proposal:{}",
            canonical_digest(&ProposalContent {
                schema_version: self.schema_version,
                enterprise_id: &self.enterprise_id,
                anchor_manifest_id: &self.anchor_manifest_id,
                signer_id: &self.signer_id,
                signer_public_key: &self.signer_public_key,
                roles: &self.roles,
                scope: &self.scope,
                not_before_ms: self.not_before_ms,
                expires_at_ms: self.expires_at_ms,
            })?
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseGrantBundle {
    pub trust_chain: EnterpriseTrustChain,
    pub grant: EnterpriseSignerGrant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseBatchBundle {
    pub trust_chain: EnterpriseTrustChain,
    pub signed_batch: EnterpriseSignedBatch,
}

fn decode_public_key(value: &str) -> Result<[u8; 32], String> {
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
