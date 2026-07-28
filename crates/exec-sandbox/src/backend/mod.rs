mod linux;
mod macos;
mod windows;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use exec_core::ProcessSpec;

use crate::SandboxPolicy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    MacosSeatbelt,
    LinuxBubblewrap,
    WindowsRestrictedToken,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxStatus {
    Enforced {
        backend: BackendKind,
    },
    Unavailable {
        backend: BackendKind,
        reason: String,
    },
    SetupRequired {
        backend: BackendKind,
        reason: String,
    },
}

/// Explicit setup command to present through product consent UI. Setup is
/// never launched implicitly while compiling an agent-owned process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxSetupAction {
    pub program: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub requires_elevation: bool,
    /// Best-effort cleanup owned by the unelevated host, regardless of whether
    /// the elevated process succeeds, fails, or is cancelled at the UAC prompt.
    pub cleanup_paths: Vec<PathBuf>,
}

/// Product-owned helper locations. Discovery stays outside the sandbox crate,
/// so desktop, CLI, tests, and future hosts can package helpers differently.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SandboxRuntime {
    pub linux_bubblewrap: Option<PathBuf>,
    pub windows_runner: Option<PathBuf>,
    pub windows_setup: Option<PathBuf>,
    pub windows_state_dir: Option<PathBuf>,
}

/// Platform adapter boundary. Built-in backends compile the shared policy to
/// Seatbelt, bubblewrap, or the Windows runner protocol; downstream products
/// can supply another backend without changing the executor or policy model.
pub trait SandboxBackend: fmt::Debug + Send + Sync {
    fn kind(&self) -> BackendKind;
    fn status(&self) -> &SandboxStatus;
    fn prepare(&self, policy: &SandboxPolicy, process: ProcessSpec) -> Result<ProcessSpec, String>;

    fn setup_action(&self, _policy: &SandboxPolicy) -> Result<Option<SandboxSetupAction>, String> {
        Ok(None)
    }

    fn setup_available(&self) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct SandboxManager {
    policy: SandboxPolicy,
    backend: Arc<dyn SandboxBackend>,
}

impl fmt::Debug for SandboxManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SandboxManager")
            .field("policy", &self.policy)
            .field("backend", &self.backend.kind())
            .field("status", self.backend.status())
            .finish()
    }
}

impl SandboxManager {
    pub fn current(policy: SandboxPolicy) -> Result<Self, String> {
        Self::current_with_runtime(policy, SandboxRuntime::default())
    }

    pub fn current_with_runtime(
        policy: SandboxPolicy,
        runtime: SandboxRuntime,
    ) -> Result<Self, String> {
        let backend = current_backend(&policy, runtime)?;
        Ok(Self::with_backend(policy, backend))
    }

    /// Construct a backend without probing the host. Used by cross-platform
    /// policy simulations and compiler snapshots, never by production launch.
    pub fn simulate(policy: SandboxPolicy, backend: BackendKind, helper: PathBuf) -> Self {
        Self::with_backend(
            policy,
            Arc::new(BuiltinBackend::new(
                backend,
                helper,
                SandboxStatus::Enforced { backend },
                (backend == BackendKind::WindowsRestrictedToken)
                    .then(|| PathBuf::from("/clark-windows-sandbox-state")),
                (backend == BackendKind::WindowsRestrictedToken)
                    .then(|| PathBuf::from("/clark-windows-sandbox-setup.exe")),
            )),
        )
    }

    pub fn with_backend(policy: SandboxPolicy, backend: Arc<dyn SandboxBackend>) -> Self {
        Self { policy, backend }
    }

    pub fn status(&self) -> &SandboxStatus {
        self.backend.status()
    }

    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    pub fn prepare_process(&self, process: ProcessSpec) -> Result<ProcessSpec, String> {
        if !matches!(self.backend.status(), SandboxStatus::Enforced { .. }) {
            return Err(status_error(self.backend.status()));
        }
        let process = self.prepare_environment(process);
        self.backend.prepare(&self.policy, process)
    }

    pub fn setup_action(&self) -> Result<Option<SandboxSetupAction>, String> {
        self.backend.setup_action(&self.policy)
    }

    /// Whether a complete setup boundary is installed. Unlike `setup_action`,
    /// this never creates ephemeral ownership proofs or mutates the host.
    pub fn setup_available(&self) -> bool {
        self.backend.setup_available()
    }

