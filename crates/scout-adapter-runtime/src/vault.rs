use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use fs2::FileExt;
use scout_adapter_protocol::{
    AdapterPageReceipt, AdapterPageRequest, AuthContextDescriptor, AuthContextHandle,
    CursorVaultBinding, TargetIdentity,
};
use serde::{Deserialize, Serialize};

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::AwsAuthMode;
use crate::types::AuthCandidateHandle;
use crate::vault_io::{
    digest, open_private, private_options, random_digest, reject_symlink, replace_file,
    sync_directory,
};

const VAULT_VERSION: u16 = 1;
const MAX_VAULT_BYTES: u64 = 8 * 1024 * 1024;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredAuthRef {
    GithubEnvironment { variable: String },
    GithubCli,
    GitlabEnvironment { variable: String },
    AwsEnvironment,
    AwsProfile { profile: String },
    AwsWorkload,
    GcpCli { account: String },
}

impl StoredAuthRef {
    pub(crate) fn stable_key(&self) -> String {
        match self {
            Self::GithubEnvironment { variable } => format!("github:env:{variable}"),
            Self::GithubCli => "github:cli".to_owned(),
            Self::GitlabEnvironment { variable } => format!("gitlab:env:{variable}"),
            Self::AwsEnvironment => "aws:env".to_owned(),
            Self::AwsProfile { profile } => format!("aws:profile:{profile}"),
            Self::AwsWorkload => "aws:workload".to_owned(),
            Self::GcpCli { account } => format!("gcp:cli:{account}"),
        }
    }

