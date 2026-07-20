//! Lifetime-bound containment for child-process trees.
//!
//! Unix children are isolated into a process group before launch. Windows has
//! no equivalent process-group guarantee, so a Job Object owns each child tree
//! instead. Closing the Job Object kills every process assigned to it.

/// Keeps a spawned child process tree bound to the caller's lifetime.
///
/// Attachment is deliberately best effort. Some managed Windows environments
/// prohibit assigning a child to a second Job Object; command execution still
/// works there, and the existing explicit `taskkill /T` cleanup remains the
/// fallback. When attachment succeeds, dropping this value terminates every
/// descendant automatically.
#[derive(Default)]
pub struct ProcessFence {
    #[cfg(windows)]
    job: Option<JobObject>,
}

impl ProcessFence {
    /// Attach the process identified by `pid` to a lifetime-bound containment
    /// boundary. Keep the returned value alive until the process has finished.
    pub fn attach(pid: Option<u32>) -> Self {
        #[cfg(windows)]
        {
            let Some(pid) = pid else {
                return Self::default();
            };
            let Some(job) = JobObject::create_kill_on_close() else {
                return Self::default();
            };
            if job.assign(pid) {
                return Self { job: Some(job) };
            }
            Self::default()
        }

        #[cfg(not(windows))]
        {
            let _ = pid;
            Self::default()
        }
    }

    /// Whether the platform provided a containment boundary for this child.
    pub fn is_enforced(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.is_some()
        }

        #[cfg(not(windows))]
        {
            false
        }
    }
}

#[cfg(windows)]
struct JobObject(windows_sys::Win32::Foundation::HANDLE);

// Job handles are kernel object references with thread-independent ownership.
// This wrapper has unique ownership and closes the handle exactly once. Its
// shared API only queries the wrapper after construction; Windows permits job
// handles to be used from any thread, so moving or sharing the owner is safe.
#[cfg(windows)]
unsafe impl Send for JobObject {}

#[cfg(windows)]
unsafe impl Sync for JobObject {}

#[cfg(windows)]
impl JobObject {
    fn create_kill_on_close() -> Option<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
            return None;
        }
        Some(Self(handle))
    }

    fn assign(&self, pid: u32) -> bool {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return false;
        }
        let assigned = unsafe { AssignProcessToJobObject(self.0, process) } != 0;
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(process);
        }
        assigned
    }
}

#[cfg(windows)]
impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessFence;

    #[test]
    fn absent_process_never_claims_containment() {
        assert!(!ProcessFence::attach(None).is_enforced());
    }
}
