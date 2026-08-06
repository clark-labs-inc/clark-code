use std::io::Read;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer as _, SigningKey};
use exec_private_fs::{ensure_private_dir, write_private_new, PrivateFileOptions};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize;

const KEY_MAGIC: &[u8] = b"clark-security-scanner-key-v1\0";
const KEY_BYTES: usize = 32;
const MAX_BINDING_BYTES: usize = 4_096;

/// Host-private identity for one exact Clark account, organization, and
/// scanner-kind binding. The seed never crosses this module boundary.
pub(super) struct ClarkSecurityScannerIdentity {
    key: SigningKey,
    _key_path: PathBuf,
}

impl ClarkSecurityScannerIdentity {
    pub(super) fn load_or_create(
        private_root: impl AsRef<Path>,
        binding: &str,
    ) -> Result<Self, String> {
        let private_root = private_root.as_ref();
        validate_root(private_root)?;
        validate_binding(binding)?;
        ensure_private_dir(private_root)
            .map_err(|_| "failed to prepare the Clark Security identity directory".to_string())?;

        let key_path = private_root.join(format!("scanner-{}.key", hex_sha256(binding.as_bytes())));
        match load_seed(&key_path)? {
            Some(mut seed) => {
                let key = SigningKey::from_bytes(&seed);
                seed.zeroize();
                Ok(Self {
                    key,
                    _key_path: key_path,
                })
            }
            None => {
                let mut seed = [0_u8; KEY_BYTES];
                getrandom::fill(&mut seed)
                    .map_err(|_| "failed to generate a Clark Security identity".to_string())?;
                let mut encoded = Vec::with_capacity(KEY_MAGIC.len() + KEY_BYTES);
                encoded.extend_from_slice(KEY_MAGIC);
                encoded.extend_from_slice(&seed);
                let created = write_private_new(&key_path, &encoded)
                    .map_err(|_| "failed to persist the Clark Security identity".to_string())?;
                encoded.zeroize();
                if !created {
                    seed.zeroize();
                    let mut raced_seed = load_seed(&key_path)?.ok_or_else(|| {
                        "Clark Security identity disappeared during creation".to_string()
                    })?;
                    let key = SigningKey::from_bytes(&raced_seed);
                    raced_seed.zeroize();
                    return Ok(Self {
                        key,
                        _key_path: key_path,
                    });
                }
                let key = SigningKey::from_bytes(&seed);
                seed.zeroize();
                Ok(Self {
                    key,
                    _key_path: key_path,
                })
            }
        }
    }

    pub(super) fn public_key_hex(&self) -> String {
        hex::encode(self.key.verifying_key().as_bytes())
    }

    pub(super) fn signer_id(&self) -> String {
        format!(
            "security-signer:{}",
            hex_sha256(self.key.verifying_key().as_bytes())
        )
    }

    pub(super) fn sign_hex(&self, message: &[u8]) -> String {
        hex::encode(self.key.sign(message).to_bytes())
    }

    #[cfg(test)]
    fn key_path(&self) -> &Path {
        &self._key_path
    }
}

fn validate_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("Clark Security identity root must be absolute".into());
    }
    Ok(())
}

fn validate_binding(binding: &str) -> Result<(), String> {
    if binding.is_empty()
        || binding.len() > MAX_BINDING_BYTES
        || binding.chars().any(char::is_control)
    {
        return Err("Clark Security identity binding is invalid".into());
    }
    Ok(())
}

fn load_seed(path: &Path) -> Result<Option<[u8; KEY_BYTES]>, String> {
    let mut options = PrivateFileOptions::new();
    let mut file = match options.read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("failed to open the Clark Security identity".into()),
    };
    let expected_len = KEY_MAGIC.len() + KEY_BYTES;
    let mut encoded = Vec::with_capacity(expected_len + 1);
    file.by_ref()
        .take((expected_len + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| "failed to read the Clark Security identity".to_string())?;
    if encoded.len() != expected_len || !encoded.starts_with(KEY_MAGIC) {
        encoded.zeroize();
        return Err("Clark Security identity is corrupt or has an unsupported version".into());
    }
    let mut seed = [0_u8; KEY_BYTES];
    seed.copy_from_slice(&encoded[KEY_MAGIC.len()..]);
    encoded.zeroize();
    Ok(Some(seed))
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("clark-private")
    }

    #[test]
    fn identity_is_stable_and_partitioned_by_clark_binding() {
        let temp = tempfile::tempdir().unwrap();
        let first = ClarkSecurityScannerIdentity::load_or_create(
            root(&temp),
            "https://www.clarkchat.com|account-a|org-a|desktop",
        )
        .unwrap();
        let replay = ClarkSecurityScannerIdentity::load_or_create(
            root(&temp),
            "https://www.clarkchat.com|account-a|org-a|desktop",
        )
        .unwrap();
        let poc_lab = ClarkSecurityScannerIdentity::load_or_create(
            root(&temp),
            "https://www.clarkchat.com|account-a|org-a|poc_lab",
        )
        .unwrap();

        assert_eq!(first.public_key_hex(), replay.public_key_hex());
        assert_eq!(first.signer_id(), replay.signer_id());
        assert_ne!(first.public_key_hex(), poc_lab.public_key_hex());
        assert_ne!(first.key_path(), poc_lab.key_path());
        assert!(!first.key_path().to_string_lossy().contains("account-a"));
        assert!(!first.key_path().to_string_lossy().contains("org-a"));
    }

    #[test]
    fn corrupt_or_relative_identity_fails_closed() {
        assert!(ClarkSecurityScannerIdentity::load_or_create("relative", "binding").is_err());
        let temp = tempfile::tempdir().unwrap();
        let identity =
            ClarkSecurityScannerIdentity::load_or_create(root(&temp), "binding").unwrap();
        std::fs::write(identity.key_path(), b"corrupt").unwrap();
        assert!(ClarkSecurityScannerIdentity::load_or_create(root(&temp), "binding").is_err());
    }

    #[test]
    fn signatures_are_ed25519_and_signer_id_is_clark_security_namespaced() {
        let temp = tempfile::tempdir().unwrap();
        let identity =
            ClarkSecurityScannerIdentity::load_or_create(root(&temp), "binding").unwrap();
        let signature = identity.sign_hex(b"clark-security-test");
        assert_eq!(signature.len(), 128);
        assert!(identity.signer_id().starts_with("security-signer:"));
    }
}
