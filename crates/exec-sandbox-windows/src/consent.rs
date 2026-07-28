use std::ffi::OsString;
use std::path::{Path, PathBuf};

use exec_sandbox_protocol::{decode_request, WindowsSetupRequest};

// Windows `SW_HIDE`. Keep the setup helper invisible; the UAC consent surface
// is intentionally the only window in this flow.
#[cfg(any(windows, test))]
const SETUP_WINDOW_VISIBILITY: i32 = 0;

/// Launch the product-owned setup helper through Windows' `runas` consent UI.
/// Proof files are always removed when the prompt is cancelled, the helper
/// fails, or setup succeeds.
pub fn run_setup_with_consent(
    program: &Path,
    args: &[OsString],
    cleanup_paths: Vec<PathBuf>,
) -> Result<(), String> {
    let _cleanup = CleanupPaths(cleanup_paths);
    let parameters = validate_args(args)?;
    native_runas(program, &parameters)
}

/// Execute one product-owned setup action. The first action crosses UAC once;
/// later actions mutate ACLs in the current user's own workspaces without
/// launching the elevated helper.
pub fn run_setup_action(
    program: &Path,
    args: &[OsString],
    requires_elevation: bool,
    cleanup_paths: Vec<PathBuf>,
) -> Result<(), String> {
    let _cleanup = CleanupPaths(cleanup_paths);
    let parameters = validate_args(args)?;
    if requires_elevation {
        return native_runas(program, &parameters);
    }
    for request in decode_requests(args)? {
        crate::native_enroll(request)?;
    }
    Ok(())
}

/// Retry an enrollment that Windows refused in user mode. This is reserved for
/// protected or administrator-owned roots and is never invoked implicitly by
/// provider startup.
pub fn run_elevated_enrollment(
    program: &Path,
    args: &[OsString],
    cleanup_paths: Vec<PathBuf>,
) -> Result<(), String> {
    let _cleanup = CleanupPaths(cleanup_paths);
    let parameters = format!("--enroll-only {}", validate_args(args)?);
    native_runas(program, &parameters)
}

fn decode_requests(args: &[OsString]) -> Result<Vec<WindowsSetupRequest>, String> {
    args.chunks_exact(2)
        .map(|pair| {
            let encoded = pair[1]
                .to_str()
                .ok_or_else(|| "Windows sandbox setup request is not Unicode".to_string())?;
            decode_request(encoded)
        })
        .collect()
}

fn validate_args(args: &[OsString]) -> Result<String, String> {
    const MAX_SETUP_REQUESTS: usize = 32;
    const MAX_PARAMETER_CHARS: usize = 30_000;
    if args.is_empty()
        || args.len() % 2 != 0
        || args.len() / 2 > MAX_SETUP_REQUESTS
        || args.chunks_exact(2).any(|pair| pair[0] != "--request-b64")
    {
        return Err("Windows sandbox setup action has an invalid argument shape".into());
    }
    let mut parameters = String::new();
    for pair in args.chunks_exact(2) {
        let encoded = pair[1]
            .to_str()
            .ok_or_else(|| "Windows sandbox setup request is not Unicode".to_string())?;
        if encoded.is_empty()
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Windows sandbox setup request is not URL-safe base64".into());
        }
        if !parameters.is_empty() {
            parameters.push(' ');
        }
        parameters.push_str("--request-b64 ");
        parameters.push_str(encoded);
    }
    if parameters.len() > MAX_PARAMETER_CHARS {
        return Err("Windows sandbox setup batch exceeds the safe command-line size".into());
    }
    Ok(parameters)
}

struct CleanupPaths(Vec<PathBuf>);

impl Drop for CleanupPaths {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
fn native_runas(program: &Path, parameters: &str) -> Result<(), String> {
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_FAILED};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let verb = wide("runas");
    let program = program
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let parameters = wide(parameters);
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = program.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    // UAC owns the only user-visible surface. The signed helper is a
    // short-lived console-subsystem executable, so showing it would flash a
    // second PowerShell/CMD-like window after consent.
    execute.nShow = SETUP_WINDOW_VISIBILITY;
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        return Err(format!(
            "Windows sandbox setup was cancelled or could not start: {}",
            std::io::Error::last_os_error()
        ));
    }
    if execute.hProcess.is_null() {
        return Err("Windows sandbox setup returned no process handle".into());
    }
    let wait = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    if wait == WAIT_FAILED {
        let error = std::io::Error::last_os_error();
        unsafe { CloseHandle(execute.hProcess) };
        return Err(format!("wait for Windows sandbox setup: {error}"));
    }
    let mut code = 0;
    let read = unsafe { GetExitCodeProcess(execute.hProcess, &mut code) };
    unsafe { CloseHandle(execute.hProcess) };
    if read == 0 {
        return Err(format!(
            "read Windows sandbox setup result: {}",
            std::io::Error::last_os_error()
        ));
    }
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Windows sandbox setup exited with code {code}"))
    }
}

#[cfg(not(windows))]
fn native_runas(_program: &Path, _parameters: &str) -> Result<(), String> {
    Err("elevated sandbox setup is only available on Windows".into())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_argument_injection_before_launch() {
        let args = [
            OsString::from("--request-b64"),
            OsString::from("valid;not-valid"),
        ];
        assert!(validate_args(&args).is_err());
    }

    #[test]
    fn renders_multiple_setup_requests_for_one_consent() {
        let args = [
            OsString::from("--request-b64"),
            OsString::from("Zmlyc3Q"),
            OsString::from("--request-b64"),
            OsString::from("c2Vjb25k"),
        ];
        assert_eq!(
            validate_args(&args).unwrap(),
            "--request-b64 Zmlyc3Q --request-b64 c2Vjb25k"
        );
    }

    #[test]
    fn elevated_setup_helper_is_hidden_behind_the_uac_surface() {
        assert_eq!(SETUP_WINDOW_VISIBILITY, 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn cleanup_runs_when_consent_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let proof = dir.path().join("proof");
        std::fs::write(&proof, b"proof").unwrap();
        let result = run_setup_with_consent(
            Path::new("setup.exe"),
            &[OsString::from("--request-b64"), OsString::from("YWJj")],
            vec![proof.clone()],
        );
        assert!(result.is_err());
        assert!(!proof.exists());
    }
}
