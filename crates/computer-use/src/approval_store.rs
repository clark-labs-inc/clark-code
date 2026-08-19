use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{ActionReceipt, AppApproval, ApplicationIdentity, ApprovalSnapshot, ComputerUseError};

const STORE_VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 2 * 1_048_576;
#[cfg(any(feature = "helper-service", test))]
const MAX_RECEIPTS: usize = 250;

#[derive(Clone, Debug)]
pub struct ApprovalStore {
    root: Arc<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredApproval {
    identity: ApplicationIdentity,
    app_name: String,
    granted_at_ms: u64,
    last_used_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoreState {
    version: u32,
    revision: u64,
    approvals: Vec<StoredApproval>,
    receipts: Vec<ActionReceipt>,
}

impl Default for StoreState {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            revision: 0,
            approvals: Vec::new(),
            receipts: Vec::new(),
        }
    }
}

/// Shared lock held for the complete input lease. Revocation takes the
/// exclusive side of the same lock and therefore cannot acknowledge until all
/// earlier synthesized or Accessibility input has quiesced.
#[cfg(any(all(feature = "helper-service", target_os = "macos"), test))]
pub(crate) struct ApprovalActionGuard {
    _lock: File,
    pub revision: u64,
}

impl ApprovalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
        }
    }

    pub fn root(&self) -> &Path {
        self.root.as_ref()
    }

    pub fn snapshot(&self) -> Result<ApprovalSnapshot, ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(store_error)?;
        let state = self.read_state()?;
        let mut approvals = state
            .approvals
            .into_iter()
            .map(|approval| AppApproval {
                identity_key: approval.identity.identity_key,
                bundle_id: approval.identity.bundle_id,
                app_name: approval.app_name,
                team_identifier: approval.identity.team_identifier,
                granted_at_ms: approval.granted_at_ms,
                last_used_at_ms: approval.last_used_at_ms,
            })
            .collect::<Vec<_>>();
        approvals.sort_by(|left, right| {
            left.app_name
                .to_ascii_lowercase()
                .cmp(&right.app_name.to_ascii_lowercase())
                .then_with(|| left.bundle_id.cmp(&right.bundle_id))
        });
        Ok(ApprovalSnapshot {
            revision: state.revision,
            approvals,
        })
    }

    pub fn is_granted(
        &self,
        identity: &ApplicationIdentity,
    ) -> Result<(bool, u64), ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(store_error)?;
        let state = self.read_state()?;
        Ok((
            state
                .approvals
                .iter()
                .any(|approval| approval.identity == *identity),
            state.revision,
        ))
    }

    pub fn grant(
        &self,
        identity: ApplicationIdentity,
        app_name: impl Into<String>,
    ) -> Result<u64, ComputerUseError> {
        if !identity.durable_approval_eligible {
            return Err(ComputerUseError::ApprovalStore(
                "the target has no durable signer identity".to_string(),
            ));
        }
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(store_error)?;
        let mut state = self.read_state()?;
        let now = now_ms();
        let app_name = app_name.into();
        if let Some(existing) = state
            .approvals
            .iter_mut()
            .find(|approval| approval.identity == identity)
        {
            existing.app_name = app_name;
            existing.last_used_at_ms = now;
            self.write_state(&state)?;
            return Ok(state.revision);
        }
        state.revision = state.revision.saturating_add(1);
        state.approvals.push(StoredApproval {
            identity,
            app_name,
            granted_at_ms: now,
            last_used_at_ms: now,
        });
        self.write_state(&state)?;
        Ok(state.revision)
    }

    pub fn revoke(&self, identity_key: &str) -> Result<bool, ComputerUseError> {
        let identity_key = identity_key.trim();
        if identity_key.is_empty() || identity_key.len() > 128 {
            return Err(ComputerUseError::ApprovalStore(
                "approval identity key is empty or oversized".to_string(),
            ));
        }
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(store_error)?;
        let mut state = self.read_state()?;
        let before = state.approvals.len();
        state
            .approvals
            .retain(|approval| approval.identity.identity_key != identity_key);
        let removed = state.approvals.len() != before;
        if removed {
            state.revision = state.revision.saturating_add(1);
            self.write_state(&state)?;
        }
        Ok(removed)
    }

    pub fn revoke_all(&self) -> Result<usize, ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(store_error)?;
        let mut state = self.read_state()?;
        let removed = state.approvals.len();
        if removed > 0 {
            state.approvals.clear();
            state.revision = state.revision.saturating_add(1);
            self.write_state(&state)?;
        }
        Ok(removed)
    }

    pub fn recent_receipts(&self) -> Result<Vec<ActionReceipt>, ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(store_error)?;
        Ok(self.read_state()?.receipts)
    }

    #[cfg(any(feature = "helper-service", test))]
    pub(crate) fn record_receipt(
        &self,
        mut receipt: ActionReceipt,
    ) -> Result<(), ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_exclusive(&lock).map_err(store_error)?;
        let mut state = self.read_state()?;
        // The serialized copy is itself the proof that persistence succeeded.
        // Callers construct receipts before attempting the write, so do not
        // preserve their provisional `false` value in the durable ledger.
        receipt.persisted = true;
        if let Some(approval) = state
            .approvals
            .iter_mut()
            .find(|approval| approval.identity.identity_key == receipt.application_identity_key)
        {
            approval.last_used_at_ms = approval.last_used_at_ms.max(receipt.completed_at_ms);
        }
        state.receipts.push(receipt);
        if state.receipts.len() > MAX_RECEIPTS {
            let remove = state.receipts.len() - MAX_RECEIPTS;
            state.receipts.drain(..remove);
        }
        self.write_state(&state)
    }

    #[cfg(any(all(feature = "helper-service", target_os = "macos"), test))]
    pub(crate) fn begin_action(
        &self,
        identity: &ApplicationIdentity,
        expected_revision: u64,
        requires_durable_grant: bool,
    ) -> Result<ApprovalActionGuard, ComputerUseError> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).map_err(store_error)?;
        let state = self.read_state()?;
        if state.revision != expected_revision {
            return Err(ComputerUseError::ApprovalRequired);
        }
        if requires_durable_grant
            && !state
                .approvals
                .iter()
                .any(|approval| approval.identity == *identity)
        {
            return Err(ComputerUseError::ApprovalRequired);
        }
        Ok(ApprovalActionGuard {
            _lock: lock,
            revision: state.revision,
        })
    }

    fn open_lock(&self) -> Result<File, ComputerUseError> {
        self.ensure_root()?;
        let path = self.root.join("approvals.lock");
        reject_symlink(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(store_error)?;
        secure_file_mode(&file)?;
        Ok(file)
    }

    fn ensure_root(&self) -> Result<(), ComputerUseError> {
        if self.root.exists() {
            let metadata = fs::symlink_metadata(self.root.as_ref()).map_err(store_error)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ComputerUseError::ApprovalStore(format!(
                    "{} must be a real directory",
                    self.root.display()
                )));
            }
        } else {
            fs::create_dir_all(self.root.as_ref()).map_err(store_error)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.root.as_ref(), fs::Permissions::from_mode(0o700))
                .map_err(store_error)?;
        }
        Ok(())
    }

    fn read_state(&self) -> Result<StoreState, ComputerUseError> {
        let path = self.root.join("approvals.json");
        reject_symlink(&path)?;
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreState::default())
            }
            Err(error) => return Err(store_error(error)),
        };
        let metadata = file.metadata().map_err(store_error)?;
        if metadata.len() > MAX_STORE_BYTES {
            return Err(ComputerUseError::ApprovalStore(format!(
                "{} exceeds the {} byte limit",
                path.display(),
                MAX_STORE_BYTES
            )));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes).map_err(store_error)?;
        if bytes.is_empty() {
            return Ok(StoreState::default());
        }
        let state: StoreState = serde_json::from_slice(&bytes)
            .map_err(|error| ComputerUseError::ApprovalStore(error.to_string()))?;
        if state.version != STORE_VERSION {
            return Err(ComputerUseError::ApprovalStore(format!(
                "unsupported approval store version {}",
                state.version
            )));
        }
        Ok(state)
    }

    fn write_state(&self, state: &StoreState) -> Result<(), ComputerUseError> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| ComputerUseError::ApprovalStore(error.to_string()))?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(ComputerUseError::ApprovalStore(
                "approval store exceeds its bounded size".to_string(),
            ));
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = self
            .root
            .join(format!(".approvals-{}-{sequence}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(store_error)?;
        if let Err(error) = (|| {
            secure_file_mode(&file)?;
            file.write_all(&bytes).map_err(store_error)?;
            file.sync_all().map_err(store_error)?;
            fs::rename(&temporary, self.root.join("approvals.json")).map_err(store_error)?;
            sync_store_directory(self.root.as_ref())
        })() {
            drop(fs::remove_file(&temporary));
            return Err(error);
        }
        Ok(())
    }
}

