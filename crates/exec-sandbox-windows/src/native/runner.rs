use std::ffi::{OsStr, OsString};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::ptr;

use exec_sandbox_protocol::{decode_request, encode_request, WindowsRunnerRequest};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE, WAIT_FAILED};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, GetTokenInformation, IsTokenRestricted, TokenUser,
    DISABLE_MAX_PRIVILEGE, LUA_TOKEN, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    SID_AND_ATTRIBUTES, TOKEN_ALL_ACCESS, TOKEN_USER, WRITE_RESTRICTED,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, CreateDesktopW, HDESK};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessWithLogonW, GetCurrentProcess, GetExitCodeProcess,
    OpenProcessToken, ResumeThread, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::launch::LaunchHost;

use super::process::{
    command_line, inner_environment, wide_os, wide_str, worker_environment, TRACE_ENV,
    WORKER_REQUEST_ENV,
};
use super::transport::{ParentTransport, WorkerTransport};

const INFINITE: u32 = u32::MAX;
const WORKER_SWITCH: &str = "--restricted-worker";
const CHILD_CREATION_FLAGS: u32 = CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW;
const DESKTOP_ALL_ACCESS: u32 = 0x01ff;
// Git for Windows and other normal desktop CLIs consult HKCU during startup.
// `CreateProcessWithLogonW` leaves that hive unloaded by default; load only
// the offline worker's profile, then replace its child environment explicitly
// and create the actual command with the restricted token below.
const WORKER_LOGON_FLAGS: u32 = LOGON_WITH_PROFILE;

pub struct WindowsLaunchHost {
    state_dir: std::path::PathBuf,
    credential: Option<(String, String, String)>,
}

impl WindowsLaunchHost {
    pub fn new(state_dir: std::path::PathBuf) -> Self {
        Self {
            state_dir,
            credential: None,
        }
    }
}

impl LaunchHost for WindowsLaunchHost {
    fn verify_identity(&mut self, sid: &str) -> Result<(), String> {
        let credential = super::identity::load_offline_password(&self.state_dir)?;
        if !credential.2.eq_ignore_ascii_case(sid) {
            return Err("Windows sandbox credential SID does not match setup attestation".into());
        }
        self.credential = Some(credential);
        Ok(())
    }

    fn verify_network_denied(&mut self, sid: &str) -> Result<(), String> {
        super::firewall::verify_network_denied(sid)
    }

    fn spawn_restricted(
        &mut self,
        request: &WindowsRunnerRequest,
        sid: &str,
    ) -> Result<i32, String> {
        let (username, password, _) = self
            .credential
            .take()
            .ok_or_else(|| "Windows sandbox credential was not verified".to_string())?;
        spawn_worker(request, sid, &username, &password)
    }
}

pub fn is_worker_request(args: &[OsString]) -> bool {
    args.iter().any(|argument| argument == WORKER_SWITCH)
}

pub fn worker_request_from_environment() -> Result<WindowsRunnerRequest, String> {
    let encoded = std::env::var_os(WORKER_REQUEST_ENV)
        .ok_or_else(|| format!("missing {WORKER_REQUEST_ENV}"))?;
    let encoded = encoded
        .to_str()
        .ok_or_else(|| format!("{WORKER_REQUEST_ENV} is not Unicode"))?;
    decode_request(encoded)
}

