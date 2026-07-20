use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use windows_sys::Win32::Foundation::{GetLastError, LocalFree, ERROR_INSUFFICIENT_BUFFER};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NetUserAdd, NetUserSetInfo, UF_DONT_EXPIRE_PASSWD, UF_SCRIPT, USER_INFO_1, USER_INFO_1003,
    USER_PRIV_USER,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, CryptProtectData, CryptUnprotectData, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};

use crate::provision::ProvisionedIdentity;
use crate::state::{
    credential_path, read_credential, write_json_atomic, CredentialEnvelope, CREDENTIAL_VERSION,
};

const OFFLINE_USERNAME: &str = "ClarkSandboxOffline";
const NERR_SUCCESS: u32 = 0;
const NERR_USER_EXISTS: u32 = 2224;

pub fn ensure_offline_identity(state_dir: &Path) -> Result<ProvisionedIdentity, String> {
    let password = random_password()?;
    ensure_local_user(OFFLINE_USERNAME, &password)?;
    let sid = account_sid_string(OFFLINE_USERNAME)?;
    let protected_password = protect_password(&password)?;
    let credential = CredentialEnvelope {
        version: CREDENTIAL_VERSION,
        username: OFFLINE_USERNAME.to_string(),
        sid: sid.clone(),
        protected_password_b64: BASE64.encode(protected_password),
    };
    write_json_atomic(&credential_path(state_dir), &credential)?;
    Ok(ProvisionedIdentity { sid })
}

pub fn load_offline_password(state_dir: &Path) -> Result<(String, String, String), String> {
    let credential = read_credential(state_dir)?;
    let protected = BASE64
        .decode(&credential.protected_password_b64)
        .map_err(|error| format!("decode Windows sandbox credential: {error}"))?;
    let password = unprotect_password(&protected)?;
    Ok((credential.username, password, credential.sid))
}

fn ensure_local_user(username: &str, password: &str) -> Result<(), String> {
    let mut username_w = wide(username);
    let mut password_w = wide(password);
    let info = USER_INFO_1 {
        usri1_name: username_w.as_mut_ptr(),
        usri1_password: password_w.as_mut_ptr(),
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: ptr::null_mut(),
        usri1_comment: ptr::null_mut(),
        usri1_flags: UF_SCRIPT | UF_DONT_EXPIRE_PASSWD,
        usri1_script_path: ptr::null_mut(),
    };
    let mut parameter_error = 0;
    let status = unsafe {
        NetUserAdd(
            ptr::null(),
            1,
            &info as *const USER_INFO_1 as *const u8,
            &mut parameter_error,
        )
    };
    if status == NERR_SUCCESS {
        return Ok(());
    }
    if status != NERR_USER_EXISTS {
        return Err(format!(
            "create Windows sandbox identity failed with status {status} at parameter {parameter_error}"
        ));
    }

    let update = USER_INFO_1003 {
        usri1003_password: password_w.as_mut_ptr(),
    };
    let status = unsafe {
        NetUserSetInfo(
            ptr::null(),
            username_w.as_ptr(),
            1003,
            &update as *const USER_INFO_1003 as *const u8,
            &mut parameter_error,
        )
    };
    if status == NERR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "update Windows sandbox identity failed with status {status} at parameter {parameter_error}"
        ))
    }
}

fn account_sid_string(username: &str) -> Result<String, String> {
    let username = wide(username);
    let mut sid_bytes = 0;
    let mut domain_chars = 0;
    let mut sid_kind = SID_NAME_USE::default();
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            username.as_ptr(),
            ptr::null_mut(),
            &mut sid_bytes,
            ptr::null_mut(),
            &mut domain_chars,
            &mut sid_kind,
        );
    }
    if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(format!(
            "size Windows sandbox identity SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut sid = vec![0_u8; sid_bytes as usize];
    let mut domain = vec![0_u16; domain_chars as usize];
    let found = unsafe {
        LookupAccountNameW(
            ptr::null(),
            username.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_bytes,
            domain.as_mut_ptr(),
            &mut domain_chars,
            &mut sid_kind,
        )
    };
    if found == 0 {
        return Err(format!(
            "resolve Windows sandbox identity SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut text = ptr::null_mut();
    let converted = unsafe { ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut text) };
    if converted == 0 {
        return Err(format!(
            "format Windows sandbox identity SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = wide_ptr_to_string(text);
    unsafe {
        LocalFree(text.cast());
    }
    result
}

fn random_password() -> Result<String, String> {
    let mut random = [0_u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            random.as_mut_ptr(),
            random.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!(
            "generate Windows sandbox credential failed with NTSTATUS {status:#x}"
        ));
    }
    Ok(format!("C!{}a9Z", BASE64.encode(random)))
}

fn protect_password(password: &str) -> Result<Vec<u8>, String> {
    let bytes = password.as_bytes();
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let description = wide("Clark sandbox offline identity");
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let protected = unsafe {
        CryptProtectData(
            &input,
            description.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if protected == 0 {
        return Err(format!(
            "protect Windows sandbox credential: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

fn unprotect_password(protected: &[u8]) -> Result<String, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let unprotected = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if unprotected == 0 {
        return Err(format!(
            "unprotect Windows sandbox credential: {}",
            std::io::Error::last_os_error()
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = String::from_utf8(bytes.to_vec())
        .map_err(|error| format!("Windows sandbox credential is not UTF-8: {error}"));
    unsafe {
        LocalFree(output.pbData.cast());
    }
    result
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn wide_ptr_to_string(value: *const u16) -> Result<String, String> {
    if value.is_null() {
        return Err("Windows returned a null UTF-16 string".into());
    }
    let mut length = 0;
    unsafe {
        while *value.add(length) != 0 {
            length += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(value, length))
            .map_err(|error| format!("Windows returned invalid UTF-16: {error}"))
    }
}

#[allow(dead_code)]
fn _assert_send_sync(_: *mut c_void) {}
