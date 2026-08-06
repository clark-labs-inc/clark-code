use std::io::Read;
use std::path::{Path, PathBuf};

use exec_private_fs::{ensure_private_dir, write_private_new, PrivateFileOptions};
use scout_ingest_protocol::cartography::CollectorSigningKey;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize;

const KEY_MAGIC: &[u8] = b"clark-scout-key-v1\0";
const KEY_BYTES: usize = 32;
const MAX_BINDING_BYTES: usize = 4_096;

/// Host-private signing identity for one Clark organization/workspace binding.
///
/// The seed is stored below an explicit host-private directory and is never
/// exposed by this API. The type intentionally does not implement `Debug`,
/// `Clone`, or serialization.
pub struct CollectorMachineIdentity {
    key: CollectorSigningKey,
    key_path: PathBuf,
}

impl CollectorMachineIdentity {
    /// Load or atomically create the identity for an exact backend binding.
    ///
    /// `private_root` must be an absolute host-owned path outside any project
    /// workspace. `binding` should include the Clark origin, organization id,
    /// and workspace id; only its SHA-256 appears in the filename.
    pub fn load_or_create(private_root: impl AsRef<Path>, binding: &str) -> Result<Self, String> {
        let private_root = private_root.as_ref();
        validate_root(private_root)?;
        validate_binding(binding)?;
        ensure_private_dir(private_root)
            .map_err(|_| "failed to prepare the Scout machine identity directory".to_string())?;

        let key_path = private_root.join(format!("collector-{}.key", hex_sha256(binding)));
        match load_seed(&key_path)? {
            Some(mut seed) => {
                let key = CollectorSigningKey::from_seed(seed);
                seed.zeroize();
                Ok(Self { key, key_path })
            }
            None => {
                let mut seed = [0_u8; KEY_BYTES];
                getrandom::fill(&mut seed)
                    .map_err(|_| "failed to generate a Scout machine identity".to_string())?;
                let mut encoded = Vec::with_capacity(KEY_MAGIC.len() + KEY_BYTES);
                encoded.extend_from_slice(KEY_MAGIC);
                encoded.extend_from_slice(&seed);
                let created = write_private_new(&key_path, &encoded)
                    .map_err(|_| "failed to persist the Scout machine identity".to_string())?;
                encoded.zeroize();
                if !created {
                    seed.zeroize();
                    let mut raced_seed = load_seed(&key_path)?.ok_or_else(|| {
                        "Scout machine identity disappeared during creation".to_string()
                    })?;
                    let key = CollectorSigningKey::from_seed(raced_seed);
                    raced_seed.zeroize();
                    return Ok(Self { key, key_path });
                }
                let key = CollectorSigningKey::from_seed(seed);
                seed.zeroize();
                Ok(Self { key, key_path })
            }
        }
    }

    pub fn signing_key(&self) -> &CollectorSigningKey {
        &self.key
    }

    pub fn public_key_hex(&self) -> String {
        self.key.public_key_hex()
    }

    pub fn signer_id(&self) -> String {
        self.key.signer_id()
    }

    /// Path is exposed for host diagnostics and deletion/revocation workflows;
    /// its filename contains only a binding digest.
    pub fn key_path(&self) -> &Path {
        &self.key_path
    }
}

fn validate_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("Scout machine identity root must be absolute".into());
    }
    Ok(())
}

fn validate_binding(binding: &str) -> Result<(), String> {
    if binding.is_empty()
        || binding.len() > MAX_BINDING_BYTES
        || binding.chars().any(char::is_control)
    {
        return Err("Scout machine identity binding is invalid".into());
    }
    Ok(())
}

fn load_seed(path: &Path) -> Result<Option<[u8; KEY_BYTES]>, String> {
    let mut options = PrivateFileOptions::new();
    let mut file = match options.read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("failed to open the Scout machine identity".into()),
    };
    let expected_len = KEY_MAGIC.len() + KEY_BYTES;
    let mut encoded = Vec::with_capacity(expected_len + 1);
    file.by_ref()
        .take((expected_len + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| "failed to read the Scout machine identity".to_string())?;
    if encoded.len() != expected_len || !encoded.starts_with(KEY_MAGIC) {
        encoded.zeroize();
        return Err("Scout machine identity is corrupt or has an unsupported version".into());
    }
    let mut seed = [0_u8; KEY_BYTES];
    seed.copy_from_slice(&encoded[KEY_MAGIC.len()..]);
    encoded.zeroize();
    Ok(Some(seed))
}

fn hex_sha256(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("host-private")
    }

    #[test]
    fn identity_is_stable_and_separated_by_backend_binding() {
        let temp = tempfile::tempdir().unwrap();
        let first = CollectorMachineIdentity::load_or_create(
            root(&temp),
            "https://api.clarkslabs.com|org-a|workspace-a",
        )
        .unwrap();
        let replay = CollectorMachineIdentity::load_or_create(
            root(&temp),
            "https://api.clarkslabs.com|org-a|workspace-a",
        )
        .unwrap();
        let other = CollectorMachineIdentity::load_or_create(
            root(&temp),
            "https://api.clarkslabs.com|org-b|workspace-b",
        )
        .unwrap();
        assert_eq!(first.public_key_hex(), replay.public_key_hex());
        assert_eq!(first.signer_id(), replay.signer_id());
        assert_ne!(first.public_key_hex(), other.public_key_hex());
        assert_ne!(first.key_path(), other.key_path());
        assert!(!first.key_path().to_string_lossy().contains("org-a"));
        assert!(!first.key_path().to_string_lossy().contains("workspace-a"));
    }

    #[test]
    fn relative_root_and_corrupt_identity_fail_closed() {
        assert!(CollectorMachineIdentity::load_or_create("relative", "binding").is_err());

        let temp = tempfile::tempdir().unwrap();
        let identity = CollectorMachineIdentity::load_or_create(root(&temp), "binding").unwrap();
        std::fs::write(identity.key_path(), b"corrupt").unwrap();
        assert!(CollectorMachineIdentity::load_or_create(root(&temp), "binding").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn identity_file_is_owner_only_and_symlinks_are_refused() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let identity = CollectorMachineIdentity::load_or_create(root(&temp), "binding").unwrap();
        assert_eq!(
            std::fs::metadata(identity.key_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let target = temp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let linked = temp.path().join("linked");
        symlink(&target, &linked).unwrap();
        assert!(CollectorMachineIdentity::load_or_create(linked, "binding").is_err());
    }
}
