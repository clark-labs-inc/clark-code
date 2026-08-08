use std::fs;
use std::io::Read;
use std::path::Path;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use zeroize::Zeroizing;

use super::{validate_state, PlaintextState, KEY_BYTES, MAX_FILE_BYTES, NONCE_BYTES};
use crate::product::CredentialEnvelopePolicy;

fn empty_state() -> PlaintextState {
    PlaintextState {
        version: 2,
        retained_auth: None,
        code_keys: Default::default(),
        mcp_env: Default::default(),
    }
}

pub(super) fn load_state(
    root: &Path,
    policy: CredentialEnvelopePolicy,
) -> Result<PlaintextState, String> {
    let path = root.join("credentials.enc");
    let encrypted = match read_private(&path)? {
        Some(bytes) => bytes,
        None => return Ok(empty_state()),
    };
    if encrypted.starts_with(policy.obsolete_magic) {
        fs::remove_file(&path)
            .map_err(|_| "could not remove obsolete Agent Desktop credentials".to_string())?;
        return Ok(empty_state());
    }
    if encrypted.len() < policy.magic.len() + NONCE_BYTES + 16
        || &encrypted[..policy.magic.len()] != policy.magic
    {
        return Err("Agent Desktop's encrypted credential file is invalid".into());
    }
    let key = Zeroizing::new(load_or_create_key(root)?);
    let nonce = Nonce::try_from(&encrypted[policy.magic.len()..policy.magic.len() + NONCE_BYTES])
        .map_err(|_| "Agent Desktop's encrypted credential nonce is invalid".to_string())?;
    let cipher = ChaCha20Poly1305::new((&*key).into());
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &encrypted[policy.magic.len() + NONCE_BYTES..],
                    aad: policy.aad,
                },
            )
            .map_err(|_| {
                "Agent Desktop's encrypted credential file failed authentication".to_string()
            })?,
    );
    let state: PlaintextState = serde_json::from_slice(&plaintext)
        .map_err(|_| "Agent Desktop's encrypted credential payload is invalid".to_string())?;
    validate_state(&state)
        .map_err(|_| "Agent Desktop's encrypted credential payload is unsupported".to_string())?;
    Ok(state)
}

pub(super) fn persist_state(
    root: &Path,
    state: PlaintextState,
    policy: CredentialEnvelopePolicy,
) -> Result<(), String> {
    validate_state(&state)?;
    let key = Zeroizing::new(load_or_create_key(root)?);
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&state)
            .map_err(|_| "could not encode Agent Desktop's credential payload".to_string())?,
    );
    let mut nonce_bytes = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|_| "could not generate Agent Desktop credential nonce".to_string())?;
    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| "could not initialize Agent Desktop credential nonce".to_string())?;
    let cipher = ChaCha20Poly1305::new((&*key).into());
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: policy.aad,
            },
        )
        .map_err(|_| "could not encrypt Agent Desktop's credential payload".to_string())?;
    let mut envelope = Vec::with_capacity(policy.magic.len() + NONCE_BYTES + ciphertext.len());
    envelope.extend_from_slice(policy.magic);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&ciphertext);
    write_atomic_private(root, &root.join("credentials.enc"), &envelope)
}

fn load_or_create_key(root: &Path) -> Result<[u8; KEY_BYTES], String> {
    let path = root.join("credentials.key");
    if let Some(bytes) = read_private(&path)? {
        return bytes
            .try_into()
            .map_err(|_| "Agent Desktop's local credential key is invalid".to_string());
    }
    let mut generated = [0_u8; KEY_BYTES];
    getrandom::fill(&mut generated)
        .map_err(|_| "could not generate Agent Desktop's local credential key".to_string())?;
    match exec_private_fs::write_private_new(&path, &generated) {
        Ok(true) => Ok(generated),
        Ok(false) => read_private(&path)?
            .ok_or_else(|| "Agent Desktop's local credential key disappeared".to_string())?
            .try_into()
            .map_err(|_| "Agent Desktop's local credential key is invalid".to_string()),
        Err(_) => Err("could not write Agent Desktop's local credential key".into()),
    }
}

fn read_private(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("could not inspect Agent Desktop's credential file".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_FILE_BYTES {
        return Err("Agent Desktop's credential file is unsafe".into());
    }
    let mut file = exec_private_fs::PrivateFileOptions::new()
        .read(true)
        .open(path)
        .map_err(|_| "could not read Agent Desktop's credential file".to_string())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| "could not read Agent Desktop's credential file".to_string())?;
    Ok(Some(bytes))
}

fn write_atomic_private(root: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = root.join(format!(
        ".credentials-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    exec_private_fs::write_private_new(&temporary, bytes)
        .map_err(|_| "could not write Agent Desktop's encrypted credential file".to_string())?;
    let result = replace_file(&temporary, destination);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination)
        .map_err(|_| "could not install Agent Desktop's encrypted credential file".to_string())?;
    fs::File::open(
        destination
            .parent()
            .ok_or("Agent Desktop credential file has no parent directory")?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(|_| "could not sync Agent Desktop's credential directory".to_string())
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
        .ok_or_else(|| "could not install Agent Desktop's encrypted credential file".to_string())
}
