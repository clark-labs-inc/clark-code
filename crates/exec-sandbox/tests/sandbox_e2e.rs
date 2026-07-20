use std::path::Path;
use std::time::Duration;

use exec_core::{run_process_streaming, Executor, ProcessSpec};
#[cfg(target_os = "linux")]
use exec_sandbox::SandboxRuntime;
use exec_sandbox::{SandboxManager, SandboxPolicy, SandboxStatus, SandboxedExecutor};
use tokio_util::sync::CancellationToken;

const PROBE_MODE: &str = "CLARK_SANDBOX_TEST_PROBE";

#[test]
fn sandbox_network_probe_child() {
    if std::env::var(PROBE_MODE).as_deref() != Ok("network") {
        return;
    }
    let address = std::env::var("CLARK_SANDBOX_TEST_ADDRESS").unwrap();
    let address = address.parse().unwrap();
    let connected = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok();
    std::process::exit(if connected { 0 } else { 73 });
}

#[tokio::test]
async fn workspace_write_policy_holds_at_direct_process_and_network_boundaries() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join(".git")).unwrap();

    let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
    let manager = SandboxManager::current(policy.clone()).unwrap();
    #[cfg(windows)]
    let manager = setup_windows_if_required(manager, &policy);
    if !matches!(manager.status(), SandboxStatus::Enforced { .. }) {
        assert!(
            std::env::var_os("CLARK_SANDBOX_E2E_REQUIRED").is_none(),
            "native sandbox is required but unavailable: {:?}",
            manager.status()
        );
        eprintln!("native sandbox unavailable: {:?}", manager.status());
        return;
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(expected) = std::env::var_os("CLARK_BWRAP_PATH") {
            let prepared = manager
                .prepare_process(ProcessSpec::argv("/bin/true", workspace.path()))
                .unwrap();
            assert_eq!(
                prepared.program,
                Path::new(&expected),
                "the explicitly staged release helper must be the enforced backend"
            );
        }
        let fallback = SandboxManager::current_with_runtime(
            policy.clone(),
            SandboxRuntime {
                linux_bubblewrap: Some(Path::new("/bin/false").to_path_buf()),
                ..SandboxRuntime::default()
            },
        )
        .unwrap();
        assert!(
            matches!(fallback.status(), SandboxStatus::Enforced { .. }),
            "an unusable bundled helper must not mask the distro bubblewrap: {:?}",
            fallback.status()
        );
    }
    let executor = SandboxedExecutor::with_manager(manager).unwrap();

    executor
        .write(&workspace.path().join("direct.txt"), b"inside")
        .await
        .unwrap();
    assert!(executor
        .write(&outside.path().join("direct.txt"), b"outside")
        .await
        .is_err());
    assert!(executor
        .write(&workspace.path().join(".git/config"), b"mutation")
        .await
        .is_err());

    let cancel = CancellationToken::new();
    let inside = workspace.path().join("process.txt");
    let output = executor
        .exec(
            &write_command(&inside),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(output.code, Some(0), "{:?}", output.stderr);
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "inside");

    let outside_process = outside.path().join("process.txt");
    let output = executor
        .exec(
            &write_command(&outside_process),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_ne!(output.code, Some(0));
    assert!(!outside_process.exists());

    let git_config = workspace.path().join(".git/config");
    let output = executor
        .exec(
            &write_command(&git_config),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_ne!(output.code, Some(0));
    assert!(!git_config.exists());

    #[cfg(unix)]
    {
        let escape = workspace.path().join("escape-link");
        std::os::unix::fs::symlink(outside.path(), &escape).unwrap();
        let symlink_escape = escape.join("process-symlink.txt");
        let output = executor
            .exec(
                &write_command(&symlink_escape),
                workspace.path(),
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        assert_ne!(output.code, Some(0));
        assert!(!outside.path().join("process-symlink.txt").exists());
    }

    let pty_inside = workspace.path().join("pty.txt");
    let output = executor
        .exec_streaming_pty(
            &write_command(&pty_inside),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
            &|_, _| {},
        )
        .await
        .unwrap();
    assert_eq!(output.code, Some(0), "{:?}", output.stderr);
    assert!(pty_inside.exists());

    assert_network_is_denied(&executor, workspace.path(), &cancel).await;
}

#[tokio::test]
async fn read_only_policy_denies_project_mutation_but_keeps_private_temp_usable() {
    let workspace = tempfile::tempdir().unwrap();
    let private_temp = tempfile::tempdir().unwrap();
    let policy =
        SandboxPolicy::read_only().with_process_temp_root(private_temp.path().to_path_buf());
    let manager = SandboxManager::current(policy.clone()).unwrap();
    #[cfg(windows)]
    let manager = setup_windows_if_required(manager, &policy);
    if !matches!(manager.status(), SandboxStatus::Enforced { .. }) {
        assert!(
            std::env::var_os("CLARK_SANDBOX_E2E_REQUIRED").is_none(),
            "native sandbox is required but unavailable: {:?}",
            manager.status()
        );
        eprintln!("native sandbox unavailable: {:?}", manager.status());
        return;
    }
    let executor = SandboxedExecutor::with_manager(manager).unwrap();
    let cancel = CancellationToken::new();

    let project_write = workspace.path().join("readonly-escape.txt");
    let output = executor
        .exec(
            &write_command(&project_write),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_ne!(output.code, Some(0));
    assert!(!project_write.exists());

    let temp_write = private_temp.path().join("tool-temp.txt");
    let output = executor
        .exec(
            &write_command(&temp_write),
            workspace.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(output.code, Some(0), "{:?}", output.stderr);
    assert_eq!(std::fs::read_to_string(&temp_write).unwrap(), "inside");
}

#[cfg(windows)]
fn setup_windows_if_required(manager: SandboxManager, policy: &SandboxPolicy) -> SandboxManager {
    if std::env::var_os("CLARK_SANDBOX_E2E_REQUIRED").is_none()
        || !matches!(manager.status(), SandboxStatus::SetupRequired { .. })
        || !manager.setup_available()
    {
        return manager;
    }
    let action = manager
        .setup_action()
        .unwrap()
        .expect("Windows setup action");
    let status = std::process::Command::new(&action.program)
        .args(&action.args)
        .status()
        .unwrap();
    for proof in action.cleanup_paths {
        let _ = std::fs::remove_file(proof);
    }
    assert!(status.success(), "Windows sandbox setup failed: {status}");
    SandboxManager::current(policy.clone()).unwrap()
}

async fn assert_network_is_denied(
    executor: &SandboxedExecutor,
    cwd: &Path,
    cancel: &CancellationToken,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let process = ProcessSpec::argv(std::env::current_exe().unwrap(), cwd)
        .args(["--exact", "sandbox_network_probe_child", "--nocapture"])
        .env(PROBE_MODE, "network")
        .env("CLARK_SANDBOX_TEST_ADDRESS", address.to_string());
    let process = executor.prepare_process(process).unwrap();
    let output = run_process_streaming(&process, Duration::from_secs(10), cancel, &|_, _| {})
        .await
        .unwrap();
    assert_ne!(
        output.code,
        Some(0),
        "sandboxed child unexpectedly connected to host listener"
    );
}

#[cfg(unix)]
fn write_command(path: &Path) -> String {
    format!(
        "printf inside > '{}'",
        path.to_string_lossy().replace('\'', "'\\''")
    )
}

#[cfg(windows)]
fn write_command(path: &Path) -> String {
    format!(
        "Set-Content -LiteralPath '{}' -NoNewline -Value inside",
        path.to_string_lossy().replace('\'', "''")
    )
}