fn spawn_worker(
    request: &WindowsRunnerRequest,
    offline_sid: &str,
    username: &str,
    password: &str,
) -> Result<i32, String> {
    trace("spawn_worker:begin");
    let process_cwd = request.process.cwd.to_os_string();
    super::acl::grant_runtime_cwd_read(Path::new(&process_cwd), offline_sid)?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve Windows sandbox runner: {error}"))?;
    let encoded = encode_request(request)?;
    let transport = ParentTransport::create(offline_sid)?;
    // CreateProcessWithLogonW accepts a substantially shorter command line
    // than CreateProcessW. Keep the request in the private worker environment
    // and reserve argv for short transport endpoint names. The inner command
    // receives a separately constructed environment, so this value is not
    // propagated into model-authored child processes.
    let mut args = vec![OsString::from(WORKER_SWITCH)];
    args.extend(transport.worker_args());
    let command_line = command_line(&executable, &args);
    let environment = worker_environment(&encoded);
    // The orchestration worker is logged on as the offline identity before its
    // restricted token exists. Do not start that worker in the caller's cwd:
    // hosted-runner temporary directories can be private to the interactive
    // account, causing CreateProcessWithLogonW to reject an otherwise valid
    // request before the sandboxed command starts. The setup state directory
    // has an explicit traversal grant for the offline identity. Only the inner,
    // restricted child receives the requested cwd below.
    let cwd = request.state_dir.as_os_str();
    let mut process = unsafe {
        create_with_logon(
            username,
            password,
            &executable,
            command_line,
            &cwd,
            &environment,
        )?
    };
    trace(&format!("spawn_worker:created:pid={}", process.dwProcessId));
    let job = OwnedHandle::new(
        unsafe { CreateJobObjectW(ptr::null(), ptr::null()) },
        "CreateJobObjectW",
    )?;
    configure_kill_job(job.0)?;
    if unsafe { AssignProcessToJobObject(job.0, process.hProcess) } == 0 {
        unsafe { terminate_process_info(&mut process) };
        return Err(last_error("AssignProcessToJobObject"));
    }
    if unsafe { ResumeThread(process.hThread) } == u32::MAX {
        unsafe { terminate_process_info(&mut process) };
        return Err(last_error("ResumeThread"));
    }
    trace("spawn_worker:resumed");
    let drain = transport.begin_drain();
    trace("spawn_worker:wait_worker");
    let process_result = wait_process(process);
    trace("spawn_worker:worker_exited");
    // The job is the process-tree lifetime boundary, while the transport is
    // only the root command's output boundary. A descendant may inherit a pipe
    // writer and outlive the root (Git for Windows can do this even for a
    // read-only status). Waiting for pipe EOF while the kill-on-close job is
    // still alive deadlocks the caller: the descendant keeps the pipe open and
    // the job is not closed until after the drain. Close the job first so every
    // surviving descendant is terminated, then drain the already-written bytes.
    drop(job);
    trace("spawn_worker:job_closed");
    let drain_result = drain.finish();
    trace("spawn_worker:drain_finished");
    match (process_result, drain_result) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

pub fn run_restricted_worker(
    request: &WindowsRunnerRequest,
    expected_sid: &str,
    transport: &WorkerTransport,
) -> Result<i32, String> {
    transport.write_trace("restricted_worker:begin");
    let base_token = current_process_token()?;
    let actual_sid = token_user_sid(base_token.0)?;
    if !actual_sid.eq_ignore_ascii_case(expected_sid) {
        return Err("restricted worker is not running as the attested offline identity".into());
    }
    // `launch` already verifies the offline identity's firewall rules before
    // this worker is created. Reopening the Firewall COM server from the
    // profile-less, noninteractive offline account can block indefinitely on
    // hosted Windows, while adding no protection against a post-verification
    // change (that would require administrator authority). Keep the check at
    // the parent boundary and proceed directly to the WRITE_RESTRICTED token.
    let capability_sids = request.policy.write_capability_sids();
    super::acl::grant_current_window_station_access(&capability_sids)?;
    let restricted = restricted_write_token(base_token.0, &capability_sids)?;
    if unsafe { IsTokenRestricted(restricted.0) } == 0 {
        return Err("CreateRestrictedToken returned an unrestricted token".into());
    }
    transport.write_trace("restricted_worker:token_ready");
    let desktop = PrivateDesktop::create(expected_sid, &capability_sids)?;
    transport.write_trace("restricted_worker:desktop_ready");
    spawn_inner(request, restricted.0, transport, &desktop)
}

fn spawn_inner(
    request: &WindowsRunnerRequest,
    token: HANDLE,
    transport: &WorkerTransport,
    desktop: &PrivateDesktop,
) -> Result<i32, String> {
    transport.write_trace("inner:begin");
    let process = &request.process;
    let program = process.program.to_os_string();
    let args = git_args_without_optional_locks(
        Path::new(&program),
        process
            .args
            .iter()
            .map(|argument| argument.to_os_string())
            .collect(),
    );
    let mut command_line = command_line(Path::new(&program), &args);
    let mut program_w = wide_os(&program);
    let cwd = process.cwd.to_os_string();
    let cwd_w = wide_os(&cwd);
    let mut environment = inner_environment(request);
    let startup = transport.startup_info(desktop.path());
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessAsUserW(
            token,
            program_w.as_mut_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CHILD_CREATION_FLAGS,
            environment.as_mut_ptr().cast(),
            cwd_w.as_ptr(),
            &startup,
            &mut info,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessAsUserW"));
    }
    transport.write_trace(&format!("inner:created:pid={}", info.dwProcessId));
    let result = wait_process(info);
    transport.write_trace("inner:exited");
    result
}

struct PrivateDesktop {
    desktop: HDESK,
    path: Vec<u16>,
}

impl PrivateDesktop {
    fn create(user_sid: &str, capability_sids: &[String]) -> Result<Self, String> {
        let security = DesktopSecurity::new(user_sid, capability_sids)?;
        let desktop_name = format!("AgentSandbox-{}", std::process::id());
        let desktop_w = wide_str(&desktop_name);
        let desktop = unsafe {
            CreateDesktopW(
                desktop_w.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                DESKTOP_ALL_ACCESS,
                &security.attributes,
            )
        };
        if desktop.is_null() {
            return Err(last_error("CreateDesktopW"));
        }
        Ok(Self {
            desktop,
            // A desktop name without a backslash resolves inside the worker's
            // existing noninteractive window station.
            path: desktop_w,
        })
    }

    fn path(&self) -> &[u16] {
        &self.path
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        unsafe { CloseDesktop(self.desktop) };
    }
}

struct DesktopSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl DesktopSecurity {
    fn new(user_sid: &str, capability_sids: &[String]) -> Result<Self, String> {
        let mut sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{user_sid})");
        for sid in capability_sids {
            sddl.push_str(&format!("(A;;GA;;;{sid})"));
        }
        let sddl = wide_str(&sddl);
        let mut descriptor = ptr::null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(last_error(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW(desktop)",
            ));
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }
}

