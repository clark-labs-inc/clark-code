use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"clark.scout.enterprise.auth/v1\0";

pub struct EnterpriseSigningKey(SigningKey);

pub(super) struct AuthTranscript<'a> {
    pub kind: &'a str,
    pub enterprise_id: &'a str,
    pub payload_id: &'a str,
    pub manifest_id: &'a str,
    pub grant_id: &'a str,
    pub signer_id: &'a str,
}

pub(super) fn authenticated_batch_payload_id(batch_id: &str, signed_at_ms: u64) -> String {
    format!("{batch_id}@signed-at:{signed_at_ms}")
}

impl EnterpriseSigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    pub fn public_key_hex(&self) -> String {
        hex_encode(self.0.verifying_key().as_bytes())
    }

    pub fn signer_id(&self) -> String {
        signer_id(self.0.verifying_key().as_bytes())
    }

    pub(super) fn sign(&self, transcript: &AuthTranscript<'_>) -> String {
        hex_encode(&self.0.sign(&encode_transcript(transcript)).to_bytes())
    }
}

impl fmt::Debug for EnterpriseSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnterpriseSigningKey")
            .field("signer_id", &self.signer_id())
            .finish_non_exhaustive()
    }
}

pub(super) fn verify_signature(
    public_key_hex: &str,
    signature_hex: &str,
    transcript: &AuthTranscript<'_>,
) -> Result<(), String> {
    let public = hex_decode_array::<32>("signer public key", public_key_hex)?;
    if signer_id(&public) != transcript.signer_id {
        return Err("signer id does not match its public key".into());
    }
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|error| format!("invalid signer public key: {error}"))?;
    let signature =
        Signature::from_bytes(&hex_decode_array::<64>("Ed25519 signature", signature_hex)?);
    key.verify_strict(&encode_transcript(transcript), &signature)
        .map_err(|_| "Ed25519 signature verification failed".to_string())
}

pub(super) fn signer_id(public_key: &[u8; 32]) -> String {
    format!("signer:{:x}", Sha256::digest(public_key))
}

pub(super) fn validate_public_key(value: &str) -> Result<(), String> {
    hex_decode_array::<32>("signer public key", value).map(|_| ())
}

pub(super) fn validate_signature(value: &str) -> Result<(), String> {
    hex_decode_array::<64>("Ed25519 signature", value).map(|_| ())
}

fn encode_transcript(transcript: &AuthTranscript<'_>) -> Vec<u8> {
    let mut output = DOMAIN.to_vec();
    for field in [
        transcript.kind,
        transcript.enterprise_id,
        transcript.payload_id,
        transcript.manifest_id,
        transcript.grant_id,
        transcript.signer_id,
    ] {
        output.extend_from_slice(&(field.len() as u64).to_le_bytes());
        output.extend_from_slice(field.as_bytes());
    }
    output
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to String cannot fail");
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
