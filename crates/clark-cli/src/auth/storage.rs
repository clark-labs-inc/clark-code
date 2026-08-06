use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use directories::BaseDirs;
use zeroize::Zeroizing;

use super::Credential;

const MAGIC: &[u8; 8] = b"CLKCLI01";
const AAD: &[u8] = b"clark-cli-credential-v1";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const MAX_CREDENTIAL_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialSource {
    Environment,
    EncryptedFile,
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Environment => "CLARK_API_KEY environment variable",
            Self::EncryptedFile => "app-owned encrypted credential file",
        })
    }
}

pub struct CredentialStore {
    root: PathBuf,
    active_source: std::cell::Cell<CredentialSource>,
}

impl CredentialStore {
    pub fn new() -> Result<Self, String> {
        let root = match std::env::var_os("CLARK_HOME") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => BaseDirs::new()
                .map(|dirs| dirs.home_dir().join(".clark"))
                .ok_or_else(|| "could not locate the current user's home directory".to_string())?,
        };
        Ok(Self::at(root))
    }

    fn at(root: PathBuf) -> Self {
        Self {
            root,
            active_source: std::cell::Cell::new(CredentialSource::EncryptedFile),
        }
    }

    pub fn active_source(&self) -> CredentialSource {
        self.active_source.get()
    }

    pub fn load(&self) -> Result<Option<Credential>, String> {
        self.remove_obsolete_plaintext()?;
        let envelope = match read_private(&self.envelope_path())? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        if envelope.len() < MAGIC.len() + NONCE_BYTES + 16 || !envelope.starts_with(MAGIC) {
            return Err("Clark's encrypted CLI credential file is invalid".into());
        }
        let key = Zeroizing::new(self.load_key()?);
        let nonce = Nonce::try_from(&envelope[MAGIC.len()..MAGIC.len() + NONCE_BYTES])
            .map_err(|_| "Clark's encrypted CLI credential nonce is invalid".to_string())?;
        let plaintext = Zeroizing::new(
            ChaCha20Poly1305::new((&*key).into())
                .decrypt(
                    &nonce,
                    Payload {
                        msg: &envelope[MAGIC.len() + NONCE_BYTES..],
                        aad: AAD,
                    },
                )
                .map_err(|_| {
                    "Clark's encrypted CLI credential failed authentication".to_string()
                })?,
        );
        let credential = serde_json::from_slice(&plaintext)
            .map_err(|_| "Clark's encrypted CLI credential payload is invalid".to_string())?;
        self.active_source.set(CredentialSource::EncryptedFile);
        Ok(Some(credential))
    }

    pub fn save(&self, credential: &Credential) -> Result<CredentialSource, String> {
        ensure_private_root(&self.root)?;
        self.remove_obsolete_plaintext()?;
        let key = Zeroizing::new(self.load_or_create_key()?);
        let plaintext = Zeroizing::new(
            serde_json::to_vec(credential)
                .map_err(|_| "could not encode Clark CLI credential".to_string())?,
        );
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|_| "could not generate Clark CLI credential nonce".to_string())?;
        let nonce = Nonce::try_from(nonce_bytes.as_slice())
            .map_err(|_| "could not initialize Clark CLI credential nonce".to_string())?;
        let ciphertext = ChaCha20Poly1305::new((&*key).into())
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| "could not encrypt Clark CLI credential".to_string())?;
        let mut envelope = Vec::with_capacity(MAGIC.len() + NONCE_BYTES + ciphertext.len());
        envelope.extend_from_slice(MAGIC);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);
        write_atomic_private(&self.root, &self.envelope_path(), &envelope)?;
        self.active_source.set(CredentialSource::EncryptedFile);
        Ok(CredentialSource::EncryptedFile)
    }

    pub fn delete(&self) -> Result<bool, String> {
        let mut deleted = false;
        for path in [
            self.envelope_path(),
            self.key_path(),
            self.obsolete_plaintext_path(),
        ] {
            match fs::remove_file(&path) {
                Ok(()) => deleted = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "could not delete Clark credential {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(deleted)
    }

    fn envelope_path(&self) -> PathBuf {
        self.root.join("auth.enc")
    }

    fn key_path(&self) -> PathBuf {
        self.root.join("auth.key")
    }

    fn obsolete_plaintext_path(&self) -> PathBuf {
        self.root.join("auth.json")
    }

    fn remove_obsolete_plaintext(&self) -> Result<(), String> {
        match fs::remove_file(self.obsolete_plaintext_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not remove obsolete plaintext Clark credential: {error}"
            )),
        }
    }

    fn load_key(&self) -> Result<[u8; KEY_BYTES], String> {
        read_private(&self.key_path())?
            .ok_or_else(|| "Clark's local CLI credential key is missing".to_string())?
            .try_into()
            .map_err(|_| "Clark's local CLI credential key is invalid".to_string())
    }

    fn load_or_create_key(&self) -> Result<[u8; KEY_BYTES], String> {
        if let Some(bytes) = read_private(&self.key_path())? {
            return bytes
                .try_into()
                .map_err(|_| "Clark's local CLI credential key is invalid".to_string());
        }
        let mut generated = [0_u8; KEY_BYTES];
        getrandom::fill(&mut generated)
            .map_err(|_| "could not generate Clark CLI credential key".to_string())?;
        match exec_private_fs::write_private_new(self.key_path(), &generated) {
            Ok(true) => Ok(generated),
            Ok(false) => self.load_key(),
            Err(error) => Err(format!("could not write Clark CLI credential key: {error}")),
        }
    }
}

