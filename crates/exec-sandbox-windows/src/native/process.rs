use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use exec_sandbox_protocol::WindowsRunnerRequest;

pub(super) const WORKER_REQUEST_ENV: &str = "CLARK_WINDOWS_SANDBOX_WORKER_REQUEST_B64";
pub(super) const TRACE_ENV: &str = "CLARK_WINDOWS_SANDBOX_TRACE";

pub(super) fn worker_environment(encoded_request: &str) -> Vec<u16> {
    let mut overrides = windows_noninteractive_environment().collect::<Vec<_>>();
    overrides.push((
        OsString::from(WORKER_REQUEST_ENV),
        OsString::from(encoded_request),
    ));
    if let Some(value) = std::env::var_os(TRACE_ENV) {
        overrides.push((OsString::from(TRACE_ENV), value));
    }
    environment_block(overrides)
}

pub(super) fn inner_environment(request: &WindowsRunnerRequest) -> Vec<u16> {
    let mut overrides = request
        .process
        .env
        .iter()
        .map(|(key, value)| (key.to_os_string(), value.to_os_string()))
        .collect::<Vec<_>>();
    overrides.extend(windows_noninteractive_environment());
    inject_git_safe_directory(
        &mut overrides,
        Path::new(&request.process.cwd.to_os_string()),
    );
    if let Some(temp) = &request.policy.process_temp_root {
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TMPDIR",
            "TMP",
            "TEMP",
        ] {
            overrides.push((OsString::from(key), temp.as_os_str().to_os_string()));
        }
    }
    environment_block(overrides)
}

fn windows_noninteractive_environment() -> impl Iterator<Item = (OsString, OsString)> {
    [
        // `more.com` needs a console and can leave a hidden sandbox child
        // waiting forever. Git documents `cat` as the non-pager value; it
        // also overrides any machine-wide `core.pager` configuration.
        ("PAGER", "cat"),
        ("GIT_PAGER", "cat"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("GIT_TERMINAL_PROMPT", "0"),
        // Do not let a machine-wide Git config select a pager, credential
        // helper, or other host-integrated behavior for the offline worker.
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "core.fsmonitor"),
        ("GIT_CONFIG_VALUE_0", "false"),
        ("TERM", "dumb"),
        ("NO_COLOR", "1"),
    ]
    .into_iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
}

fn inject_git_safe_directory(overrides: &mut Vec<(OsString, OsString)>, cwd: &Path) {
    let Some(git_root) = cwd
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
    else {
        return;
    };
    // The non-interactive baseline owns slot zero (`core.fsmonitor=false`).
    // Append instead of replacing it so repository-selected fsmonitor helpers
    // remain disabled while Git accepts the deliberately different offline
    // account as a read-only/workspace-scoped operator.
    overrides.extend([
        (OsString::from("GIT_CONFIG_COUNT"), OsString::from("2")),
        (
            OsString::from("GIT_CONFIG_KEY_1"),
            OsString::from("safe.directory"),
        ),
        (
            OsString::from("GIT_CONFIG_VALUE_1"),
            git_root.as_os_str().to_os_string(),
        ),
    ]);
}

fn environment_block(overrides: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<u16> {
    let mut values = BTreeMap::<String, (OsString, OsString)>::new();
    for key in ["SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT"] {
        if let Some(value) = std::env::var_os(key) {
            values.insert(key.to_ascii_uppercase(), (OsString::from(key), value));
        }
    }
    for (key, value) in overrides {
        values.insert(key.to_string_lossy().to_ascii_uppercase(), (key, value));
    }
    let mut block = Vec::new();
    for (_, (key, value)) in values {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

pub(super) fn command_line(program: &Path, args: &[OsString]) -> Vec<u16> {
    let mut rendered = quote_windows_argument(program.as_os_str());
    for argument in args {
        rendered.push(b' ' as u16);
        rendered.extend(quote_windows_argument(argument));
    }
    rendered.push(0);
    rendered
}

fn quote_windows_argument(value: &OsStr) -> Vec<u16> {
    let value = value.encode_wide().collect::<Vec<_>>();
    if !value.is_empty()
        && !value
            .iter()
            .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x0a | 0x0d | 0x0b | 0x22))
    {
        return value;
    }
    let mut result = vec![b'"' as u16];
    let mut backslashes = 0;
    for unit in value {
        if unit == b'\\' as u16 {
            backslashes += 1;
        } else if unit == b'"' as u16 {
            result.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            result.push(b'"' as u16);
            backslashes = 0;
        } else {
            result.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
            backslashes = 0;
            result.push(unit);
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    result.push(b'"' as u16);
    result
}

pub(super) fn wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_argument_quoting_preserves_spaces_quotes_and_backslashes() {
        let rendered =
            |value: &str| String::from_utf16(&quote_windows_argument(OsStr::new(value))).unwrap();
        assert_eq!(rendered("plain"), "plain");
        assert_eq!(rendered("two words"), "\"two words\"");
        assert_eq!(rendered("a\\\"b"), "\"a\\\\\\\"b\"");
        assert_eq!(rendered("tail\\"), "tail\\");
    }

    #[test]
    fn environment_is_sorted_case_insensitively_and_double_terminated() {
        let block = environment_block([
            (OsString::from("zeta"), OsString::from("2")),
            (OsString::from("Alpha"), OsString::from("1")),
        ]);
        assert_eq!(block.last(), Some(&0));
        assert_eq!(block[block.len() - 2], 0);
        let text = String::from_utf16_lossy(&block);
        assert!(text.find("Alpha=1").unwrap() < text.find("zeta=2").unwrap());
    }

    #[test]
    fn noninteractive_git_environment_cannot_spawn_a_pager_or_system_helper() {
        let values = windows_noninteractive_environment().collect::<BTreeMap<_, _>>();
        assert_eq!(
            values.get(&OsString::from("PAGER")),
            Some(&OsString::from("cat"))
        );
        assert_eq!(
            values.get(&OsString::from("GIT_PAGER")),
            Some(&OsString::from("cat"))
        );
        assert_eq!(
            values.get(&OsString::from("GIT_CONFIG_NOSYSTEM")),
            Some(&OsString::from("1"))
        );
    }
}
