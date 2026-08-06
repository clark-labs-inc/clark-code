#![cfg(windows)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use exec_sandbox_protocol::{
    WindowsSetupRequest, WireNetworkPolicy, WireSandboxPolicy, SETUP_PROTOCOL_VERSION,
};

#[path = "windows_native/support.rs"]
mod support;
use support::*;

#[test]
fn native_windows_sandbox_enforces_filesystem_process_and_network_boundaries() {
    if std::env::var_os("CLARK_WINDOWS_SANDBOX_E2E_REQUIRED").is_none() {
        eprintln!("set CLARK_WINDOWS_SANDBOX_E2E_REQUIRED=1 to run the machine-mutating test");
        return;
    }

    let runner = required_helper("CLARK_WINDOWS_SANDBOX_RUNNER");
    let setup = required_helper("CLARK_WINDOWS_SANDBOX_SETUP");
    assert_eq!(runner.parent(), setup.parent(), "helpers must be siblings");
    let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    let state_dir = local_app_data.join("Clark").join("Code").join("sandbox");
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let process_temp_guard = tempfile::tempdir().unwrap();
    let process_temp = process_temp_guard.path().to_path_buf();
    let dot_git = workspace.path().join(".git");
    let second_workspace = tempfile::tempdir().unwrap();
    let second_temp = second_workspace.path().join(".clark-sandbox-tmp");
    std::fs::create_dir_all(&process_temp).unwrap();
    std::fs::create_dir_all(&second_temp).unwrap();
    let git = find_program("git.exe");
    if let Some(git) = &git {
        let git_init = Command::new(git)
            .args(["init", "--quiet"])
            .arg(workspace.path())
            .output()
            .unwrap();
        assert_success("git fixture", &git_init);
    } else {
        // Git is optional for Clark Code and is intentionally not bundled.
        // Keep the protected-metadata boundary covered on a clean machine.
        std::fs::create_dir_all(&dot_git).unwrap();
    }

    let policy = WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: vec![workspace.path().to_path_buf(), process_temp.clone()],
        deny_read: Vec::new(),
        deny_write: vec![dot_git.clone()],
        network: WireNetworkPolicy::Restricted,
        process_temp_root: Some(process_temp.clone()),
    };
    let second_policy = WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: vec![second_workspace.path().to_path_buf(), second_temp.clone()],
        deny_read: Vec::new(),
        deny_write: Vec::new(),
        network: WireNetworkPolicy::Restricted,
        process_temp_root: Some(second_temp),
    };
    // The first project performs the one-time machine bootstrap and enrollment
    // behind one UAC consent transaction.
    let setup_id = format!("native-setup-{}", std::process::id());
    let setup_request = WindowsSetupRequest {
        protocol_version: SETUP_PROTOCOL_VERSION,
        request_id: setup_id.clone(),
        state_dir: state_dir.clone(),
        runner_path: runner.clone(),
        policy: policy.clone(),
        root_proofs: proof_roots(&policy, &setup_id),
    };
    assert_initial_setup_success(
        &setup,
        &[&setup_request],
        "initial bootstrap and enrollment",
    );

    // A later user-owned project is enrolled in-process without launching the
    // elevated helper again. Runtime tokens still select only the active
    // policy's root capability SID, so enrollment does not combine authority.
    let second_setup_id = format!("native-second-{}", std::process::id());
    let second_setup_request = WindowsSetupRequest {
        protocol_version: SETUP_PROTOCOL_VERSION,
        request_id: second_setup_id.clone(),
        state_dir: state_dir.clone(),
        runner_path: runner.clone(),
        policy: second_policy.clone(),
        root_proofs: proof_roots(&second_policy, &second_setup_id),
    };
    assert_user_enrollment_success(&setup, &[&second_setup_request], "user-mode enrollment");

    let environment_probe = format!(
        "if ($env:CLARK_SANDBOX_TEST_EXPLICIT -ne 'preserved') {{ exit 74 }}; \
         if ($env:USERPROFILE -ne {temp}) {{ exit 75 }}; \
         if ($env:LOCALAPPDATA -ne {temp}) {{ exit 76 }}; \
         if ($env:GIT_OPTIONAL_LOCKS -ne '0') {{ exit 77 }}",
        temp = ps_quote(&process_temp),
    );
    assert_success(
        "sanitized and explicit environment",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &environment_probe,
        ),
    );

    // Plan/read-only mode selects only the already-consented temp capability.
    // It must not need another setup transaction, and the old workspace ACL is
    // inert when its root capability is absent from the restricted token.
    let readonly_policy = WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: vec![process_temp.clone()],
        deny_read: Vec::new(),
        deny_write: Vec::new(),
        network: WireNetworkPolicy::Restricted,
        process_temp_root: Some(process_temp.clone()),
    };
    let readonly_escape = workspace.path().join("readonly-escape.txt");
    assert_failure(
        "read-only workspace write",
        &run_powershell(
            &runner,
            &state_dir,
            &readonly_policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&readonly_escape)
            ),
        ),
    );
    assert!(!readonly_escape.exists());
    let readonly_temp = process_temp.join("readonly-temp.txt");
    assert_success(
        "read-only private temp write",
        &run_powershell(
            &runner,
            &state_dir,
            &readonly_policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -NoNewline -Value inside",
                ps_quote(&readonly_temp)
            ),
        ),
    );

    let inside = workspace.path().join("inside.txt");
    assert_success(
        "inside write",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -NoNewline -Value inside",
                ps_quote(&inside)
            ),
        ),
    );
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "inside");

    let outside_file = outside.path().join("outside.txt");
    assert_failure(
        "outside write",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&outside_file)
            ),
        ),
    );
    assert!(!outside_file.exists());

    let git_config = dot_git.join("config");
    assert_failure(
        "protected git metadata",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&git_config)
            ),
        ),
    );
    assert!(!git_config.exists());

    let child_escape = outside.path().join("child.txt");
    let cmd = std::env::var_os("COMSPEC").map(PathBuf::from).unwrap();
    let output = run_process(
        &runner,
        &state_dir,
        &policy,
        workspace.path(),
        &cmd,
        &[
            "/D",
            "/Q",
            "/C",
            &format!("echo escaped>{}", child_escape.display()),
        ],
    );
    assert_failure("child process outside write", &output);
    assert!(!child_escape.exists());

    assert_success(
        "NUL device redirection",
        &run_process(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &cmd,
            &["/D", "/Q", "/C", "echo sandbox-ok>NUL"],
        ),
    );

    let junction = workspace.path().join("escape-junction");
    let junction_output = Command::new(&cmd)
        .args(["/D", "/Q", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert_success("junction fixture", &junction_output);
    let junction_escape = junction.join("junction.txt");
    assert_failure(
        "junction escape",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&junction_escape)
            ),
        ),
    );
    assert!(!outside.path().join("junction.txt").exists());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    assert_failure(
        "loopback network",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "$c=[Net.Sockets.TcpClient]::new(); try {{$c.Connect('127.0.0.1',{port}); exit 0}} catch {{exit 73}}"
            ),
        ),
    );
    drop(listener);

    assert_failure(
        "loopback ICMP",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            "$ok=Test-Connection -ComputerName 127.0.0.1 -Count 1 -Quiet; if ($ok) {exit 0} else {exit 73}",
        ),
    );

    let orphan = workspace.path().join("orphan.txt");
    let script = format!(
        "Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Milliseconds 800; Set-Content -LiteralPath {} -Value orphan'); exit 0",
        ps_quote(&orphan)
    );
    assert_success(
        "detached child launch",
        &run_powershell(&runner, &state_dir, &policy, workspace.path(), &script),
    );
    thread::sleep(Duration::from_millis(1200));
    assert!(
        !orphan.exists(),
        "kill-on-close job allowed an orphan process"
    );

    // Both projects were consented together, but each runtime policy carries
    // only its active root capability SIDs.
    let second_escape = second_workspace.path().join("first-policy-escape.txt");
    assert_failure(
        "first policy cannot use second project capability",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&second_escape)
            ),
        ),
    );
    assert!(!second_escape.exists());

    let stale_escape = workspace.path().join("stale-project.txt");
    assert_failure(
        "stale project capability",
        &run_powershell(
            &runner,
            &state_dir,
            &second_policy,
            second_workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -Value escaped",
                ps_quote(&stale_escape)
            ),
        ),
    );
    assert!(!stale_escape.exists());

    let second_inside = second_workspace.path().join("inside.txt");
    assert_success(
        "second project inside write",
        &run_powershell(
            &runner,
            &state_dir,
            &second_policy,
            second_workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -NoNewline -Value inside",
                ps_quote(&second_inside)
            ),
        ),
    );

    let first_again = workspace.path().join("first-project-still-active.txt");
    assert_success(
        "first project capability remains independently usable",
        &run_powershell(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            &format!(
                "Set-Content -LiteralPath {} -NoNewline -Value inside",
                ps_quote(&first_again)
            ),
        ),
    );
    eprintln!("clark_windows_core_containment=passed");

    // Git is an optional external integration, not one of the filesystem,
    // process-tree, or network-containment assertions above. Keep its stricter
    // compatibility contract in the dedicated Windows diagnostic lane without
    // allowing a third-party executable to suppress the core security receipt
    // or Windows installer publication.
    if let Some(git) = &git {
        let git_status = run_process(
            &runner,
            &state_dir,
            &policy,
            workspace.path(),
            git,
            &["status", "--short"],
        );
        if git_status.status.success() {
            eprintln!("clark_windows_git_compatibility=passed");
        } else if std::env::var_os("CLARK_WINDOWS_SANDBOX_GIT_REQUIRED").is_some() {
            eprintln!("clark_windows_git_compatibility=failed_required");
            assert_success(
                "Git safe.directory and noninteractive environment",
                &git_status,
            );
        } else {
            eprintln!(
                "clark_windows_git_compatibility=failed_optional\nstatus={:?}\nstdout={}\nstderr={}",
                git_status.status.code(),
                String::from_utf8_lossy(&git_status.stdout),
                String::from_utf8_lossy(&git_status.stderr),
            );
        }
    }
}
