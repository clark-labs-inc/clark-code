use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use exec_sandbox_protocol::{
    encode_request, WindowsRootProof, WindowsRunnerRequest, WindowsSetupRequest, WireOsString,
    WireProcess, WireSandboxPolicy, RUNNER_PROTOCOL_VERSION,
};

pub fn run_powershell(
    runner: &Path,
    state_dir: &Path,
    policy: &WireSandboxPolicy,
    cwd: &Path,
    script: &str,
) -> Output {
    let powershell = PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    run_process(
        runner,
        state_dir,
        policy,
        cwd,
        &powershell,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
}

pub fn run_process(
    runner: &Path,
    state_dir: &Path,
    policy: &WireSandboxPolicy,
    cwd: &Path,
    program: &Path,
    args: &[&str],
) -> Output {
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
    const TIMEOUT_MARKER: &str = "clark Windows sandbox test command timed out";
    let request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: format!("native-run-{}", std::process::id()),
        state_dir: state_dir.to_path_buf(),
        policy: policy.clone(),
        process: WireProcess {
            program: WireOsString::from_os(program.as_os_str()),
            args: args
                .iter()
                .map(|argument| WireOsString::from_os(OsStr::new(argument)))
                .collect(),
            cwd: WireOsString::from_os(cwd.as_os_str()),
            env: vec![
                (
                    WireOsString::from_os(OsStr::new("TMP")),
                    WireOsString::from_os(policy.process_temp_root.as_ref().unwrap().as_os_str()),
                ),
                (
                    WireOsString::from_os(OsStr::new("TEMP")),
                    WireOsString::from_os(policy.process_temp_root.as_ref().unwrap().as_os_str()),
                ),
                (
                    WireOsString::from_os(OsStr::new("CLARK_SANDBOX_TEST_EXPLICIT")),
                    WireOsString::from_os(OsStr::new("preserved")),
                ),
            ],
        },
    };
    let mut child = Command::new(runner)
        .args(["--request-b64", &encode_request(&request).unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() >= COMMAND_TIMEOUT {
            child.kill().unwrap();
            let mut output = child.wait_with_output().unwrap();
            output.stderr.extend_from_slice(
                format!(
                    "{TIMEOUT_MARKER} after {}s: runner={} program={} cwd={}\\n",
                    COMMAND_TIMEOUT.as_secs(),
                    runner.display(),
                    program.display(),
                    cwd.display(),
                )
                .as_bytes(),
            );
            return output;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn required_helper(name: &str) -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
    assert!(path.is_file(), "missing helper {}", path.display());
    path.canonicalize().unwrap()
}

pub fn find_program(name: &str) -> Option<PathBuf> {
    let output = Command::new("where.exe").arg(name).output().unwrap();
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)?;
    assert!(path.is_file(), "missing program {}", path.display());
    Some(path)
}

fn setup_args(requests: &[&WindowsSetupRequest]) -> Vec<OsString> {
    requests
        .iter()
        .flat_map(|request| {
            [
                OsString::from("--request-b64"),
                OsString::from(encode_request(*request).unwrap()),
            ]
        })
        .collect()
}

fn cleanup_paths(requests: &[&WindowsSetupRequest]) -> Vec<PathBuf> {
    requests
        .iter()
        .flat_map(|request| request.root_proofs.iter())
        .map(|proof| proof.proof_path.clone())
        .collect()
}

pub fn assert_initial_setup_success(setup: &Path, requests: &[&WindowsSetupRequest], label: &str) {
    let args = setup_args(requests);
    if std::env::var_os("CLARK_WINDOWS_SANDBOX_E2E_USE_CONSENT").is_some() {
        exec_sandbox_windows::run_setup_action(setup, &args, true, cleanup_paths(requests))
            .unwrap_or_else(|error| panic!("{label} failed: {error}"));
    } else {
        let output = Command::new(setup).args(&args).output().unwrap();
        assert_success(label, &output);
    }
}

pub fn assert_user_enrollment_success(
    setup: &Path,
    requests: &[&WindowsSetupRequest],
    label: &str,
) {
    exec_sandbox_windows::run_setup_action(
        setup,
        &setup_args(requests),
        false,
        cleanup_paths(requests),
    )
    .unwrap_or_else(|error| panic!("{label} failed without elevation: {error}"));
}

pub fn proof_roots(policy: &WireSandboxPolicy, request_id: &str) -> Vec<WindowsRootProof> {
    policy
        .write_roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            std::fs::create_dir_all(root).unwrap();
            let nonce = format!("{:032x}", index + 1);
            let proof_path = root.join(format!(".clark-sandbox-setup-{request_id}-{index}.proof"));
            std::fs::write(&proof_path, &nonce).unwrap();
            WindowsRootProof {
                root: root.clone(),
                proof_path,
                nonce,
            }
        })
        .collect()
}

pub fn ps_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

pub fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure(label: &str, output: &Output) {
    assert!(
        !output.status.success()
            && !String::from_utf8_lossy(&output.stderr)
                .contains("clark Windows sandbox test command timed out"),
        "{label} unexpectedly succeeded\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
