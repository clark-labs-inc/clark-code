use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, ERROR_BROKEN_PIPE,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{STARTF_USESTDHANDLES, STARTUPINFOW};

const STDOUT_PIPE_SWITCH: &str = "--stdout-pipe";
const STDERR_PIPE_SWITCH: &str = "--stderr-pipe";
const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
static PIPE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Parent half of Clark's process transport. The offline worker receives only
/// unguessable pipe names, never ambient desktop-process handles.
pub struct ParentTransport {
    stdout: ServerPipe,
    stderr: ServerPipe,
}

impl ParentTransport {
    pub fn create(offline_sid: &str) -> Result<Self, String> {
        let canonical_sid = canonical_sid(offline_sid)?;
        let security = PipeSecurity::for_sid(&canonical_sid)?;
        let sequence = PIPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let prefix = format!(r"\\.\pipe\clark-sandbox-{}-{sequence}", std::process::id());
        Ok(Self {
            stdout: ServerPipe::create(format!("{prefix}-stdout"), &security)?,
            stderr: ServerPipe::create(format!("{prefix}-stderr"), &security)?,
        })
    }

    pub fn worker_args(&self) -> [OsString; 4] {
        [
            OsString::from(STDOUT_PIPE_SWITCH),
            OsString::from(&self.stdout.name),
            OsString::from(STDERR_PIPE_SWITCH),
            OsString::from(&self.stderr.name),
        ]
    }

    pub fn begin_drain(self) -> TransportDrain {
        TransportDrain {
            stdout: self.stdout.drain(OutputChannel::Stdout),
            stderr: self.stderr.drain(OutputChannel::Stderr),
        }
    }
}

pub struct TransportDrain {
    stdout: JoinHandle<Result<(), String>>,
    stderr: JoinHandle<Result<(), String>>,
}

impl TransportDrain {
    pub fn finish(self) -> Result<(), String> {
        join_drain(self.stdout, "stdout")?;
        join_drain(self.stderr, "stderr")
    }
}

/// Worker half of the transport. These are the only inheritable handles passed
/// to the command launched with the restricted token.
pub struct WorkerTransport {
    stdin: OwnedHandle,
    stdout: OwnedHandle,
    stderr: OwnedHandle,
}

impl WorkerTransport {
    pub fn connect(args: &[OsString]) -> Result<Self, String> {
        let stdout_name = switch_value(args, STDOUT_PIPE_SWITCH)?;
        let stderr_name = switch_value(args, STDERR_PIPE_SWITCH)?;
        let stdout = open_pipe_writer(&stdout_name)?;
        let stderr = open_pipe_writer(&stderr_name)?;
        let stdin = open_null_reader()?;
        Ok(Self {
            stdin,
            stdout,
            stderr,
        })
    }

    pub fn startup_info(&self) -> STARTUPINFOW {
        let mut startup: STARTUPINFOW = unsafe { zeroed() };
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        // These children are created with CREATE_NO_WINDOW and must stay on
        // CreateProcessAsUserW's noninteractive window station. Requesting
        // `winsta0\default` requires explicit window-station and desktop DACL
        // grants for the restricted logon session; without them, console
        // applications can remain blocked in their conhost LPC handshake
        // before their runtime initializes.
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = self.stdin.0;
        startup.hStdOutput = self.stdout.0;
        startup.hStdError = self.stderr.0;
        startup
    }

    pub fn write_failure(&self, message: &str) {
        let rendered = format!("clark Windows sandbox: {message}\r\n");
        let _ = write_handle(self.stderr.0, rendered.as_bytes());
    }

    pub fn write_trace(&self, message: &str) {
        if std::env::var_os(super::process::TRACE_ENV).is_some() {
            let rendered = format!(
                "clark Windows sandbox trace: {message}; pid={}\r\n",
                std::process::id()
            );
            let _ = write_handle(self.stderr.0, rendered.as_bytes());
        }
    }
}

struct ServerPipe {
    name: String,
    handle: OwnedHandle,
}

impl ServerPipe {
    fn create(name: String, security: &PipeSecurity) -> Result<Self, String> {
        let name_w = wide(OsStr::new(&name));
        let handle = unsafe {
            CreateNamedPipeW(
                name_w.as_ptr(),
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                0,
                &security.attributes,
            )
        };
        Ok(Self {
            name,
            handle: OwnedHandle::new(handle, "CreateNamedPipeW")?,
        })
    }

    fn drain(self, channel: OutputChannel) -> JoinHandle<Result<(), String>> {
        thread::spawn(move || drain_pipe(self.handle, channel))
    }
}

