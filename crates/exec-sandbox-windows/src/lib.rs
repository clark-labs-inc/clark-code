//! Windows-only privilege-boundary executables.
//!
//! The protocol and setup attestation are production contracts. Native token,
//! ACL, credential, firewall, and process-tree enforcement live behind narrow
//! host traits so ordering can be simulated on every platform and exercised at
//! the real boundary by the required Windows CI suite.

mod consent;
mod launch;
#[cfg(windows)]
mod native;
#[cfg_attr(not(windows), allow(dead_code))]
mod ownership;
mod provision;
#[cfg(any(windows, test))]
mod state;

pub use consent::{run_elevated_enrollment, run_setup_action, run_setup_with_consent};
pub use launch::{launch, LaunchHost};
pub use provision::{enroll, provision, EnrollmentHost, ProvisionedIdentity, ProvisioningHost};

use std::ffi::OsString;

use exec_sandbox_protocol::{
    decode_request, read_setup_marker, WindowsRunnerRequest, WindowsSetupRequest,
    EXIT_CONTAINMENT_FAILED, EXIT_INVALID_REQUEST, EXIT_SETUP_REQUIRED, RUNNER_PROTOCOL_VERSION,
    SETUP_PROTOCOL_VERSION,
};

pub fn runner_main(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--self-test") {
        print_self_test("runner", RUNNER_PROTOCOL_VERSION);
        return 0;
    }
    if native_worker_requested(&args) {
        return native_worker_main(&args);
    }
    let request = match request_argument::<WindowsRunnerRequest>(&args) {
        Ok(request) => request,
        Err(error) => return fail(EXIT_INVALID_REQUEST, error),
    };
    let marker = match validate_runner_request(&request) {
        Ok(marker) => marker,
        Err((code, error)) => return fail(code, error),
    };
    native_runner(request, marker)
}

pub fn setup_main(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--self-test") {
        print_self_test("setup", SETUP_PROTOCOL_VERSION);
        return 0;
    }
    let requests = match request_arguments::<WindowsSetupRequest>(&args) {
        Ok(requests) => requests,
        Err(error) => return fail(EXIT_INVALID_REQUEST, error),
    };
    for request in &requests {
        if let Err(error) = request.validate() {
            return fail(EXIT_INVALID_REQUEST, error);
        }
    }
    if !is_elevated() {
        return fail(
            EXIT_SETUP_REQUIRED,
            "Windows sandbox setup requires explicit administrator consent".to_string(),
        );
    }

    let enroll_only = args.iter().any(|argument| argument == "--enroll-only");
    if !enroll_only {
        let code = native_setup(requests[0].clone());
        if code != 0 {
            return code;
        }
    }
    for request in requests {
        let code = native_enroll_code(request);
        if code != 0 {
            return code;
        }
    }
    0
}

fn request_argument<T: for<'de> serde::Deserialize<'de>>(args: &[OsString]) -> Result<T, String> {
    let mut requests = request_arguments(args)?;
    if requests.len() != 1 {
        return Err("runner accepts exactly one sandbox request".to_string());
    }
    Ok(requests.remove(0))
}

fn request_arguments<T: for<'de> serde::Deserialize<'de>>(
    args: &[OsString],
) -> Result<Vec<T>, String> {
    const MAX_SETUP_BATCH: usize = 32;
    let positions = args
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == "--request-b64").then_some(index))
        .collect::<Vec<_>>();
    if positions.is_empty() {
        return Err("missing --request-b64".to_string());
    }
    if positions.len() > MAX_SETUP_BATCH {
        return Err(format!(
            "sandbox setup batch exceeds {MAX_SETUP_BATCH} requests"
        ));
    }
    positions
        .into_iter()
        .map(|position| {
            let encoded = args
                .get(position + 1)
                .ok_or_else(|| "missing encoded helper request".to_string())?
                .to_str()
                .ok_or_else(|| "encoded helper request is not Unicode".to_string())?;
            decode_request(encoded)
        })
        .collect()
}

