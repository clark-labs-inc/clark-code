use std::path::{Path, PathBuf};

use provider_local::Executor;
use tokio_util::sync::CancellationToken;

use super::{
    destination_for, parse_branch_owners, parse_remote_main, validate_name, ProjectBranch,
    DEFAULT_REMOTE, GIT_TIMEOUT,
};

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn git_output(
    executor: &dyn Executor,
    cwd: &Path,
    args: &[&str],
    action: &str,
) -> Result<String, String> {
    let command = std::iter::once("git".to_string())
        .chain(
            [
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
            ]
            .into_iter()
            .chain(args.iter().copied())
            .map(shell_quote),
        )
        .collect::<Vec<_>>()
        .join(" ");
    let output = executor
        .exec(&command, cwd, GIT_TIMEOUT, &CancellationToken::new())
        .await
        .map_err(|error| format!("{action}: {error}"))?;
    if output.code != Some(0) {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("{action}: git exited with {:?}", output.code)
        } else {
            format!("{action}: {detail}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn repository_root(executor: &dyn Executor, project_path: &str) -> Result<PathBuf, String> {
    let source = executor
        .canonicalize(Path::new(project_path.trim()))
        .await
        .map_err(|error| format!("Project folder is unavailable: {error}"))?;
    let metadata = executor
        .metadata(&source)
        .await
        .map_err(|error| format!("Project folder is unavailable: {error}"))?;
    if !metadata.is_dir {
        return Err("Project path is not a folder.".into());
    }
    let root = git_output(
        executor,
        &source,
        &["rev-parse", "--show-toplevel"],
        "Find repository root",
    )
    .await?;
    executor
        .canonicalize(Path::new(&root))
        .await
        .map_err(|error| format!("Repository root is unavailable: {error}"))
}

pub(super) async fn branch_list(
    executor: &dyn Executor,
    project_path: &str,
) -> Result<Vec<ProjectBranch>, String> {
    let repo_root = repository_root(executor, project_path).await?;
    let (branches, worktrees) = tokio::try_join!(
        git_output(
            executor,
            &repo_root,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
            "List branches",
        ),
        git_output(
            executor,
            &repo_root,
            &["worktree", "list", "--porcelain", "-z"],
            "Inspect worktrees",
        ),
    )?;
    let mut owners = parse_branch_owners(&worktrees);
    Ok(branches
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(|branch| ProjectBranch {
            name: branch.to_string(),
            checkout_path: owners.remove(branch),
        })
        .collect())
}

pub(super) async fn branch_switch(
    executor: &dyn Executor,
    project_path: &str,
    branch: &str,
) -> Result<(), String> {
    let repo_root = repository_root(executor, project_path).await?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Choose a branch.".into());
    }
    let branches = branch_list(executor, &repo_root.to_string_lossy()).await?;
    let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
        return Err(format!("Local branch {branch} no longer exists."));
    };
    let current = git_output(
        executor,
        &repo_root,
        &["branch", "--show-current"],
        "Read current branch",
    )
    .await?;
    if current == branch {
        return Ok(());
    }
    if let Some(owner) = target.checkout_path.as_deref() {
        let owner_path = executor.canonicalize(Path::new(owner)).await.map_err(|_| {
            format!(
                "Branch {branch} is registered to unavailable checkout {owner}. Resolve that Git worktree record before switching."
            )
        })?;
        if owner_path != repo_root {
            return Err(format!(
                "Branch {branch} is already checked out at {owner}. Open that checkout instead."
            ));
        }
    }
    let status = git_output(
        executor,
        &repo_root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
        "Check working tree",
    )
    .await?;
    if !status.is_empty() {
        return Err("Commit or remove local changes before switching branches.".into());
    }
    git_output(
        executor,
        &repo_root,
        &["switch", "--no-guess", "--", branch],
        "Switch branch",
    )
    .await?;
    Ok(())
}

pub(super) async fn worktree_create(
    executor: &dyn Executor,
    project_path: &str,
    name: &str,
) -> Result<String, String> {
    let clean_name = validate_name(name)?.to_string();
    let repo_root = repository_root(executor, project_path).await?;
    let destination = destination_for(&repo_root, &clean_name)?;
    if executor.metadata(&destination).await.is_ok() {
        return Err(format!(
            "A folder already exists at {}. Choose another name.",
            destination.display()
        ));
    }
    let advertised = git_output(
        executor,
        &repo_root,
        &["ls-remote", DEFAULT_REMOTE, "refs/heads/main"],
        "Find latest origin/main",
    )
    .await?;
    let commit = parse_remote_main(&advertised)?;
    git_output(
        executor,
        &repo_root,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            DEFAULT_REMOTE,
            &commit,
        ],
        "Fetch latest origin/main",
    )
    .await?;
    let branch = format!("agent/{clean_name}");
    git_output(
        executor,
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &destination.to_string_lossy(),
            &commit,
        ],
        "Create permanent worktree",
    )
    .await?;
    Ok(destination.to_string_lossy().into_owned())
}
