//! Bounded Git metadata commands used by repository discovery and context.
//!
//! These commands are observational. They must not acquire optional locks,
//! execute repository-selected hooks/helpers, or wait for interactive auth.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::exec::Executor;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

pub(crate) async fn optional(
    exec: &dyn Executor,
    root: &Path,
    args: &[&str],
) -> Result<Option<String>, String> {
    let output = run(exec, root, args, &[]).await?;
    if output.code != Some(0) {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub(crate) async fn required(
    exec: &dyn Executor,
    root: &Path,
    args: &[&str],
) -> Result<String, String> {
    required_with_env(exec, root, args, &[]).await
}

pub(crate) async fn required_with_env(
    exec: &dyn Executor,
    root: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<String, String> {
    let output = run(exec, root, args, env).await?;
    if output.code != Some(0) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) async fn succeeds(
    exec: &dyn Executor,
    root: &Path,
    args: &[&str],
) -> Result<bool, String> {
    Ok(run(exec, root, args, &[]).await?.code == Some(0))
}

pub(crate) async fn linked_worktree_roots(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<Vec<PathBuf>>, String> {
    let Some(raw) = optional(exec, root, &["worktree", "list", "--porcelain", "-z"]).await? else {
        return Ok(None);
    };
    Ok(Some(parse_linked_worktree_roots(&raw)))
}

/// Main repository root shared by a checkout and all of its linked worktrees.
/// The command is observational and runs through the same bounded/helper-free
/// profile as the rest of the metadata probes.
pub(crate) async fn common_repository_root(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(raw) = optional(
        exec,
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?
    else {
        return Ok(None);
    };
    let common_dir = PathBuf::from(raw.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        root.join(common_dir)
    };
    Ok(common_dir
        .file_name()
        .is_some_and(|name| name == ".git")
        .then(|| common_dir.parent().map(Path::to_path_buf))
        .flatten())
}

fn parse_linked_worktree_roots(raw: &str) -> Vec<PathBuf> {
    raw.split("\0\0")
        .filter_map(|record| {
            let mut root = None;
            for field in record.split('\0') {
                if field.starts_with("prunable") {
                    return None;
                }
                if root.is_none() {
                    root = field.strip_prefix("worktree ").map(PathBuf::from);
                }
            }
            root
        })
        .collect()
}

async fn run(
    exec: &dyn Executor,
    root: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<exec_core::ExecOutput, String> {
    let arguments = args.iter().map(|arg| shell_word(arg)).collect::<Vec<_>>();
    exec.exec(
        &protected_git_command(&arguments.join(" "), env),
        root,
        COMMAND_TIMEOUT,
        &CancellationToken::new(),
    )
    .await
}

/// Build a non-interactive Git command using the syntax of the executor's
/// platform shell. Every Git caller shares this path so Windows never receives
/// POSIX-only `NAME=value command` prefixes or single-quote escaping.
pub(crate) fn protected_git_command(arguments: &str, env: &[(&str, &str)]) -> String {
    let mut environment = env.to_vec();
    environment.extend([("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0")]);
    format!(
        "{}git --no-optional-locks -c {} -c credential.helper= -c core.fsmonitor=false {arguments}",
        shell_environment_prefix(&environment),
        shell_word(&format!("core.hooksPath={DISABLED_HOOKS_PATH}")),
    )
}

pub(crate) fn shell_word(value: &str) -> String {
    shell_word_for(exec_core::scripted_shell_kind(), value)
}

fn shell_word_for(kind: exec_core::ShellKind, value: &str) -> String {
    if shell_word_is_unquoted_safe(kind, value) {
        return value.to_string();
    }
    match kind {
        exec_core::ShellKind::Posix => format!("'{}'", value.replace('\'', "'\\''")),
        exec_core::ShellKind::PowerShell => format!("'{}'", value.replace('\'', "''")),
        exec_core::ShellKind::Cmd => format!("\"{}\"", cmd_quoted(value)),
    }
}

fn shell_word_is_unquoted_safe(kind: exec_core::ShellKind, value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            let common = byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'+' | b',');
            let shell_specific = match kind {
                exec_core::ShellKind::Posix => matches!(byte, b'@' | b'%'),
                exec_core::ShellKind::PowerShell => byte == b'%',
                exec_core::ShellKind::Cmd => false,
            };
            common || shell_specific
        })
}

fn shell_environment_prefix(env: &[(&str, &str)]) -> String {
    shell_environment_prefix_for(exec_core::scripted_shell_kind(), env)
}

fn shell_environment_prefix_for(kind: exec_core::ShellKind, env: &[(&str, &str)]) -> String {
    match kind {
        exec_core::ShellKind::Posix => {
            env.iter()
                .map(|(name, value)| format!("{name}={}", shell_word_for(kind, value)))
                .collect::<Vec<_>>()
                .join(" ")
                + " "
        }
        exec_core::ShellKind::PowerShell => env
            .iter()
            .map(|(name, value)| format!("$env:{name} = {};", shell_word_for(kind, value)))
            .collect::<String>(),
        exec_core::ShellKind::Cmd => env
            .iter()
            .map(|(name, value)| format!("set \"{name}={}\" && ", cmd_quoted(value)))
            .collect(),
    }
}

fn cmd_quoted(value: &str) -> String {
    value
        .replace('^', "^^")
        .replace('%', "%%")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_shell_words_preserve_metadata_arguments() {
        let word = |value| shell_word_for(exec_core::ShellKind::Posix, value);
        assert_eq!(word("status"), "status");
        assert_eq!(word("--format=%H%x00%P"), "--format=%H%x00%P");
        assert_eq!(word("path with spaces"), "'path with spaces'");
        assert_eq!(word("it's"), "'it'\\''s'");
    }

    #[test]
    fn powershell_words_and_environment_prefix_escape_for_windows() {
        assert_eq!(
            shell_word_for(exec_core::ShellKind::PowerShell, "C:/O'Brien/repo"),
            "'C:/O''Brien/repo'"
        );
        assert_eq!(
            shell_environment_prefix_for(
                exec_core::ShellKind::PowerShell,
                &[("GIT_INDEX_FILE", "C:/work tree/index")],
            ),
            "$env:GIT_INDEX_FILE = 'C:/work tree/index';"
        );
    }

    #[test]
    fn cmd_prefix_disables_expansion_of_percent_in_environment_values() {
        assert_eq!(
            shell_environment_prefix_for(
                exec_core::ShellKind::Cmd,
                &[("GIT_INDEX_FILE", r"C:\work%USERPROFILE%\index")],
            ),
            r#"set "GIT_INDEX_FILE=C:\work%%USERPROFILE%%\index" && "#
        );
    }

    #[test]
    fn parses_nul_terminated_worktree_records_without_shell_splitting_paths() {
        let raw = concat!(
            "worktree /repo/main\0HEAD abc\0branch refs/heads/main\0\0",
            "worktree /repo/linked worktree\0HEAD def\0detached\0\0",
            "worktree /repo/stale\0HEAD 000\0detached\0prunable missing\0\0",
        );
        assert_eq!(
            parse_linked_worktree_roots(raw),
            vec![
                PathBuf::from("/repo/main"),
                PathBuf::from("/repo/linked worktree")
            ]
        );
    }

    #[tokio::test]
    async fn resolves_the_same_common_root_from_main_and_linked_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        std::fs::create_dir(&main).unwrap();
        let run = |cwd: &Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&main, &["init", "-q", "--initial-branch=main"]);
        run(&main, &["config", "user.name", "Clark Test"]);
        run(&main, &["config", "user.email", "clark@example.com"]);
        std::fs::write(main.join("tracked"), "one").unwrap();
        run(&main, &["add", "tracked"]);
        run(&main, &["commit", "-qm", "initial"]);
        run(
            &main,
            &[
                "worktree",
                "add",
                "--detach",
                "-q",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );

        let expected = main.canonicalize().unwrap();
        assert_eq!(
            common_repository_root(&crate::exec::LocalExecutor, &main)
                .await
                .unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            common_repository_root(&crate::exec::LocalExecutor, &linked)
                .await
                .unwrap(),
            Some(expected)
        );
    }
}
