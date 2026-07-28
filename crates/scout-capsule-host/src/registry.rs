use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CapsuleHostLimits, CAPSULE_HOST_ABI_VERSION};

pub const REGISTRY_SCHEMA: &str = "scout-capsule-registry-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleRegistryEntry {
    pub module_sha256: String,
    pub abi_version: u16,
    pub tenant_ids: BTreeSet<String>,
    pub enterprise_ids: BTreeSet<String>,
    pub input_schema: String,
    pub output_schema: String,
    pub limits: CapsuleHostLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleRegistryPayload {
    pub schema: String,
    pub generation: u64,
    pub not_before_ms: u64,
    pub expires_at_ms: u64,
    pub target_id: String,
    pub target_identity_sha256: String,
    pub entries: BTreeMap<String, CapsuleRegistryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCapsuleRegistry {
    pub payload: CapsuleRegistryPayload,
    pub admin_public_key_hex: String,
    pub signature_hex: String,
}

impl SignedCapsuleRegistry {
    pub fn sign(payload: CapsuleRegistryPayload, key: &SigningKey) -> Result<Self, String> {
        validate_payload(&payload)?;
        let encoded = serde_json::to_vec(&payload).map_err(|_| "registry encoding failed")?;
        Ok(Self {
            payload,
            admin_public_key_hex: encode_hex(key.verifying_key().as_bytes()),
            signature_hex: encode_hex(&key.sign(&encoded).to_bytes()),
        })
    }

    pub(crate) fn verify(
        &self,
        trusted_key_sha256: &str,
        minimum_generation: u64,
    ) -> Result<String, String> {
        validate_payload(&self.payload)?;
        if self.payload.generation < minimum_generation {
            return Err("registry generation is below the administrator minimum".into());
        }
        let public_bytes = decode_array::<32>(&self.admin_public_key_hex)
            .ok_or_else(|| "registry administrator key is invalid".to_string())?;
        if digest(&public_bytes) != trusted_key_sha256 {
            return Err("registry administrator key does not match the host pin".into());
        }
        let key = VerifyingKey::from_bytes(&public_bytes)
            .map_err(|_| "registry administrator key is invalid".to_string())?;
        let signature_bytes = decode_array::<64>(&self.signature_hex)
            .ok_or_else(|| "registry signature is invalid".to_string())?;
        let encoded = serde_json::to_vec(&self.payload)
            .map_err(|_| "registry encoding failed".to_string())?;
        key.verify(&encoded, &Signature::from_bytes(&signature_bytes))
            .map_err(|_| "registry signature verification failed".to_string())?;
        serde_json::to_vec(self)
            .map(|bytes| digest(&bytes))
            .map_err(|_| "registry encoding failed".to_string())
    }
}

fn validate_payload(payload: &CapsuleRegistryPayload) -> Result<(), String> {
    if payload.schema != REGISTRY_SCHEMA
        || payload.generation == 0
        || payload.not_before_ms == 0
        || payload.expires_at_ms <= payload.not_before_ms
        || payload.entries.is_empty()
    {
        return Err("registry metadata is invalid".into());
    }
    validate_safe("target_id", &payload.target_id, 256)?;
    validate_digest(&payload.target_identity_sha256)?;
    for (capsule_id, entry) in &payload.entries {
        validate_safe("capsule_id", capsule_id, 128)?;
        validate_digest(&entry.module_sha256)?;
        if entry.abi_version != CAPSULE_HOST_ABI_VERSION
            || entry.tenant_ids.is_empty()
            || entry.enterprise_ids.is_empty()
        {
            return Err("registry entry binding is invalid".into());
        }
        validate_safe("input_schema", &entry.input_schema, 128)?;
        validate_safe("output_schema", &entry.output_schema, 128)?;
        entry.limits.validate().map_err(|error| error.to_string())?;
        for value in entry.tenant_ids.iter().chain(&entry.enterprise_ids) {
            validate_safe("registry authority", value, 256)?;
        }
    }
    Ok(())
}

fn validate_safe(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err("registry digest is invalid".into())
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_array<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut bytes = [0_u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
