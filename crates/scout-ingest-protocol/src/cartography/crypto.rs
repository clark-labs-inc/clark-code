use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use sha2::{Digest as _, Sha256};

const TASK_CLAIM_DOMAIN: &[u8] = b"clark.system-cartography.task-claim/v1\0";
const EVIDENCE_UPLOAD_DOMAIN: &[u8] = b"clark.system-cartography.evidence-upload/v1\0";
const EVIDENCE_COMMIT_DOMAIN: &[u8] = b"clark.system-cartography.evidence-commit/v1\0";
const BATCH_DOMAIN: &[u8] = b"clark.system-cartography.collector-batch/v1\0";
const RECEIPT_DOMAIN: &[u8] = b"clark.system-cartography.acceptance-receipt/v1\0";

pub struct CollectorSigningKey(SigningKey);

impl CollectorSigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    pub fn public_key_hex(&self) -> String {
        hex_encode(self.0.verifying_key().as_bytes())
    }

    pub fn signer_id(&self) -> String {
        signer_id(self.0.verifying_key().as_bytes())
    }

    pub(super) fn sign_task_claim(&self, payload: &[u8]) -> String {
        sign(&self.0, TASK_CLAIM_DOMAIN, payload)
    }

    pub(super) fn sign_evidence_upload(&self, payload: &[u8]) -> String {
        sign(&self.0, EVIDENCE_UPLOAD_DOMAIN, payload)
    }

    pub(super) fn sign_evidence_commit(&self, payload: &[u8]) -> String {
        sign(&self.0, EVIDENCE_COMMIT_DOMAIN, payload)
    }

    pub(super) fn sign_batch(&self, payload: &[u8]) -> String {
        sign(&self.0, BATCH_DOMAIN, payload)
    }
}

impl fmt::Debug for CollectorSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CollectorSigningKey")
            .field("signer_id", &self.signer_id())
            .finish_non_exhaustive()
    }
}

pub(super) fn verify_receipt(
    public_key_hex: &str,
    expected_signer_id: &str,
    signature_hex: &str,
    payload: &[u8],
) -> Result<(), String> {
    let public_key = hex_decode_array::<32>("coordinator public key", public_key_hex)?;
    if signer_id(&public_key) != expected_signer_id {
        return Err("receipt coordinator id does not match its public key".into());
    }
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| format!("invalid coordinator public key: {error}"))?;
    let signature =
        Signature::from_bytes(&hex_decode_array::<64>("receipt signature", signature_hex)?);
    verifying_key
        .verify_strict(&transcript(RECEIPT_DOMAIN, payload), &signature)
        .map_err(|_| "system-cartography receipt signature verification failed".into())
}

fn sign(key: &SigningKey, domain: &[u8], payload: &[u8]) -> String {
    hex_encode(&key.sign(&transcript(domain, payload)).to_bytes())
}

fn transcript(domain: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(domain.len() + 8 + payload.len());
    transcript.extend_from_slice(domain);
    transcript.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    transcript.extend_from_slice(payload);
    transcript
}

fn signer_id(public_key: &[u8; 32]) -> String {
    format!("signer:{}", sha256_hex(public_key))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

fn hex_decode_array<const N: usize>(label: &str, value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{label} must be a {}-character hexadecimal value",
            N * 2
        ));
    }
    let mut output = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(output)
}

fn nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hexadecimal value".into()),
    }
}