enum OutputChannel {
    Stdout,
    Stderr,
}

fn drain_pipe(pipe: OwnedHandle, channel: OutputChannel) -> Result<(), String> {
    let connected = unsafe { ConnectNamedPipe(pipe.0, ptr::null_mut()) };
    if connected == 0 {
        let code = unsafe { GetLastError() };
        if code != ERROR_PIPE_CONNECTED {
            return Err(format!("ConnectNamedPipe failed with code {code}"));
        }
    }
    let mut buffer = [0_u8; 8192];
    loop {
        let mut read = 0;
        let ok = unsafe {
            ReadFile(
                pipe.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            if code == ERROR_BROKEN_PIPE {
                return Ok(());
            }
            return Err(format!("ReadFile(named pipe) failed with code {code}"));
        }
        if read == 0 {
            return Ok(());
        }
        match channel {
            OutputChannel::Stdout => std::io::stdout().lock().write_all(&buffer[..read as usize]),
            OutputChannel::Stderr => std::io::stderr().lock().write_all(&buffer[..read as usize]),
        }
        .map_err(|error| format!("forward sandbox output: {error}"))?;
    }
}

fn open_pipe_writer(name: &OsStr) -> Result<OwnedHandle, String> {
    let name = wide(name);
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    inheritable(OwnedHandle::new(handle, "CreateFileW(named pipe)")?)
}

fn open_null_reader() -> Result<OwnedHandle, String> {
    let name = wide(OsStr::new("NUL"));
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    inheritable(OwnedHandle::new(handle, "CreateFileW(NUL)")?)
}

fn inheritable(handle: OwnedHandle) -> Result<OwnedHandle, String> {
    if unsafe { SetHandleInformation(handle.0, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        Err(last_error("SetHandleInformation(HANDLE_FLAG_INHERIT)"))
    } else {
        Ok(handle)
    }
}

fn switch_value(args: &[OsString], name: &str) -> Result<OsString, String> {
    let position = args
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| format!("missing {name}"))?;
    args.get(position + 1)
        .cloned()
        .ok_or_else(|| format!("missing value after {name}"))
}

fn write_handle(handle: HANDLE, mut bytes: &[u8]) -> Result<(), String> {
    while !bytes.is_empty() {
        let mut written = 0;
        let ok = unsafe {
            WriteFile(
                handle,
                bytes.as_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error("WriteFile(named pipe)"));
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

fn join_drain(handle: JoinHandle<Result<(), String>>, channel: &str) -> Result<(), String> {
    handle
        .join()
        .map_err(|_| format!("{channel} transport thread panicked"))?
}

struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    fn for_sid(sid: &str) -> Result<Self, String> {
        let sddl = wide(OsStr::new(&format!(
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})"
        )));
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
                "ConvertStringSecurityDescriptorToSecurityDescriptorW(pipe)",
            ));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe { LocalFree(self.descriptor.cast()) };
        }
    }
}

fn canonical_sid(value: &str) -> Result<String, String> {
    let value_w = wide(OsStr::new(value));
    let mut sid = ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(value_w.as_ptr(), &mut sid) } == 0 {
        return Err(last_error("ConvertStringSidToSidW(pipe identity)"));
    }
    let mut rendered = ptr::null_mut();
    let converted = unsafe { ConvertSidToStringSidW(sid, &mut rendered) };
    unsafe { LocalFree(sid) };
    if converted == 0 {
        return Err(last_error("ConvertSidToStringSidW(pipe identity)"));
    }
    let mut length = 0;
    unsafe {
        while *rendered.add(length) != 0 {
            length += 1;
        }
    }
    let result = unsafe { OsString::from_wide(std::slice::from_raw_parts(rendered, length)) }
        .to_string_lossy()
        .into_owned();
    unsafe { LocalFree(rendered.cast()) };
    Ok(result)
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
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
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(last_error(operation))
        } else {
            Ok(Self(handle))
        }
    }
}

// A Windows kernel handle is process-scoped. Moving its single Rust owner to a
// drain thread does not change its validity or permit concurrent destruction.
unsafe impl Send for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_backed_children_do_not_request_the_interactive_desktop() {
        let transport = WorkerTransport {
            stdin: OwnedHandle(ptr::null_mut()),
            stdout: OwnedHandle(ptr::null_mut()),
            stderr: OwnedHandle(ptr::null_mut()),
        };

        assert!(transport.startup_info().lpDesktop.is_null());
    }
}
