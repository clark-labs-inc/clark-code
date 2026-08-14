use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use exec_sandbox_protocol::WindowsSetupRequest;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_SUCCESS, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, DENY_ACCESS,
    EXPLICIT_ACCESS_W, SET_ACCESS, SE_FILE_OBJECT, SE_KERNEL_OBJECT, SE_WINDOW_OBJECT,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::StationsAndDesktops::GetProcessWindowStation;
use windows_sys::Win32::UI::WindowsAndMessaging::WINSTA_ALL_ACCESS;

/// Install the one machine-wide object permission shared by every restricted
/// token. This runs only in the elevated bootstrap helper.
pub fn bootstrap() -> Result<(), String> {
    grant_null_device(&exec_sandbox_protocol::WireSandboxPolicy::device_capability_sid())
}

/// Enroll roots the current desktop user already controls. Opening every root
/// with WRITE_DAC makes Windows itself the authority check: ordinary owned
/// workspaces need no elevation, while protected roots fail closed and can be
/// retried through a deliberately elevated fallback.
pub fn enroll(request: &WindowsSetupRequest, sid: &str) -> Result<(), String> {
    for root in &request.policy.write_roots {
        ensure_not_volume_root(root)?;
        std::fs::create_dir_all(root)
            .map_err(|error| format!("create sandbox write root {}: {error}", root.display()))?;
        set_path_ace(
            root,
            sid,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE,
            SET_ACCESS,
        )?;
        let capability =
            exec_sandbox_protocol::WireSandboxPolicy::write_capability_sid_for_root(root);
        set_path_ace(
            root,
            &capability,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE,
            SET_ACCESS,
        )?;
    }
    for denied in &request.policy.deny_write {
        if denied.exists() {
            set_path_ace(
                denied,
                sid,
                FILE_GENERIC_WRITE | DELETE | FILE_DELETE_CHILD,
                DENY_ACCESS,
            )?;
        }
    }
    Ok(())
}

/// Permit the restricted offline worker to re-read the non-secret setup
/// attestation before it creates the sandboxed child. The state directory only
/// grants traversal and the marker only grants read; credentials remain
/// inaccessible. Both the offline account and the stable device capability are
/// required because a `WRITE_RESTRICTED` token must satisfy each SID set.
pub fn grant_setup_marker_read(
    state_dir: &Path,
    marker_path: &Path,
    offline_sid: &str,
) -> Result<(), String> {
    let device_capability = exec_sandbox_protocol::WireSandboxPolicy::device_capability_sid();
    for sid in [offline_sid, device_capability.as_str()] {
        set_path_ace_with_inheritance(state_dir, sid, FILE_GENERIC_EXECUTE, SET_ACCESS, 0)?;
        set_path_ace_with_inheritance(marker_path, sid, FILE_GENERIC_READ, SET_ACCESS, 0)?;
    }
    Ok(())
}

/// Let the offline identity enter the requested working directory even when a
/// hosted runner has placed it below an interactive-user-only temporary root.
/// Windows mode deliberately permits host-wide reads, so this grants only the
/// read/execute access needed to preserve the caller's working-directory
/// contract; writable authority still comes exclusively from capability SIDs.
pub fn grant_runtime_cwd_read(path: &Path, offline_sid: &str) -> Result<(), String> {
    ensure_not_volume_root(path)?;
    set_path_ace_with_inheritance(
        path,
        offline_sid,
        FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        SET_ACCESS,
        0,
    )
}

/// `WRITE_RESTRICTED` checks the restricting SID list for window-station write
/// access as well as filesystem writes. Console programs connect to the
/// worker's noninteractive station during startup, so its synthetic capability
/// SIDs must pass that second check or conhost can wait forever. The ordinary
/// offline account must still pass the station DACL independently, so these
/// ACEs cannot broaden the base token's authority.
pub fn grant_current_window_station_access(capability_sids: &[String]) -> Result<(), String> {
    let station = unsafe { GetProcessWindowStation() };
    if station.is_null() {
        return Err("GetProcessWindowStation returned null".to_string());
    }
    for sid in capability_sids {
        set_handle_ace(
            station,
            SE_WINDOW_OBJECT,
            sid,
            WINSTA_ALL_ACCESS as u32,
            SET_ACCESS,
            0,
            "Windows sandbox window station",
        )?;
    }
    Ok(())
}