static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn default_approval_store() -> Result<ApprovalStore, ComputerUseError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("DESKTOP_COMPUTER_USE_DATA_DIR") {
        return Ok(ApprovalStore::new(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        ComputerUseError::ApprovalStore("HOME is unavailable for native approval storage".into())
    })?;
    #[cfg(target_os = "macos")]
    let root = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(env!("DESKTOP_COMPUTER_USE_MAC_SUPPORT_NAME"))
        .join("Computer Use");
    #[cfg(not(target_os = "macos"))]
    let root = PathBuf::from(home)
        .join(env!("DESKTOP_COMPUTER_USE_DATA_NAMESPACE"))
        .join("computer-use");
    Ok(ApprovalStore::new(root))
}

fn reject_symlink(path: &Path) -> Result<(), ComputerUseError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ComputerUseError::ApprovalStore(
            format!("{} must not be a symbolic link", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(store_error(error)),
    }
}

fn secure_file_mode(file: &File) -> Result<(), ComputerUseError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(store_error)?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn sync_store_directory(path: &Path) -> Result<(), ComputerUseError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(store_error)
    }
    #[cfg(not(unix))]
    {
        // Windows does not allow std::fs::File::open on a directory. The state
        // file itself is flushed before the atomic replace; there is no portable
        // directory-fsync equivalent exposed by std on this platform.
        let _ = path;
        Ok(())
    }
}