impl Drop for DesktopSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe { LocalFree(self.descriptor.cast()) };
        }
    }
}

fn trace(message: &str) {
    if std::env::var_os(TRACE_ENV).is_some() {
        eprintln!(
            "agent Windows sandbox trace: {message}; pid={}",
            std::process::id()
        );
    }
}

/// Git status normally refreshes the index as a side effect.  That refresh
/// needs an index lock, while Clark Code intentionally denies writes below `.git`
/// for restricted children.  `GIT_OPTIONAL_LOCKS=0` is a useful fallback for
/// shell-launched Git, but direct Git children need the command-line contract
/// too: Git documents this exact global option for background status calls.
fn git_args_without_optional_locks(program: &Path, mut args: Vec<OsString>) -> Vec<OsString> {
    let is_git = program
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("git") || name.eq_ignore_ascii_case("git.exe")
        });
    if is_git
        && !args
            .iter()
            .any(|argument| argument == "--no-optional-locks")
    {
        args.insert(0, OsString::from("--no-optional-locks"));
    }
    args
}

unsafe fn create_with_logon(
    username: &str,
    password: &str,
    executable: &Path,
    mut command_line: Vec<u16>,
    cwd: &OsStr,
    environment: &[u16],
) -> Result<PROCESS_INFORMATION, String> {
    let username = wide_str(username);
    let domain = wide_str(".");
    let password = wide_str(password);
    let mut executable = wide_os(executable.as_os_str());
    let cwd = wide_os(cwd);
    let mut startup: STARTUPINFOW = zeroed();
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let mut info: PROCESS_INFORMATION = zeroed();
    let created = CreateProcessWithLogonW(
        username.as_ptr(),
        domain.as_ptr(),
        password.as_ptr(),
        WORKER_LOGON_FLAGS,
        executable.as_mut_ptr(),
        command_line.as_mut_ptr(),
        CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED | CREATE_NO_WINDOW,
        environment.as_ptr().cast(),
        cwd.as_ptr(),
        &startup,
        &mut info,
    );
    if created == 0 {
        Err(last_error("CreateProcessWithLogonW"))
    } else {
        Ok(info)
    }
}

fn current_process_token() -> Result<OwnedHandle, String> {
    let mut token = ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token) };
    if opened == 0 {
        Err(last_error("OpenProcessToken"))
    } else {
        OwnedHandle::new(token, "OpenProcessToken")
    }
}

fn token_user_sid(token: HANDLE) -> Result<String, String> {
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    if size == 0 {
        return Err(last_error("GetTokenInformation(size)"));
    }
    let mut token_user = vec![0_u8; size as usize];
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            token_user.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    };
    if read == 0 {
        return Err(last_error("GetTokenInformation(TokenUser)"));
    }
    let user = unsafe { &*(token_user.as_ptr() as *const TOKEN_USER) };
    sid_to_string(user.User.Sid)
}