fn set_path_ace(path: &Path, sid: &str, permissions: u32, mode: i32) -> Result<(), String> {
    set_path_ace_with_inheritance(
        path,
        sid,
        permissions,
        mode,
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
    )
}

fn set_path_ace_with_inheritance(
    path: &Path,
    sid: &str,
    permissions: u32,
    mode: i32,
    inheritance: u32,
) -> Result<(), String> {
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            READ_CONTROL | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "open sandbox ACL root {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let result = set_handle_ace(
        handle,
        SE_FILE_OBJECT,
        sid,
        permissions,
        mode,
        inheritance,
        &format!("sandbox ACL root {}", path.display()),
    );
    unsafe { CloseHandle(handle) };
    result
}

fn grant_null_device(sid: &str) -> Result<(), String> {
    let nul = r"\\.\NUL".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            nul.as_ptr(),
            READ_CONTROL | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "open Windows NUL security descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = set_handle_ace(
        handle,
        SE_KERNEL_OBJECT,
        sid,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
        SET_ACCESS,
        0,
        "Windows NUL security descriptor",
    );
    unsafe { CloseHandle(handle) };
    result
}

#[allow(clippy::too_many_arguments)]
fn set_handle_ace(
    handle: *mut std::ffi::c_void,
    object_type: i32,
    sid: &str,
    permissions: u32,
    mode: i32,
    inheritance: u32,
    label: &str,
) -> Result<(), String> {
    let sid_wide = sid.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut sid_ptr = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(sid_wide.as_ptr(), &mut sid_ptr) } == 0 {
        return Err(format!(
            "convert {label} SID: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut security_descriptor = std::ptr::null_mut();
    let result = unsafe {
        let mut existing_dacl: *mut ACL = std::ptr::null_mut();
        let get_result = GetSecurityInfo(
            handle,
            object_type,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut existing_dacl,
            std::ptr::null_mut(),
            &mut security_descriptor,
        );
        if get_result != ERROR_SUCCESS {
            Err(format!("read {label}: code {get_result}"))
        } else {
            let explicit = EXPLICIT_ACCESS_W {
                grfAccessPermissions: permissions,
                grfAccessMode: mode,
                grfInheritance: inheritance,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: 0,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_UNKNOWN,
                    ptstrName: sid_ptr.cast(),
                },
            };
            let mut updated_dacl: *mut ACL = std::ptr::null_mut();
            let update_result = SetEntriesInAclW(1, &explicit, existing_dacl, &mut updated_dacl);
            let result = if update_result != ERROR_SUCCESS {
                Err(format!("update {label}: code {update_result}"))
            } else {
                let set_result = SetSecurityInfo(
                    handle,
                    object_type,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    updated_dacl,
                    std::ptr::null_mut(),
                );
                if set_result == ERROR_SUCCESS {
                    Ok(())
                } else {
                    Err(format!("write {label}: code {set_result}"))
                }
            };
            if !updated_dacl.is_null() {
                LocalFree(updated_dacl.cast::<std::ffi::c_void>() as HLOCAL);
            }
            result
        }
    };

    unsafe {
        if !security_descriptor.is_null() {
            LocalFree(security_descriptor.cast::<std::ffi::c_void>() as HLOCAL);
        }
        LocalFree(sid_ptr.cast());
    }
    result
}

fn ensure_not_volume_root(path: &Path) -> Result<(), String> {
    if path.parent().is_none()
        || path
            .parent()
            .is_some_and(|parent| parent.parent().is_none())
    {
        Err(format!(
            "refusing to grant sandbox identity access to volume root {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}
