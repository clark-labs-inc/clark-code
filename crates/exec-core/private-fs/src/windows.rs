use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{SetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    AddAccessAllowedAceEx, GetLengthSid, GetTokenInformation, InitializeAcl, TokenUser,
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION,
    OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct WindowsHandle(HANDLE);

impl Drop for WindowsHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

struct CurrentUserSid {
    token_information: Vec<usize>,
}

impl CurrentUserSid {
    fn as_ptr(&self) -> PSID {
        let token_user = self.token_information.as_ptr().cast::<TOKEN_USER>();
        unsafe { (*token_user).User.Sid }
    }
}

pub(super) fn configure_access(options: &mut OpenOptions, read: bool, write: bool) {
    let mut access = WRITE_DAC;
    if read {
        access |= FILE_GENERIC_READ;
    }
    if write {
        access |= FILE_GENERIC_WRITE;
    }
    options.access_mode(access);
}

pub(super) fn protect(file: &File) -> io::Result<()> {
    protect_with_inheritance(file, 0)
}

pub(super) fn protect_directory(path: &std::path::Path) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .access_mode(FILE_GENERIC_READ | WRITE_DAC)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let directory = options.open(path)?;
    protect_with_inheritance(&directory, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)
}

fn protect_with_inheritance(file: &File, ace_flags: u32) -> io::Result<()> {
    // This is filesystem access control, not encryption: bytes remain
    // plaintext at rest for the current Windows user.
    let user = current_user_sid()?;
    let sid = user.as_ptr();
    let sid_bytes = unsafe { GetLengthSid(sid) };
    if sid_bytes == 0 {
        return Err(last_error("measure current user SID"));
    }
    let acl_bytes = size_of::<ACL>()
        .checked_add(size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>())
        .and_then(|bytes| bytes.checked_add(sid_bytes as usize))
        .ok_or_else(|| io::Error::other("current user ACL size overflow"))?;
    let acl_bytes =
        u32::try_from(acl_bytes).map_err(|_| io::Error::other("current user ACL is too large"))?;
    let mut acl_storage = vec![0usize; (acl_bytes as usize).div_ceil(size_of::<usize>())];
    let acl = acl_storage.as_mut_ptr().cast::<ACL>();
    if unsafe { InitializeAcl(acl, acl_bytes, ACL_REVISION) } == 0 {
        return Err(last_error("initialize owner-only file ACL"));
    }
    if unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, ace_flags, FILE_ALL_ACCESS, sid) } == 0 {
        return Err(last_error("grant current user private-file access"));
    }
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "protect private file DACL: {}",
                io::Error::from_raw_os_error(result as i32)
            ),
        ));
    }
    Ok(())
}

fn current_user_sid() -> io::Result<CurrentUserSid> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("open current process token"));
    }
    let token = WindowsHandle(token);

    let mut required_bytes = 0u32;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut required_bytes,
        );
    }
    if required_bytes == 0 {
        return Err(last_error("measure current user SID"));
    }
    let buffer_bytes = required_bytes;
    let mut token_information = vec![0usize; (buffer_bytes as usize).div_ceil(size_of::<usize>())];
    let mut returned_bytes = 0u32;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_information.as_mut_ptr().cast(),
            buffer_bytes,
            &mut returned_bytes,
        )
    } == 0
    {
        return Err(last_error("read current user SID"));
    }
    Ok(CurrentUserSid { token_information })
}

fn last_error(context: &str) -> io::Error {
    io::Error::other(format!("{context}: {}", io::Error::last_os_error()))
}

#[cfg(test)]
pub(super) fn assert_current_user_only_dacl(path: &std::path::Path) {
    let file = File::open(path).expect("open private file");
    assert_dacl(&file, true, 0);
}

#[cfg(test)]
pub(super) fn assert_current_user_only_directory_dacl(path: &std::path::Path) {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let directory = options.open(path).expect("open private directory");
    assert_dacl(&directory, true, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE);
}

#[cfg(test)]
pub(super) fn assert_inherited_current_user_only_dacl(path: &std::path::Path) {
    let file = File::open(path).expect("open inherited private file");
    assert_dacl(&file, false, 0);
}

#[cfg(test)]
fn assert_dacl(file: &File, require_protected: bool, required_ace_flags: u32) {
    use windows_sys::Win32::Foundation::HLOCAL;
    use windows_sys::Win32::Security::Authorization::GetSecurityInfo;
    use windows_sys::Win32::Security::{
        EqualSid, GetAce, GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    };

    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(result, ERROR_SUCCESS, "read private-file DACL");
    assert!(!dacl.is_null(), "private file must have a DACL");
    assert!(
        !descriptor.is_null(),
        "security descriptor must be returned"
    );

    let mut control = 0u16;
    let mut revision = 0u32;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0,
        "read security descriptor control"
    );
    if require_protected {
        assert_ne!(
            control & SE_DACL_PROTECTED,
            0,
            "private DACL must not inherit additional access"
        );
    }
    assert_eq!(unsafe { (*dacl).AceCount }, 1, "DACL must have one ACE");

    let mut ace_pointer = std::ptr::null_mut();
    assert_ne!(
        unsafe { GetAce(dacl, 0, &mut ace_pointer) },
        0,
        "read private-file ACE"
    );
    let ace = unsafe { &*ace_pointer.cast::<ACCESS_ALLOWED_ACE>() };
    assert_eq!(ace.Header.AceType, 0, "ACE must allow access");
    assert_eq!(ace.Mask, FILE_ALL_ACCESS);
    assert_eq!(
        u32::from(ace.Header.AceFlags) & required_ace_flags,
        required_ace_flags,
        "private directory ACE must inherit to children"
    );
    let ace_sid = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
    let current_user = current_user_sid().expect("current user SID");
    assert_ne!(
        unsafe { EqualSid(ace_sid, current_user.as_ptr()) },
        0,
        "sole ACE must belong to the current user"
    );

    unsafe {
        windows_sys::Win32::Foundation::LocalFree(descriptor.cast() as HLOCAL);
    }
}