    pub(crate) fn aws_mode(&self) -> Option<AwsAuthMode> {
        match self {
            Self::AwsEnvironment => Some(AwsAuthMode::Environment),
            Self::AwsProfile { profile } => Some(AwsAuthMode::Profile(profile.clone())),
            Self::AwsWorkload => Some(AwsAuthMode::Workload),
            Self::GithubEnvironment { .. }
            | Self::GithubCli
            | Self::GitlabEnvironment { .. }
            | Self::GcpCli { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ProviderCursor {
    GithubPage(u32),
    GitlabPage(u32),
    AwsOrganizations(String),
    AwsResources(String),
    GcpAfterKey { operation: u8, key: String },
}

#[derive(Clone)]
pub(crate) struct PrivateVault {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultState {
    version: u16,
    target: TargetIdentity,
    candidates: BTreeMap<String, StoredAuthRef>,
    auth_contexts: BTreeMap<String, StoredAuth>,
    cursors: BTreeMap<String, StoredCursor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAuth {
    descriptor: AuthContextDescriptor,
    reference: StoredAuthRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCursor {
    binding: CursorVaultBinding,
    nonce_b64: String,
    ciphertext_b64: String,
}

impl PrivateVault {
    pub(crate) fn open(root: impl Into<PathBuf>) -> RuntimeResult<Self> {
        let vault = Self { root: root.into() };
        vault.ensure_root()?;
        let lock = vault.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(|_| RuntimeError::Vault)?;
        let _key = vault.load_or_create_key()?;
        if !vault.state_path().exists() {
            let target = vault.create_target_identity()?;
            vault.write_state(&VaultState {
                version: VAULT_VERSION,
                target,
                candidates: BTreeMap::new(),
                auth_contexts: BTreeMap::new(),
                cursors: BTreeMap::new(),
            })?;
        } else {
            vault.read_state()?;
        }
        Ok(vault)
    }

    pub(crate) fn target(&self) -> RuntimeResult<TargetIdentity> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(|_| RuntimeError::Vault)?;
        Ok(self.read_state()?.target)
    }

    pub(crate) fn replace_candidates(
        &self,
        candidates: &[(AuthCandidateHandle, StoredAuthRef)],
    ) -> RuntimeResult<()> {
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(|_| RuntimeError::Vault)?;
        let mut state = self.read_state()?;
        state.candidates = candidates
            .iter()
            .map(|(handle, reference)| (handle.as_str().to_owned(), reference.clone()))
            .collect();
        self.write_state(&state)
    }

    pub(crate) fn candidate(&self, handle: &AuthCandidateHandle) -> RuntimeResult<StoredAuthRef> {
        handle.validate()?;
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(|_| RuntimeError::Vault)?;
        self.read_state()?
            .candidates
            .get(handle.as_str())
            .cloned()
            .ok_or(RuntimeError::CandidateNotFound)
    }

    pub(crate) fn store_auth(
        &self,
        descriptor: &AuthContextDescriptor,
        reference: StoredAuthRef,
    ) -> RuntimeResult<()> {
        descriptor.validate_at(descriptor.verified_at_ms)?;
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(|_| RuntimeError::Vault)?;
        let mut state = self.read_state()?;
        state.auth_contexts.insert(
            descriptor.handle.as_str().to_owned(),
            StoredAuth {
                descriptor: descriptor.clone(),
                reference,
            },
        );
        self.write_state(&state)
    }

    pub(crate) fn auth(
        &self,
        handle: &AuthContextHandle,
    ) -> RuntimeResult<(AuthContextDescriptor, StoredAuthRef)> {
        handle.validate()?;
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(|_| RuntimeError::Vault)?;
        let stored = self
            .read_state()?
            .auth_contexts
            .get(handle.as_str())
            .cloned()
            .ok_or(RuntimeError::AuthNotFound)?;
        Ok((stored.descriptor, stored.reference))
    }

    pub(crate) fn store_cursor(
        &self,
        receipt: &AdapterPageReceipt,
        cursor: &ProviderCursor,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> RuntimeResult<()> {
        let binding = CursorVaultBinding::for_next_page(receipt, issued_at_ms, expires_at_ms)?;
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(|_| RuntimeError::Vault)?;
        let key = self.load_or_create_key()?;
        let mut state = self.read_state()?;
        let stored = seal_cursor(cursor, &binding, &key)?;
        state
            .cursors
            .insert(binding.cursor_handle.as_str().to_owned(), stored);
        self.write_state(&state)
    }

    pub(crate) fn cursor(
        &self,
        request: &AdapterPageRequest,
        target: &TargetIdentity,
        auth: &AuthContextDescriptor,
        now_ms: u64,
    ) -> RuntimeResult<ProviderCursor> {
        let handle = request
            .cursor_handle
            .as_ref()
            .ok_or(RuntimeError::InvalidRequest)?;
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(|_| RuntimeError::Vault)?;
        let state = self.read_state()?;
        let stored = state
            .cursors
            .get(handle.as_str())
            .ok_or(RuntimeError::AuthStale)?;
        stored
            .binding
            .authorize(request, target, auth, now_ms)
            .map_err(|_| RuntimeError::TargetMismatch)?;
        let key = self.load_or_create_key()?;
        open_cursor(stored, &key)
    }

    #[cfg(test)]
    pub(crate) fn state_path_for_test(&self) -> PathBuf {
        self.state_path()
    }

    fn create_target_identity(&self) -> RuntimeResult<TargetIdentity> {
        let canonical = fs::canonicalize(&self.root).map_err(|_| RuntimeError::Vault)?;
        TargetIdentity::new(
            random_digest()?,
            random_digest()?,
            digest(canonical.as_os_str().to_string_lossy().as_bytes()),
            digest(
                format!(
                    "scout-adapter-runtime@{}:{}:{}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
                .as_bytes(),
            ),
            std::env::consts::OS.to_owned(),
            std::env::consts::ARCH.to_owned(),
        )
        .map_err(Into::into)
    }

    fn ensure_root(&self) -> RuntimeResult<()> {
        exec_private_fs::ensure_private_dir(&self.root).map_err(|_| RuntimeError::Vault)
    }

    fn open_lock(&self) -> RuntimeResult<File> {
        open_private(&self.root.join("vault.lock"), true)
    }

    fn load_or_create_key(&self) -> RuntimeResult<[u8; KEY_BYTES]> {
        let path = self.root.join("vault.key");
        reject_symlink(&path)?;
        if !path.exists() {
            let mut key = [0_u8; KEY_BYTES];
            getrandom::fill(&mut key).map_err(|_| RuntimeError::Vault)?;
            let mut options = private_options();
            match options.create_new(true).write(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(&key).map_err(|_| RuntimeError::Vault)?;
                    file.sync_all().map_err(|_| RuntimeError::Vault)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(RuntimeError::Vault),
            }
        }
        let mut file = open_private(&path, false)?;
        let mut key = [0_u8; KEY_BYTES];
        file.read_exact(&mut key).map_err(|_| RuntimeError::Vault)?;
        let mut extra = [0_u8; 1];
        if file.read(&mut extra).map_err(|_| RuntimeError::Vault)? != 0 {
            return Err(RuntimeError::Vault);
        }
        Ok(key)
    }

    fn read_state(&self) -> RuntimeResult<VaultState> {
        let path = self.state_path();
        reject_symlink(&path)?;
        let mut file = open_private(&path, false)?;
        let metadata = file.metadata().map_err(|_| RuntimeError::Vault)?;
        if metadata.len() == 0 || metadata.len() > MAX_VAULT_BYTES {
            return Err(RuntimeError::Vault);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        (&mut file)
            .take(MAX_VAULT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RuntimeError::Vault)?;
        if bytes.len() as u64 > MAX_VAULT_BYTES {
            return Err(RuntimeError::Vault);
        }
        let state: VaultState = serde_json::from_slice(&bytes).map_err(|_| RuntimeError::Vault)?;
        if state.version != VAULT_VERSION {
            return Err(RuntimeError::Vault);
        }
        state.target.validate()?;
        Ok(state)
    }

    fn write_state(&self, state: &VaultState) -> RuntimeResult<()> {
        let bytes = serde_json::to_vec(state).map_err(|_| RuntimeError::Vault)?;
        if bytes.len() as u64 > MAX_VAULT_BYTES {
            return Err(RuntimeError::Vault);
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join(format!(".vault-{}-{sequence}.tmp", std::process::id()));
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| RuntimeError::Vault)?;
        if let Err(error) = (|| {
            file.write_all(&bytes).map_err(|_| RuntimeError::Vault)?;
            file.sync_all().map_err(|_| RuntimeError::Vault)?;
            replace_file(&temporary, &self.state_path())?;
            sync_directory(&self.root)
        })() {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok(())
    }

    fn state_path(&self) -> PathBuf {
        self.root.join("vault.json")
    }
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn seal_cursor(
    cursor: &ProviderCursor,
    binding: &CursorVaultBinding,
    key: &[u8; KEY_BYTES],
) -> RuntimeResult<StoredCursor> {
    let mut nonce_bytes = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce_bytes).map_err(|_| RuntimeError::Vault)?;
    let nonce = Nonce::try_from(nonce_bytes.as_slice()).map_err(|_| RuntimeError::Vault)?;
    let aad = serde_json::to_vec(binding).map_err(|_| RuntimeError::Vault)?;
    let plaintext = encode_cursor(cursor);
    let cipher = ChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| RuntimeError::Vault)?;
    Ok(StoredCursor {
        binding: binding.clone(),
        nonce_b64: BASE64.encode(nonce_bytes),
        ciphertext_b64: BASE64.encode(ciphertext),
    })
}

fn open_cursor(stored: &StoredCursor, key: &[u8; KEY_BYTES]) -> RuntimeResult<ProviderCursor> {
    let nonce = BASE64
        .decode(&stored.nonce_b64)
        .map_err(|_| RuntimeError::Vault)?;
    if nonce.len() != NONCE_BYTES {
        return Err(RuntimeError::Vault);
    }
    let ciphertext = BASE64
        .decode(&stored.ciphertext_b64)
        .map_err(|_| RuntimeError::Vault)?;
    let aad = serde_json::to_vec(&stored.binding).map_err(|_| RuntimeError::Vault)?;
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| RuntimeError::Vault)?;
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| RuntimeError::Vault)?;
    decode_cursor(&plaintext)
}

fn encode_cursor(cursor: &ProviderCursor) -> Vec<u8> {
    match cursor {
        ProviderCursor::GithubPage(page) => {
            let mut bytes = vec![1];
            bytes.extend(page.to_be_bytes());
            bytes
        }
        ProviderCursor::GitlabPage(page) => {
            let mut bytes = vec![5];
            bytes.extend(page.to_be_bytes());
            bytes
        }
        ProviderCursor::AwsOrganizations(token) => {
            let mut bytes = vec![2];
            bytes.extend(token.as_bytes());
            bytes
        }
        ProviderCursor::AwsResources(token) => {
            let mut bytes = vec![3];
            bytes.extend(token.as_bytes());
            bytes
        }
        ProviderCursor::GcpAfterKey { operation, key } => {
            let mut bytes = vec![4, *operation];
            bytes.extend(key.as_bytes());
            bytes
        }
    }
}

fn decode_cursor(bytes: &[u8]) -> RuntimeResult<ProviderCursor> {
    match bytes {
        [1, page @ ..] if page.len() == 4 => Ok(ProviderCursor::GithubPage(u32::from_be_bytes(
            page.try_into().map_err(|_| RuntimeError::Vault)?,
        ))),
        [5, page @ ..] if page.len() == 4 => Ok(ProviderCursor::GitlabPage(u32::from_be_bytes(
            page.try_into().map_err(|_| RuntimeError::Vault)?,
        ))),
        [2, token @ ..] => String::from_utf8(token.to_vec())
            .map(ProviderCursor::AwsOrganizations)
            .map_err(|_| RuntimeError::Vault),
        [3, token @ ..] => String::from_utf8(token.to_vec())
            .map(ProviderCursor::AwsResources)
            .map_err(|_| RuntimeError::Vault),
        [4, operation, key @ ..] => String::from_utf8(key.to_vec())
            .map(|key| ProviderCursor::GcpAfterKey {
                operation: *operation,
                key,
            })
            .map_err(|_| RuntimeError::Vault),
        _ => Err(RuntimeError::Vault),
    }
}