    fn prepare_environment(&self, mut process: ProcessSpec) -> ProcessSpec {
        let Some(root) = self.policy.process_temp_root.as_ref() else {
            return process;
        };
        process.env.retain(|(name, _)| {
            !matches!(
                name.to_string_lossy().to_ascii_uppercase().as_str(),
                "TMPDIR" | "TMP" | "TEMP"
            )
        });
        for name in ["TMPDIR", "TMP", "TEMP"] {
            process
                .env
                .push((name.into(), root.as_os_str().to_os_string()));
        }
        process
    }
}

#[derive(Debug)]
struct BuiltinBackend {
    kind: BackendKind,
    helper: PathBuf,
    status: SandboxStatus,
    windows_state_dir: Option<PathBuf>,
    windows_setup: Option<PathBuf>,
}

impl BuiltinBackend {
    fn new(
        kind: BackendKind,
        helper: PathBuf,
        status: SandboxStatus,
        windows_state_dir: Option<PathBuf>,
        windows_setup: Option<PathBuf>,
    ) -> Self {
        Self {
            kind,
            helper,
            status,
            windows_state_dir,
            windows_setup,
        }
    }
}

impl SandboxBackend for BuiltinBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn status(&self) -> &SandboxStatus {
        &self.status
    }

    fn prepare(&self, policy: &SandboxPolicy, process: ProcessSpec) -> Result<ProcessSpec, String> {
        match self.kind {
            BackendKind::MacosSeatbelt => macos::prepare(policy, &self.helper, process),
            BackendKind::LinuxBubblewrap => linux::prepare(policy, &self.helper, process),
            BackendKind::WindowsRestrictedToken => windows::prepare(
                policy,
                &self.helper,
                self.windows_state_dir
                    .as_deref()
                    .ok_or_else(|| "Windows sandbox state directory is missing".to_string())?,
                process,
            ),
        }
    }

    fn setup_action(&self, policy: &SandboxPolicy) -> Result<Option<SandboxSetupAction>, String> {
        if self.kind != BackendKind::WindowsRestrictedToken {
            return Ok(None);
        }
        let setup = self
            .windows_setup
            .as_deref()
            .ok_or_else(|| "Windows sandbox setup helper is missing".to_string())?;
        let state_dir = self
            .windows_state_dir
            .as_deref()
            .ok_or_else(|| "Windows sandbox state directory is missing".to_string())?;
        windows::setup_action(policy, &self.helper, setup, state_dir).map(Some)
    }

    fn setup_available(&self) -> bool {
        self.kind == BackendKind::WindowsRestrictedToken
            && self.helper.is_file()
            && self
                .windows_setup
                .as_ref()
                .is_some_and(|setup| setup.is_file())
            && self.windows_state_dir.is_some()
    }
}

fn status_error(status: &SandboxStatus) -> String {
    match status {
        SandboxStatus::Enforced { .. } => "sandbox is ready".to_string(),
        SandboxStatus::Unavailable { reason, .. } => format!("sandbox unavailable: {reason}"),
        SandboxStatus::SetupRequired { reason, .. } => {
            format!("sandbox setup required: {reason}")
        }
    }
}