fn store_error(error: std::io::Error) -> ComputerUseError {
    ComputerUseError::ApprovalStore(error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(key: &str) -> ApplicationIdentity {
        ApplicationIdentity {
            bundle_id: "com.example.fixture".to_string(),
            team_identifier: Some("TEAM123".to_string()),
            designated_requirement: "identifier fixture and anchor apple generic".to_string(),
            identity_key: key.to_string(),
            durable_approval_eligible: true,
        }
    }

    #[test]
    fn grants_are_signer_bound_and_revocation_advances_revision() {
        let root = tempfile::tempdir().unwrap();
        let store = ApprovalStore::new(root.path().join("store"));
        let first = identity("identity-a");
        let impostor = identity("identity-b");

        let revision = store.grant(first.clone(), "Fixture").unwrap();
        assert_eq!(revision, 1);
        assert_eq!(store.is_granted(&first).unwrap(), (true, 1));
        assert_eq!(store.is_granted(&impostor).unwrap(), (false, 1));
        assert!(store.revoke("identity-a").unwrap());
        assert_eq!(store.snapshot().unwrap().revision, 2);
        assert!(!store.is_granted(&first).unwrap().0);
    }

    #[test]
    fn active_action_lock_delays_revocation_ack() {
        let root = tempfile::tempdir().unwrap();
        let store = ApprovalStore::new(root.path().join("store"));
        let app = identity("identity-a");
        let revision = store.grant(app.clone(), "Fixture").unwrap();
        let guard = store.begin_action(&app, revision, true).unwrap();
        assert_eq!(guard.revision, revision);
        let revoke_store = store.clone();
        let revoke = std::thread::spawn(move || revoke_store.revoke("identity-a").unwrap());

        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!revoke.is_finished());
        drop(guard);
        assert!(revoke.join().unwrap());
    }

    #[test]
    fn receipts_are_bounded_and_payloads_remain_redacted() {
        let root = tempfile::tempdir().unwrap();
        let store = ApprovalStore::new(root.path().join("store"));
        for index in 0..MAX_RECEIPTS + 3 {
            store
                .record_receipt(ActionReceipt {
                    receipt_id: format!("receipt-{index}"),
                    prepared_action_id: format!("prepared-{index}"),
                    application_identity_key: "identity-a".to_string(),
                    bundle_id: "com.example.fixture".to_string(),
                    pid: 42,
                    window_id: 7,
                    action_kind: crate::ActionKind::TypeText,
                    disposition: crate::ActionDisposition::PreapprovalEligible,
                    outcome: crate::ReceiptOutcome::Succeeded,
                    payload_summary: "text redacted (8 characters)".to_string(),
                    completed_at_ms: index as u64,
                    persisted: false,
                })
                .unwrap();
        }
        let receipts = store.recent_receipts().unwrap();
        assert_eq!(receipts.len(), MAX_RECEIPTS);
        assert_eq!(receipts[0].receipt_id, "receipt-3");
        assert!(receipts.iter().all(|receipt| receipt.persisted));
        assert!(!serde_json::to_string(&receipts)
            .unwrap()
            .contains("password"));
    }

    #[test]
    fn recording_a_receipt_updates_the_matching_approval_last_used_time() {
        let root = tempfile::tempdir().unwrap();
        let store = ApprovalStore::new(root.path().join("store"));
        let app = identity("identity-a");
        store.grant(app, "Fixture").unwrap();
        let granted_at = store.snapshot().unwrap().approvals[0].last_used_at_ms;
        let completed_at = granted_at.saturating_add(1_000);

        store
            .record_receipt(ActionReceipt {
                receipt_id: "receipt".to_string(),
                prepared_action_id: "prepared".to_string(),
                application_identity_key: "identity-a".to_string(),
                bundle_id: "com.example.fixture".to_string(),
                pid: 42,
                window_id: 7,
                action_kind: crate::ActionKind::Click,
                disposition: crate::ActionDisposition::Allow,
                outcome: crate::ReceiptOutcome::Succeeded,
                payload_summary: "no sensitive payload".to_string(),
                completed_at_ms: completed_at,
                persisted: false,
            })
            .unwrap();

        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.approvals[0].last_used_at_ms, completed_at);
        assert!(store.recent_receipts().unwrap()[0].persisted);
    }
}
