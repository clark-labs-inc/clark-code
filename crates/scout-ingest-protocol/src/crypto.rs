use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

pub(crate) const RECEIPT_DOMAIN: &[u8] = b"clark.scout.central-ingestion.receipt/v1\0";

pub struct CoordinatorSigningKey(SigningKey);

impl CoordinatorSigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    pub fn public_key_hex(&self) -> String {
        hex_encode(self.0.verifying_key().as_bytes())
    }

    pub fn coordinator_id(&self) -> String {
        coordinator_id(self.0.verifying_key().as_bytes())
    }

    pub(crate) fn sign(&self, transcript: &[u8]) -> String {
        hex_encode(&self.0.sign(transcript).to_bytes())
    }
}

impl fmt::Debug for CoordinatorSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoordinatorSigningKey")
            .field("coordinator_id", &self.coordinator_id())
            .finish_non_exhaustive()
    }
}

pub(crate) fn verify(
    public_key_hex: &str,
    coordinator: &str,
    signature_hex: &str,
    transcript: &[u8],
) -> Result<(), String> {
    let public = hex_decode_array::<32>("coordinator public key", public_key_hex)?;
    if coordinator_id(&public) != coordinator {
        return Err("coordinator id does not match its public key".into());
    }
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|error| format!("invalid coordinator public key: {error}"))?;
    let signature = Signature::from_bytes(&hex_decode_array::<64>(
        "coordinator signature",
        signature_hex,
    )?);
    key.verify_strict(transcript, &signature)
        .map_err(|_| "coordinator receipt signature verification failed".to_string())
}

pub(crate) fn coordinator_id(public_key: &[u8; 32]) -> String {
    format!("coordinator:{}", hex_encode(&Sha256::digest(public_key)))
}

pub(crate) fn digest_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

pub(crate) fn validate_digest_reference(
    label: &str,
    value: &str,
    prefix: &str,
) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has an invalid prefix"))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} has an invalid digest"));
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

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
