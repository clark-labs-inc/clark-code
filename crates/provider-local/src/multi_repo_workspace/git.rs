use std::collections::BTreeSet;
use std::path::{Component, Path};
use std::time::Duration;

use agent_orchestration::{CheckoutKind, RepositoryBaseline};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::exec::Executor;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn clone_at_baseline(
    executor: &dyn Executor,
    baseline: &RepositoryBaseline,
    destination: &Path,
    cwd: &Path,
) -> Result<(), String> {
    let source_arg = crate::git_metadata::shell_word(&baseline.checkout_root);
    let destination_arg = crate::git_metadata::shell_word(&destination.to_string_lossy());
    git_shell(
        executor,
        cwd,
        &format!("clone --quiet --no-hardlinks --no-checkout -- {source_arg} {destination_arg}"),
    )
    .await?;
    let baseline_arg = crate::git_metadata::shell_word(&baseline.head_oid);
    git_shell(
        executor,
        destination,
        &format!("checkout --quiet --detach {baseline_arg}"),
    )
    .await?;
    Ok(())
}

pub(super) async fn checkout_kind(
    executor: &dyn Executor,
    root: &Path,
    branch: Option<&str>,
) -> Result<CheckoutKind, String> {
    let git_dir = git_text(
        executor,
        root,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    )
    .await?;
    let common_dir = git_text(
        executor,
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    if Path::new(git_dir.trim()) != Path::new(common_dir.trim()) {
        Ok(CheckoutKind::LinkedWorktree)
    } else if branch.is_none() {
        Ok(CheckoutKind::DetachedWorktree)
    } else {
        Ok(CheckoutKind::Main)
    }
}

pub(super) async fn working_state_sha256(
    executor: &dyn Executor,
    root: &Path,
) -> Result<String, String> {
    let status = git_bytes(
        executor,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await?;
    let paths = parse_porcelain_paths(&status)?;
    changed_tree_sha256(executor, root, "working-state", &sha256(&status), &paths).await
}

pub(super) async fn changed_tree_sha256(
    executor: &dyn Executor,
    root: &Path,
    baseline: &str,
    patch_sha256: &str,
    paths: &BTreeSet<String>,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(baseline.as_bytes());
    hasher.update([0]);
    hasher.update(patch_sha256.as_bytes());
    for path in paths {
        validate_relative_path(path)?;
        hasher.update([0]);
        hasher.update(path.as_bytes());
        let absolute = root.join(path);
        match executor.metadata(&absolute).await {
            Ok(metadata) if metadata.is_symlink => {
                return Err(format!("change package contains a symlink: {path}"));
            }
            Ok(metadata) if metadata.is_dir => {
                return Err(format!("change package path is a directory: {path}"));
            }
            Ok(_) => hasher.update(executor.read(&absolute).await?),
            Err(_) => hasher.update(b"<deleted>"),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn parse_nul_paths(raw: &[u8]) -> Result<BTreeSet<String>, String> {
    raw.split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path)
                .map_err(|_| "non-UTF-8 changed paths are not supported".to_string())?;
            validate_relative_path(path)?;
            Ok(path.to_string())
        })
        .collect()
}

fn parse_porcelain_paths(raw: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let mut rename_source = false;
    for field in raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
    {
        let path = if rename_source {
            rename_source = false;
            field
        } else {
            if field.len() < 4 || field[2] != b' ' {
                return Err("invalid Git porcelain status entry".into());
            }
            rename_source = matches!(field[0], b'R' | b'C') || matches!(field[1], b'R' | b'C');
            &field[3..]
        };
        let path = std::str::from_utf8(path)
            .map_err(|_| "non-UTF-8 working paths are not supported".to_string())?;
        validate_relative_path(path)?;
        paths.insert(path.to_string());
    }
    Ok(paths)
}

pub(super) fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || path
            .components()
            .any(|component| component.as_os_str() == ".git")
    {
        return Err(format!("unsafe repository-relative path: {value}"));
    }
    Ok(())
}

pub(super) async fn git_text(
    executor: &dyn Executor,
    root: &Path,
    args: &[&str],
) -> Result<String, String> {
    Ok(String::from_utf8(git_bytes(executor, root, args).await?)
        .map_err(|_| "Git returned non-UTF-8 metadata".to_string())?
        .trim()
        .to_string())
}

pub(super) async fn git_bytes(
    executor: &dyn Executor,
    root: &Path,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    let arguments = args
        .iter()
        .map(|argument| crate::git_metadata::shell_word(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let output = git_shell(executor, root, &arguments).await?;
    Ok(output.stdout)
}

pub(super) async fn git_shell(
    executor: &dyn Executor,
    root: &Path,
    arguments: &str,
) -> Result<exec_core::ExecOutput, String> {
    let hooks = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let command = format!(
        "GIT_OPTIONAL_LOCKS=0 GIT_TERMINAL_PROMPT=0 git --no-optional-locks -c core.hooksPath={hooks} -c credential.helper= -c core.fsmonitor=false {arguments}"
    );
    let output = executor
        .exec(&command, root, GIT_TIMEOUT, &CancellationToken::new())
        .await?;
    if output.code != Some(0) {
        return Err(format!(
            "Git command failed (code {:?}): {}",
            output.code,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