fn restricted_write_token(
    base_token: HANDLE,
    capability_sids: &[String],
) -> Result<OwnedHandle, String> {
    if capability_sids.is_empty() {
        return Err("Windows sandbox has no restricting capability SID".into());
    }
    let capabilities = capability_sids
        .iter()
        .map(|sid| LocalSid::from_string(sid))
        .collect::<Result<Vec<_>, _>>()?;
    let mut entries = capabilities
        .iter()
        .map(|sid| SID_AND_ATTRIBUTES {
            Sid: sid.0,
            Attributes: 0,
        })
        .collect::<Vec<_>>();
    let mut restricted = ptr::null_mut();
    let created = unsafe {
        CreateRestrictedToken(
            base_token,
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
            0,
            ptr::null(),
            0,
            ptr::null(),
            entries.len() as u32,
            entries.as_mut_ptr(),
            &mut restricted,
        )
    };
    if created == 0 {
        Err(last_error("CreateRestrictedToken"))
    } else {
        OwnedHandle::new(restricted, "CreateRestrictedToken")
    }
}

struct LocalSid(*mut std::ffi::c_void);

impl LocalSid {
    fn from_string(value: &str) -> Result<Self, String> {
        let value = wide_str(value);
        let mut sid = ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(value.as_ptr(), &mut sid) } == 0 {
            Err(last_error("ConvertStringSidToSidW(capability)"))
        } else {
            Ok(Self(sid))
        }
    }
}

impl Drop for LocalSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::LocalFree(self.0.cast()) };
        }
    }
}

fn sid_to_string(sid: *mut std::ffi::c_void) -> Result<String, String> {
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(last_error("ConvertSidToStringSidW"));
    }
    let mut length = 0;
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    let value = unsafe { OsString::from_wide(std::slice::from_raw_parts(text, length)) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(text.cast());
    }
    Ok(value)
}

fn configure_kill_job(job: HANDLE) -> Result<(), String> {
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        Err(last_error("SetInformationJobObject"))
    } else {
        Ok(())
    }
}

fn wait_process(info: PROCESS_INFORMATION) -> Result<i32, String> {
    let process = OwnedHandle::new(info.hProcess, "process handle")?;
    let _thread = OwnedHandle::new(info.hThread, "thread handle")?;
    if unsafe { WaitForSingleObject(process.0, INFINITE) } == WAIT_FAILED {
        return Err(last_error("WaitForSingleObject"));
    }
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0 {
        return Err(last_error("GetExitCodeProcess"));
    }
    Ok(exit_code as i32)
}

unsafe fn terminate_process_info(info: &mut PROCESS_INFORMATION) {
    if !info.hProcess.is_null() {
        windows_sys::Win32::System::Threading::TerminateProcess(info.hProcess, 1);
        CloseHandle(info.hProcess);
        info.hProcess = ptr::null_mut();
    }
    if !info.hThread.is_null() {
        CloseHandle(info.hThread);
        info.hThread = ptr::null_mut();
    }
}

fn last_error(operation: &str) -> String {
    format!(
        "{operation} failed: {} (code {})",
        std::io::Error::last_os_error(),
        unsafe { GetLastError() }
    )
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE, operation: &str) -> Result<Self, String> {
        if handle.is_null() {
            Err(last_error(operation))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_children_never_request_a_visible_console() {
        assert_ne!(CHILD_CREATION_FLAGS & CREATE_NO_WINDOW, 0);
        assert_ne!(CHILD_CREATION_FLAGS & CREATE_UNICODE_ENVIRONMENT, 0);
    }

    #[test]
    fn worker_loads_its_offline_profile_before_starting_restricted_children() {
        assert_eq!(WORKER_LOGON_FLAGS & LOGON_WITH_PROFILE, LOGON_WITH_PROFILE);
    }

    #[test]
    fn direct_git_commands_disable_optional_index_locks() {
        assert_eq!(
            git_args_without_optional_locks(
                Path::new(r"C:\\Program Files\\Git\\bin\\git.exe"),
                vec![OsString::from("status"), OsString::from("--short")],
            ),
            vec![
                OsString::from("--no-optional-locks"),
                OsString::from("status"),
                OsString::from("--short"),
            ],
        );
    }

    #[test]
    fn git_lock_override_is_not_duplicated() {
        assert_eq!(
            git_args_without_optional_locks(
                Path::new("git"),
                vec![
                    OsString::from("--no-optional-locks"),
                    OsString::from("status")
                ],
            ),
            vec![
                OsString::from("--no-optional-locks"),
                OsString::from("status")
            ],
        );
    }
}
