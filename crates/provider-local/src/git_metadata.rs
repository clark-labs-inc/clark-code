//! Bounded Git metadata commands used by repository discovery and context.
//!
//! These commands are observational. They must not acquire optional locks,
//! execute repository-selected hooks/helpers, or wait for interactive auth.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::exec::Executor;

const COMMAND_TIMEOUT: Duration = if cfg!(windows) {
    // Windows ARM VMs commonly run Git's x64 distribution under emulation.
    // Metadata remains bounded, but five seconds is too short under parallel
    // test/build load and turns valid repositories into false negatives.
    Duration::from_secs(30)
} else {
    Duration::from_secs(5)
};
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Read-only identity for the checkout selected by a local or remote executor.
///
/// This deliberately exposes only Git-derived metadata. Callers that need
/// product-specific activity signals can compose those independently without
/// rebuilding an unprotected shell pipeline around Git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCheckoutContext {
    pub branch: String,
    pub detached: bool,
    pub is_worktree: bool,
    pub worktree_root: PathBuf,
}

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

/// Inspect a checkout through the same bounded, hook-free Git profile used by
/// repository discovery. A non-Git directory is normal and returns `None`.
pub async fn inspect_git_checkout(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<GitCheckoutContext>, String> {
    let Some(inside) = optional(exec, root, &["rev-parse", "--is-inside-work-tree"]).await? else {
        return Ok(None);
    };
    if inside != "true" {
        return Ok(None);
    }

    let (worktree_root, branch, git_dir, common_dir) = tokio::try_join!(
        optional(exec, root, &["rev-parse", "--show-toplevel"]),
        optional(exec, root, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        optional(
            exec,
            root,
            &["rev-parse", "--path-format=absolute", "--git-dir"],
        ),
        optional(
            exec,
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ),
    )?;
    let (Some(worktree_root), Some(git_dir), Some(common_dir)) =
        (worktree_root, git_dir, common_dir)
    else {
        return Ok(None);
    };

    let worktree_root = PathBuf::from(worktree_root.trim());
    let worktree_root = exec
        .canonicalize(&worktree_root)
        .await
        .unwrap_or(worktree_root);
    let git_dir = canonical_git_path(exec, root, &git_dir).await;
    let common_dir = canonical_git_path(exec, root, &common_dir).await;
    let branch = branch.filter(|value| !value.is_empty());
    let detached = branch.is_none();
    let branch = if let Some(branch) = branch {
        branch
    } else {
        let Some(revision) = optional(exec, root, &["rev-parse", "--short", "HEAD"]).await? else {
            return Ok(None);
        };
        revision
    };

    Ok(Some(GitCheckoutContext {
        branch,
        detached,
        is_worktree: git_dir != common_dir,
        worktree_root,
    }))
}

/// Read a porcelain working-tree snapshot under the protected Git profile.
/// The distinction between a clean tree (`Some("")`) and an unavailable Git
/// checkout (`None`) is retained for activity callers.
pub async fn git_working_tree_status(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<String>, String> {
    optional(
        exec,
        root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )
    .await
}

async fn canonical_git_path(exec: &dyn Executor, root: &Path, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value.trim());
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    exec.canonicalize(&candidate).await.unwrap_or(candidate)
}

/// Main repository root shared by a checkout and all of its linked worktrees.
/// The command is observational and runs through the same bounded/helper-free
/// profile as the rest of the metadata probes.
pub(crate) async fn common_repository_root(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<PathBuf>, String> {
    let command_result = optional(
        exec,
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await;
    if let Ok(Some(raw)) = &command_result {
        let common_dir = PathBuf::from(raw.trim());
        let common_dir = if common_dir.is_absolute() {
            common_dir
        } else {
            root.join(common_dir)
        };
        let common_dir = exec.canonicalize(&common_dir).await.unwrap_or(common_dir);
        if let Some(repository_root) = repository_root_from_common_dir(&common_dir) {
            return Ok(Some(repository_root));
        }
    }

    // If a contained Git process cannot report the common directory, resolve
    // the standard `.git`/`commondir` files through the same policy-checked
    // executor instead of dropping repository-family identity from the
    // session.
    if let Some(repository_root) = common_repository_root_from_files(exec, root).await {
        return Ok(Some(repository_root));
    }
    command_result.map(|_| None)
}

fn repository_root_from_common_dir(common_dir: &Path) -> Option<PathBuf> {
    common_dir
        .file_name()
        .is_some_and(|name| name == ".git")
        .then(|| common_dir.parent().map(Path::to_path_buf))
        .flatten()
}

async fn common_repository_root_from_files(exec: &dyn Executor, root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let metadata = exec.metadata(&dot_git).await.ok()?;
    let common_dir = if metadata.is_dir {
        dot_git
    } else if !metadata.is_symlink {
        let contents = String::from_utf8(exec.read(&dot_git).await.ok()?).ok()?;
        let target = contents.trim().strip_prefix("gitdir:")?.trim();
        let git_dir = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            root.join(target)
        };
        let common_file = git_dir.join("commondir");
        let common = String::from_utf8(exec.read(&common_file).await.ok()?).ok()?;
        let common = common.trim();
        if Path::new(common).is_absolute() {
            PathBuf::from(common)
        } else {
            git_dir.join(common)
        }
    } else {
        return None;
    };
    let common_dir = exec.canonicalize(&common_dir).await.ok()?;
    repository_root_from_common_dir(&common_dir)
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

/// Quote a host path for Git after removing Windows' verbatim-path prefix.
///
/// `std::fs::canonicalize` returns paths such as `\\?\C:\repo` on Windows.
/// Git for Windows does not recognize that spelling as a local clone source
/// and instead parses its colon as scp-style remote syntax. Forward slashes
/// preserve the same path while keeping it unambiguously local to Git.
pub(crate) fn shell_path_word(path: &Path) -> String {
    let rendered = path.to_string_lossy();
    #[cfg(windows)]
    let rendered = windows_git_path(&rendered);
    shell_word(&rendered)
}

#[cfg(any(windows, test))]
fn windows_git_path(value: &str) -> String {
    let local = if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("//{rest}")
    } else if let Some(rest) = value.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        value.to_string()
    };
    local.replace('\\', "/")
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
    fn canonical_windows_paths_are_rendered_as_local_git_paths() {
        assert_eq!(
            windows_git_path(r"\\?\C:\Agent Desktop QA\repo"),
            "C:/Agent Desktop QA/repo"
        );
        assert_eq!(
            windows_git_path(r"\\?\UNC\server\share\repo"),
            "//server/share/repo"
        );
        assert_eq!(windows_git_path(r"C:\repo"), "C:/repo");
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
        run(&main, &["config", "user.name", "Agent Test"]);
        run(&main, &["config", "user.email", "agent@example.com"]);
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
        assert_eq!(
            common_repository_root_from_files(&crate::exec::LocalExecutor, &linked).await,
            Some(main.canonicalize().unwrap())
        );
    }
}