fn ensure_private_root(root: &Path) -> Result<(), String> {
    exec_private_fs::ensure_private_dir(root)
        .map_err(|error| format!("could not protect Clark home {}: {error}", root.display()))
}

fn read_private(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not inspect Clark credential {}: {error}",
                path.display()
            ));
        }
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_CREDENTIAL_BYTES
    {
        return Err(format!("Clark credential {} is unsafe", path.display()));
    }
    let mut file = exec_private_fs::PrivateFileOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| format!("could not read Clark credential: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("could not read Clark credential: {error}"))?;
    Ok(Some(bytes))
}

fn write_atomic_private(root: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = root.join(format!(".auth-{}.tmp", uuid::Uuid::new_v4().simple()));
    exec_private_fs::write_private_new(&temporary, bytes)
        .map_err(|error| format!("could not write encrypted Clark credential: {error}"))?;
    let result = replace_file(&temporary, destination);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination)
        .map_err(|error| format!("could not install encrypted Clark credential: {error}"))?;
    fs::File::open(
        destination
            .parent()
            .ok_or("Clark credential file has no parent directory")?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(|error| format!("could not sync Clark credential directory: {error}"))
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    (result != 0)
        .then_some(())
        .ok_or_else(|| "could not install encrypted Clark credential".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> Credential {
        Credential {
            api_key: "ck_live_test_secret_that_must_not_be_plaintext".into(),
            account_email: Some("qa@example.invalid".into()),
            api_key_id: Some("key_qa".into()),
            created_by: "api_key".into(),
        }
    }

    #[test]
    fn encrypted_file_round_trips_without_plaintext() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::at(temp.path().join("clark"));
        let expected = credential();

        assert_eq!(
            store.save(&expected).unwrap(),
            CredentialSource::EncryptedFile
        );
        let envelope = fs::read(store.envelope_path()).unwrap();
        assert!(envelope.starts_with(MAGIC));
        assert!(!envelope
            .windows(expected.api_key.len())
            .any(|window| window == expected.api_key.as_bytes()));
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.api_key, expected.api_key);
        assert_eq!(loaded.account_email, expected.account_email);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.envelope_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.key_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn tampering_fails_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::at(temp.path().join("clark"));
        store.save(&credential()).unwrap();
        let mut envelope = fs::read(store.envelope_path()).unwrap();
        *envelope.last_mut().unwrap() ^= 1;
        exec_private_fs::write_private(store.envelope_path(), &envelope).unwrap();
        assert!(store.load().unwrap_err().contains("failed authentication"));
    }

    #[test]
    fn obsolete_plaintext_is_deleted_not_migrated() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("clark");
        ensure_private_root(&root).unwrap();
        let store = CredentialStore::at(root);
        exec_private_fs::write_private(store.obsolete_plaintext_path(), b"plaintext").unwrap();

        assert!(store.load().unwrap().is_none());
        assert!(!store.obsolete_plaintext_path().exists());
    }

    #[test]
    fn delete_removes_envelope_key_and_obsolete_plaintext() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::at(temp.path().join("clark"));
        store.save(&credential()).unwrap();
        exec_private_fs::write_private(store.obsolete_plaintext_path(), b"obsolete").unwrap();

        assert!(store.delete().unwrap());
        assert!(!store.envelope_path().exists());
        assert!(!store.key_path().exists());
        assert!(!store.obsolete_plaintext_path().exists());
    }
}