fn print_self_test(kind: &str, protocol_version: u32) {
    let status = if cfg!(windows) {
        "native_boundary_ready_setup_required"
    } else {
        "protocol_only_non_windows_host"
    };
    println!(
        "{}",
        serde_json::json!({
            "helper": kind,
            "protocol_version": protocol_version,
            "platform": std::env::consts::OS,
            "status": status
        })
    );
}

fn fail(code: i32, message: String) -> i32 {
    eprintln!("clark Windows sandbox: {message}");
    code
}

#[cfg(windows)]
fn is_elevated() -> bool {
    use std::ffi::c_void;
    use std::ptr;
    use windows_sys::Win32::Security::{
        AllocateAndInitializeSid, CheckTokenMembership, FreeSid, SECURITY_NT_AUTHORITY,
    };

    const SECURITY_BUILTIN_DOMAIN_RID: u32 = 0x20;
    const DOMAIN_ALIAS_RID_ADMINS: u32 = 0x220;

    unsafe {
        let mut administrators: *mut c_void = ptr::null_mut();
        let allocated = AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            2,
            SECURITY_BUILTIN_DOMAIN_RID,
            DOMAIN_ALIAS_RID_ADMINS,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut administrators,
        );
        if allocated == 0 {
            return false;
        }
        let mut member = 0;
        let checked = CheckTokenMembership(ptr::null_mut(), administrators, &mut member);
        FreeSid(administrators);
        checked != 0 && member != 0
    }
}

#[cfg(not(windows))]
fn is_elevated() -> bool {
    false
}

#[cfg(windows)]
fn native_runner(
    request: WindowsRunnerRequest,
    marker: exec_sandbox_protocol::WindowsSetupMarker,
) -> i32 {
    let mut host = native::WindowsLaunchHost::new(request.state_dir.clone());
    match launch(&request, &marker, &mut host) {
        Ok(code) => code,
        Err(error) => fail(EXIT_CONTAINMENT_FAILED, error),
    }
}

#[cfg(not(windows))]
fn native_runner(
    _request: WindowsRunnerRequest,
    _marker: exec_sandbox_protocol::WindowsSetupMarker,
) -> i32 {
    fail(
        EXIT_CONTAINMENT_FAILED,
        "native restricted-token runner is not installed in this build".to_string(),
    )
}

fn validate_runner_request(
    request: &WindowsRunnerRequest,
) -> Result<exec_sandbox_protocol::WindowsSetupMarker, (i32, String)> {
    request
        .validate()
        .map_err(|error| (EXIT_INVALID_REQUEST, error))?;
    let marker =
        read_setup_marker(&request.state_dir).map_err(|error| (EXIT_SETUP_REQUIRED, error))?;
    let current_exe = std::env::current_exe().map_err(|error| {
        (
            EXIT_SETUP_REQUIRED,
            format!("resolve Windows sandbox runner: {error}"),
        )
    })?;
    marker
        .validate_for_runner(&current_exe)
        .map_err(|error| (EXIT_SETUP_REQUIRED, error))?;
    marker
        .validate_for_policy(&request.policy)
        .map_err(|error| (EXIT_SETUP_REQUIRED, error))?;
    Ok(marker)
}

#[cfg(windows)]
fn native_worker_main(args: &[OsString]) -> i32 {
    // Connect before decoding or validating anything. That guarantees every
    // fail-closed worker path closes both endpoints and reaches the caller.
    let transport = match native::WorkerTransport::connect(args) {
        Ok(transport) => transport,
        Err(error) => return fail(EXIT_CONTAINMENT_FAILED, error),
    };
    let request = match native::worker_request_from_environment() {
        Ok(request) => request,
        Err(error) => return worker_fail(&transport, EXIT_INVALID_REQUEST, error),
    };
    let marker = match validate_runner_request(&request) {
        Ok(marker) => marker,
        Err((code, error)) => return worker_fail(&transport, code, error),
    };
    match native::run_restricted_worker(&request, &marker.offline_identity_sid, &transport) {
        Ok(code) => code,
        Err(error) => worker_fail(&transport, EXIT_CONTAINMENT_FAILED, error),
    }
}

