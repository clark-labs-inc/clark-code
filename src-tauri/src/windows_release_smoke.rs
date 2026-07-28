use std::ffi::OsStr;
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

const WINDOWS_SANDBOX_SMOKE_ARG: &str = "--windows-sandbox-smoke";

pub fn run_windows_sandbox_smoke_if_requested() -> bool {
    let Some((output, cwd)) = windows_sandbox_smoke_paths(std::env::args_os().skip(1)) else {
        return false;
    };

    #[cfg(windows)]
    {
        run_windows_sandbox_smoke(&output, &cwd);
        true
    }

    #[cfg(not(windows))]
    {
        let _ = (output, cwd);
        eprintln!("Windows sandbox smoke is supported only on Windows");
        std::process::exit(2);
    }
}

fn windows_sandbox_smoke_paths(
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Option<(PathBuf, PathBuf)> {
    let mut arguments = arguments.into_iter();
    if arguments.next()?.as_ref() != WINDOWS_SANDBOX_SMOKE_ARG {
        return None;
    }
    let output = PathBuf::from(arguments.next()?.as_ref());
    let cwd = PathBuf::from(arguments.next()?.as_ref());
    if arguments.next().is_some() || !is_windows_absolute(&output) || !is_windows_absolute(&cwd) {
        return None;
    }
    Some((output, cwd))
}

fn is_windows_absolute(path: &PathBuf) -> bool {
    let value = path.to_string_lossy().as_bytes().to_vec();
    value.len() >= 3
        && value[0].is_ascii_alphabetic()
        && value[1] == b':'
        && matches!(value[2], b'\\' | b'/')
}

#[cfg(windows)]
fn run_windows_sandbox_smoke(output: &Path, cwd: &Path) {
    use exec_core::Executor;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    if let Err(error) = clark_install_context::activate_bundled_path() {
        fail(&format!("could not activate packaged PATH: {error}"));
    }
    let inside = cwd.join("clark-release-sandbox-smoke.txt");
    let outside = PathBuf::from(r"C:\Users\Public\clark-release-sandbox-escape.txt");
    let _ = std::fs::remove_file(&inside);
    let _ = std::fs::remove_file(&outside);
    let (inside_command, outside_command) =
        smoke_commands(exec_core::scripted_shell_kind(), &inside, &outside);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|error| fail(&format!("sandbox smoke runtime failed: {error}")));
    let result = runtime.block_on(async {
        let executor = crate::sandbox_setup::release_smoke_executor(cwd)?;
        let ordinary = executor
            .exec_streaming(
                &inside_command,
                cwd,
                Duration::from_secs(15),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await?;
        let terminal = executor
            .exec_streaming_pty(
                &inside_command,
                cwd,
                Duration::from_secs(15),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await?;
        let escape = executor
            .exec(
                &outside_command,
                cwd,
                Duration::from_secs(15),
                &CancellationToken::new(),
            )
            .await?;
        Ok::<_, String>((ordinary, terminal, escape))
    });
    let receipt = match result {
        Ok((ordinary, terminal, escape)) => {
            let ordinary_output = String::from_utf8_lossy(&ordinary.stdout);
            let terminal_output = String::from_utf8_lossy(&terminal.stdout);
            let inside_written = std::fs::read_to_string(&inside)
                .is_ok_and(|value| value.contains("CLARK_SANDBOX_OK"));
            let outside_write_blocked = escape.code != 0 && !outside.exists();
            let passed = ordinary.code == 0
                && terminal.code == 0
                && ordinary_output.contains("CLARK_SANDBOX_OK")
                && terminal_output.contains("CLARK_SANDBOX_OK")
                && inside_written
                && outside_write_blocked;
            json!({
                "status": if passed { "passed" } else { "failed" },
                "containment": "managed",
                "ordinary_exit_code": ordinary.code,
                "ordinary_output_seen": ordinary_output.contains("CLARK_SANDBOX_OK"),
                "pty_exit_code": terminal.code,
                "pty_output_seen": terminal_output.contains("CLARK_SANDBOX_OK"),
                "inside_write_observed": inside_written,
                "outside_exit_code": escape.code,
                "outside_write_blocked": outside_write_blocked,
            })
        }
        Err(error) => json!({ "status": "failed", "error": error }),
    };
    let _ = std::fs::remove_file(&inside);
    let _ = std::fs::remove_file(&outside);
    if let Err(error) = std::fs::write(output, serde_json::to_vec(&receipt).unwrap_or_default()) {
        fail(&format!("sandbox smoke receipt failed: {error}"));
    }
    if receipt["status"] != "passed" {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn smoke_commands(kind: exec_core::ShellKind, inside: &Path, outside: &Path) -> (String, String) {
    match kind {
        exec_core::ShellKind::PowerShell => (
            format!(
                "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value CLARK_SANDBOX_OK; Get-Content -LiteralPath '{}'",
                powershell_path(inside),
                powershell_path(inside),
            ),
            format!(
                "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value ESCAPE",
                powershell_path(outside),
            ),
        ),
        exec_core::ShellKind::Cmd => (
            format!(
                "> \"{}\" echo CLARK_SANDBOX_OK & type \"{}\"",
                inside.display(),
                inside.display(),
            ),
            format!("> \"{}\" echo ESCAPE", outside.display()),
        ),
        exec_core::ShellKind::Posix => unreachable!("Windows cannot select a POSIX shell"),
    }
}

#[cfg(windows)]
fn powershell_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

#[cfg(windows)]
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::{windows_sandbox_smoke_paths, WINDOWS_SANDBOX_SMOKE_ARG};

    #[test]
    fn sandbox_smoke_requires_exact_absolute_output_and_cwd() {
        assert_eq!(
            windows_sandbox_smoke_paths([
                WINDOWS_SANDBOX_SMOKE_ARG,
                r"C:\Users\Public\receipt.json",
                r"C:\Users\home\ClarkCodeQA",
            ]),
            Some((
                std::path::PathBuf::from(r"C:\Users\Public\receipt.json"),
                std::path::PathBuf::from(r"C:\Users\home\ClarkCodeQA"),
            )),
        );
        assert_eq!(
            windows_sandbox_smoke_paths([
                WINDOWS_SANDBOX_SMOKE_ARG,
                "receipt.json",
                r"C:\Users\home\ClarkCodeQA",
            ]),
            None,
        );
        assert_eq!(
            windows_sandbox_smoke_paths([
                WINDOWS_SANDBOX_SMOKE_ARG,
                r"C:\Users\Public\receipt.json",
            ]),
            None,
        );
    }
}
