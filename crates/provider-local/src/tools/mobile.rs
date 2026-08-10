//! Shared helpers for the mobile device control tools (`ios_simulator`,
//! `android_emulator`). These deliberately never go through `ctx.executor` —
//! simulators/emulators are a property of the machine running Clark Code
//! Desktop's own GUI, not of whichever project executor (local or
//! SSH-remote) happens to be active for the current session.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::ToolCtx;

#[derive(Debug)]
pub(crate) struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

/// Run a local system binary directly and capture its output as text. Mirrors
/// `exec-core`'s `exec()` cancel/timeout `select!` pattern, parameterized on
/// `program`/`args` instead of hardcoded to `/bin/sh -c`. A spawn failure
/// because the binary isn't on `PATH` is mapped to `install_hint` instead of
/// a raw OS error, since every caller here is gated behind an optional local
/// toolchain (Xcode Command Line Tools, `idb`, Android SDK Platform Tools).
pub(crate) async fn run_cmd(
    program: &str,
    args: &[&str],
    timeout: Duration,
    cancel: &CancellationToken,
    install_hint: &str,
) -> Result<CmdOutput, String> {
    let child = spawn(program, args, install_hint)?;
    let wait = child.wait_with_output();
    let output = tokio::select! {
        _ = cancel.cancelled() => return Err("command cancelled".into()),
        res = tokio::time::timeout(timeout, wait) => res,
    };
    match output {
        Ok(Ok(out)) => Ok(CmdOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code(),
        }),
        Ok(Err(e)) => Err(format!("{program} failed: {e}")),
        Err(_) => Err(format!(
            "{program} timed out after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Same as `run_cmd`, but returns raw stdout bytes instead of lossily
/// decoding it as text — for binary output piped straight off a command's
/// stdout (a screenshot PNG from `adb exec-out screencap -p`, for example).
pub(crate) async fn run_cmd_bytes(
    program: &str,
    args: &[&str],
    timeout: Duration,
    cancel: &CancellationToken,
    install_hint: &str,
) -> Result<(Vec<u8>, Option<i32>), String> {
    let child = spawn(program, args, install_hint)?;
    let wait = child.wait_with_output();
    let output = tokio::select! {
        _ = cancel.cancelled() => return Err("command cancelled".into()),
        res = tokio::time::timeout(timeout, wait) => res,
    };
    match output {
        Ok(Ok(out)) => Ok((out.stdout, out.status.code())),
        Ok(Err(e)) => Err(format!("{program} failed: {e}")),
        Err(_) => Err(format!(
            "{program} timed out after {} ms",
            timeout.as_millis()
        )),
    }
}

fn spawn(
    program: &str,
    args: &[&str],
    install_hint: &str,
) -> Result<tokio::process::Child, String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    exec_core::suppress_console_window(&mut command);
    command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{program} is not installed. {install_hint}")
        } else {
            format!("failed to spawn {program}: {e}")
        }
    })
}

/// A short, monotonically-increasing slug for uniquely naming a screenshot
/// file (`<platform>-<device>-<slug>.png`).
pub(crate) fn timestamp_slug() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

/// Where mobile-tool screenshots get written: the session's document
/// workspace when one exists (so the Artifact card can pick it up), falling
/// back to the app-wide workspace root, and finally the project sandbox root
/// itself (always available) for a remote-project session with neither.
pub(crate) fn screenshot_dir(ctx: &ToolCtx) -> PathBuf {
    ctx.sandbox
        .docs_root()
        .map(|p| p.to_path_buf())
        .or_else(crate::workspace::workspace_root)
        .unwrap_or_else(|| ctx.sandbox.root().to_path_buf())
        .join("mobile-screenshots")
}

/// Best-effort retention for `screenshot_dir`: drop files older than a week,
/// then trim to the newest 200 if still over. Called after every screenshot
/// write; errors are logged and swallowed — pruning must never fail the tool
/// call that triggered it.
pub(crate) fn prune_screenshots(dir: &std::path::Path) {
    const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
    const MAX_COUNT: usize = 200;

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("prune_screenshots: couldn't list {}: {e}", dir.display());
            return;
        }
    };

    let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if let Ok(age) = std::time::SystemTime::now().duration_since(modified) {
            if age > MAX_AGE {
                if let Err(e) = std::fs::remove_file(entry.path()) {
                    tracing::warn!("prune_screenshots: couldn't remove {:?}: {e}", entry.path());
                }
                continue;
            }
        }
        files.push((entry.path(), modified));
    }

    if files.len() > MAX_COUNT {
        files.sort_by_key(|(_, modified)| *modified);
        for (path, _) in files.iter().take(files.len() - MAX_COUNT) {
            if let Err(e) = std::fs::remove_file(path) {
                tracing::warn!("prune_screenshots: couldn't remove {path:?}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn run_cmd_maps_missing_binary_to_install_hint() {
        let err = run_cmd(
            "definitely-not-a-real-binary-xyz",
            &[],
            Duration::from_secs(1),
            &CancellationToken::new(),
            "install it via brew",
        )
        .await
        .unwrap_err();
        assert!(err.contains("is not installed"));
        assert!(err.contains("install it via brew"));
    }

    #[tokio::test]
    async fn run_cmd_captures_stdout() {
        #[cfg(windows)]
        let (program, args) = ("cmd.exe", &["/D", "/Q", "/C", "echo hello"][..]);
        #[cfg(not(windows))]
        let (program, args) = ("echo", &["hello"][..]);
        let out = run_cmd(
            program,
            args,
            Duration::from_secs(5),
            &CancellationToken::new(),
            "",
        )
        .await
        .unwrap();
        assert_eq!(out.stdout.trim(), "hello");
        assert_eq!(out.code, Some(0));
    }

    #[tokio::test]
    async fn run_cmd_bytes_captures_raw_stdout() {
        #[cfg(windows)]
        let (program, args) = ("cmd.exe", &["/D", "/Q", "/C", "echo raw"][..]);
        #[cfg(not(windows))]
        let (program, args) = ("printf", &["%s", "\\xff\\xferaw"][..]);
        let (bytes, code) = run_cmd_bytes(
            program,
            args,
            Duration::from_secs(5),
            &CancellationToken::new(),
            "",
        )
        .await
        .unwrap();
        assert_eq!(code, Some(0));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn prune_screenshots_leaves_a_small_directory_untouched() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("shot-{i}.png")), [0u8; 1]).unwrap();
        }
        // Far under MAX_COUNT and freshly written (under MAX_AGE): nothing removed.
        prune_screenshots(dir.path());
        let remaining: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(remaining.len(), 5);
    }
}