#[cfg(not(windows))]
fn native_worker_main(_args: &[OsString]) -> i32 {
    fail(
        EXIT_CONTAINMENT_FAILED,
        "native restricted-token worker is not installed in this build".to_string(),
    )
}

#[cfg(windows)]
fn worker_fail(transport: &native::WorkerTransport, code: i32, error: String) -> i32 {
    transport.write_failure(&error);
    code
}

#[cfg(windows)]
fn native_worker_requested(args: &[OsString]) -> bool {
    native::is_worker_request(args)
}

#[cfg(not(windows))]
fn native_worker_requested(_args: &[OsString]) -> bool {
    false
}

#[cfg(windows)]
fn native_setup(request: WindowsSetupRequest) -> i32 {
    let setup_helper = match current_setup_helper() {
        Ok(path) => path,
        Err(error) => return fail(EXIT_CONTAINMENT_FAILED, error),
    };
    let mut host = native::WindowsProvisioningHost::new(request.state_dir.clone(), setup_helper);
    let result = provision(&request, &mut host);
    match result {
        Ok(_) => 0,
        Err(error) => fail(EXIT_CONTAINMENT_FAILED, error),
    }
}

#[cfg(windows)]
fn native_enroll(
    request: WindowsSetupRequest,
    setup_helper: &std::path::Path,
) -> Result<(), String> {
    static ENROLLMENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENROLLMENT_LOCK
        .lock()
        .map_err(|_| "Windows sandbox enrollment lock is poisoned".to_string())?;
    let mut host =
        native::WindowsProvisioningHost::new(request.state_dir.clone(), setup_helper.to_path_buf());
    enroll(&request, &mut host).map(|_| ())
}

#[cfg(not(windows))]
fn native_enroll(
    _request: WindowsSetupRequest,
    _setup_helper: &std::path::Path,
) -> Result<(), String> {
    Err("native Windows workspace enrollment is not available on this host".to_string())
}

fn native_enroll_code(request: WindowsSetupRequest) -> i32 {
    let setup_helper = match current_setup_helper() {
        Ok(path) => path,
        Err(error) => return fail(EXIT_CONTAINMENT_FAILED, error),
    };
    match native_enroll(request, &setup_helper) {
        Ok(()) => 0,
        Err(error) => fail(EXIT_CONTAINMENT_FAILED, error),
    }
}

#[cfg(windows)]
fn current_setup_helper() -> Result<std::path::PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("resolve Windows sandbox setup executable: {error}"))
}

#[cfg(not(windows))]
fn current_setup_helper() -> Result<std::path::PathBuf, String> {
    Err("native Windows sandbox setup is not available on this host".into())
}

#[cfg(not(windows))]
fn native_setup(_request: WindowsSetupRequest) -> i32 {
    fail(
        EXIT_CONTAINMENT_FAILED,
        "native identity, ACL, and WFP setup is not installed in this build".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_report_protocol_readiness_without_mutating_host_state() {
        assert_eq!(runner_main([OsString::from("--self-test")]), 0);
        assert_eq!(setup_main([OsString::from("--self-test")]), 0);
    }

    #[test]
    fn malformed_requests_fail_closed() {
        assert_eq!(runner_main(Vec::<OsString>::new()), EXIT_INVALID_REQUEST);
        assert_eq!(setup_main(Vec::<OsString>::new()), EXIT_INVALID_REQUEST);
    }

    #[test]
    fn setup_request_batches_preserve_order() {
        let first = exec_sandbox_protocol::encode_request(&"first").unwrap();
        let second = exec_sandbox_protocol::encode_request(&"second").unwrap();
        let decoded = request_arguments::<String>(&[
            OsString::from("--request-b64"),
            OsString::from(first),
            OsString::from("--request-b64"),
            OsString::from(second),
        ])
        .unwrap();
        assert_eq!(decoded, ["first", "second"]);
    }
}