fn current_backend(
    policy: &SandboxPolicy,
    runtime: SandboxRuntime,
) -> Result<Arc<dyn SandboxBackend>, String> {
    let _ = policy;
    let _ = &runtime;
    #[cfg(target_os = "macos")]
    {
        let backend = BackendKind::MacosSeatbelt;
        let helper = PathBuf::from("/usr/bin/sandbox-exec");
        let status = if helper.is_file() {
            SandboxStatus::Enforced { backend }
        } else {
            SandboxStatus::Unavailable {
                backend,
                reason: "/usr/bin/sandbox-exec is missing".to_string(),
            }
        };
        return Ok(Arc::new(BuiltinBackend::new(
            backend, helper, status, None, None,
        )));
    }

    #[cfg(target_os = "linux")]
    {
        let backend = BackendKind::LinuxBubblewrap;
        let (helper, status) = select_linux_bwrap(runtime.linux_bubblewrap);
        return Ok(Arc::new(BuiltinBackend::new(
            backend, helper, status, None, None,
        )));
    }

    #[cfg(target_os = "windows")]
    {
        let backend = BackendKind::WindowsRestrictedToken;
        let helper = runtime
            .windows_runner
            .or_else(|| std::env::var_os("CLARK_WINDOWS_SANDBOX_RUNNER").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("clark-command-runner.exe"));
        let setup = runtime
            .windows_setup
            .or_else(|| std::env::var_os("CLARK_WINDOWS_SANDBOX_SETUP").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("clark-windows-sandbox-setup.exe"));
        let state_dir = runtime
            .windows_state_dir
            .or_else(|| std::env::var_os("CLARK_WINDOWS_SANDBOX_STATE_DIR").map(PathBuf::from))
            .or_else(default_windows_state_dir);
        let policy_validation = windows::wire_policy(policy).validate_windows_enforceable();
        let status = if let Err(reason) = policy_validation {
            SandboxStatus::Unavailable { backend, reason }
        } else if !helper.is_file() || !setup.is_file() {
            SandboxStatus::SetupRequired {
                backend,
                reason: "the signed Windows sandbox runner and setup helper are not installed"
                    .to_string(),
            }
        } else if let Some(state_dir) = state_dir.as_ref() {
            match exec_sandbox_protocol::read_setup_marker(state_dir).and_then(|marker| {
                marker.validate_for_runner(&helper)?;
                marker.validate_for_policy(&windows::wire_policy(policy))
            }) {
                Ok(()) => SandboxStatus::Enforced { backend },
                Err(reason) => SandboxStatus::SetupRequired { backend, reason },
            }
        } else {
            SandboxStatus::SetupRequired {
                backend,
                reason: "the Windows sandbox state directory is unavailable".to_string(),
            }
        };
        return Ok(Arc::new(BuiltinBackend::new(
            backend,
            helper,
            status,
            state_dir,
            Some(setup),
        )));
    }

    #[allow(unreachable_code)]
    Err("Clark has no sandbox backend for this operating system".to_string())
}

#[cfg(target_os = "windows")]
fn default_windows_state_dir() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let durable =
        windows_product_data_root_from(&local_app_data, cfg!(debug_assertions)).join("sandbox");
    migrate_legacy_windows_state(&local_app_data, &durable);
    Some(durable)
}

#[cfg(target_os = "windows")]
fn migrate_legacy_windows_state(local_app_data: &Path, durable: &Path) {
    if durable.exists() {
        return;
    }
    let legacy_product = if cfg!(debug_assertions) {
        "Clark Code Dev"
    } else {
        "Clark Code"
    };
    let legacy = local_app_data.join(legacy_product).join("sandbox");
    let Ok(metadata) = std::fs::symlink_metadata(&legacy) else {
        return;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return;
    }
    let Some(parent) = durable.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_ok() {
        let _ = std::fs::rename(legacy, durable);
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_product_data_root_from(local_app_data: &Path, debug_build: bool) -> PathBuf {
    local_app_data
        .join("Clark")
        .join(if debug_build { "Code Dev" } else { "Code" })
}

#[cfg(target_os = "linux")]
fn linux_bwrap_ready(helper: &Path) -> Result<(), String> {
    let output = std::process::Command::new(helper)
        .args(["--ro-bind", "/", "/", "--unshare-user", "--", "/bin/true"])
        .output()
        .map_err(|error| format!("failed to probe bubblewrap: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "bubblewrap cannot create a sandbox on this host: {}",
            stderr.trim().chars().take(300).collect::<String>()
        ))
    }
}

#[cfg(target_os = "linux")]
fn select_linux_bwrap(bundled: Option<PathBuf>) -> (PathBuf, SandboxStatus) {
    let backend = BackendKind::LinuxBubblewrap;
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os("CLARK_BWRAP_PATH").map(PathBuf::from) {
        candidates.push(explicit);
    }
    if let Some(bundled) = bundled {
        candidates.push(bundled);
    }
    candidates.extend([PathBuf::from("/usr/bin/bwrap"), PathBuf::from("/bin/bwrap")]);
    candidates.dedup();

    let mut failures = Vec::new();
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        match linux_bwrap_ready(&candidate) {
            Ok(()) => return (candidate, SandboxStatus::Enforced { backend }),
            Err(reason) => failures.push(reason),
        }
    }

    let reason = if failures.is_empty() {
        "bubblewrap is not installed or bundled".to_string()
    } else {
        failures.join("; ")
    };
    (
        PathBuf::from("bwrap"),
        SandboxStatus::Unavailable { backend, reason },
    )
}

fn original_parts(process: ProcessSpec) -> (PathBuf, Vec<std::ffi::OsString>, PathBuf) {
    (process.program, process.args, process.cwd)
}

fn append_inner_command(
    args: &mut Vec<std::ffi::OsString>,
    program: &Path,
    inner_args: Vec<std::ffi::OsString>,
) {
    args.push(program.as_os_str().to_os_string());
    args.extend(inner_args);
}
