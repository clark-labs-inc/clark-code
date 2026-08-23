use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Serialize;
use tauri::State;
use tokio::{process::Command, time::timeout};

use crate::{
    commands::{project_executor, RemoteArg},
    AppState,
};

pub(crate) mod managed;
mod remote_ops;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_REMOTE: &str = "origin";
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranch {
    pub name: String,
    pub checkout_path: Option<String>,
}

fn validate_name(name: &str) -> Result<&str, String> {
    let clean = name.trim();
    if clean.is_empty() {
        return Err("Enter a worktree name.".into());
    }
    if clean.len() > 48 {
        return Err("Worktree names must be 48 characters or fewer.".into());
    }
    if clean.starts_with('-')
        || !clean
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("Use letters, numbers, hyphens, or underscores.".into());
    }
    Ok(clean)
}

fn destination_for(repo_root: &Path, name: &str) -> Result<PathBuf, String> {
    let parent = repo_root
        .parent()
        .ok_or_else(|| "The repository has no parent folder for a sibling worktree.".to_string())?;
    let repo_name = repo_root
        .file_name()
        .and_then(|part| part.to_str())
        .filter(|part| !part.is_empty())
        .ok_or_else(|| "The repository folder has no usable name.".to_string())?;
    Ok(parent.join(format!("{repo_name}-{name}")))
}

async fn git_output(cwd: &Path, args: Vec<OsString>, action: &str) -> Result<String, String> {
    git_output_with_timeout(cwd, args, action, GIT_TIMEOUT).await
}

/// Run Git with the same non-interactive, hook-free containment as every other
/// project action, but let latency-sensitive discovery use a shorter deadline.
/// A stale remote must never make opening a new session feel hung.
async fn git_output_with_timeout(
    cwd: &Path,
    args: Vec<OsString>,
    action: &str,
    deadline: Duration,
) -> Result<String, String> {
    let mut command = Command::new("git");
    let hooks_path = format!("core.hooksPath={DISABLED_HOOKS_PATH}");
    command
        .current_dir(cwd)
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            &hooks_path,
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    exec_core::isolate_process_group(&mut command);
    let child = command
        .spawn()
        .map_err(|error| format!("{action}: failed to start git: {error}"))?;
    let output = timeout(deadline, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "{action}: git timed out after {} seconds",
                deadline.as_secs()
            )
        })?
        .map_err(|error| format!("{action}: failed to wait for git: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("{action}: git exited with {}", output.status)
        } else {
            format!("{action}: {detail}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn repository_root(project_path: &str) -> Result<PathBuf, String> {
    let source = PathBuf::from(project_path.trim())
        .canonicalize()
        .map_err(|error| format!("Project folder is unavailable: {error}"))?;
    if !source.is_dir() {
        return Err("Project path is not a folder.".into());
    }

    let root = git_output(
        &source,
        vec!["rev-parse".into(), "--show-toplevel".into()],
        "Find repository root",
    )
    .await?;
    PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("Repository root is unavailable: {error}"))
}

fn parse_remote_default(output: &str) -> Result<(String, String), String> {
    let branch = output.lines().find_map(|line| {
        let reference = line.strip_prefix("ref: refs/heads/")?;
        let (branch, destination) = reference.split_once(char::is_whitespace)?;
        (destination.trim() == "HEAD" && !branch.is_empty()).then(|| branch.to_string())
    });
    let commit = output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let candidate = fields.next()?;
        let destination = fields.next()?;
        (destination == "HEAD"
            && matches!(candidate.len(), 40 | 64)
            && candidate.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| candidate.to_string())
    });
    match (branch, commit) {
        (Some(branch), Some(commit)) => Ok((branch, commit)),
        _ => Err(format!(
            "{DEFAULT_REMOTE} did not advertise a valid default branch."
        )),
    }
}

/// Fetch the advertised default-branch commit without updating FETCH_HEAD,
/// the current branch, or the selected checkout. Downloaded objects and the new
/// worktree branch are the only durable repository changes required to create
/// a checkout at the latest remote commit.
async fn fetch_latest_default(repo_root: &Path) -> Result<String, String> {
    let advertised = git_output(
        repo_root,
        vec![
            "ls-remote".into(),
            "--symref".into(),
            DEFAULT_REMOTE.into(),
            "HEAD".into(),
        ],
        "Find latest remote default branch",
    )
    .await?;
    let (branch, commit) = parse_remote_default(&advertised)?;
    git_output(
        repo_root,
        vec![
            "fetch".into(),
            "--quiet".into(),
            "--no-tags".into(),
            "--no-write-fetch-head".into(),
            DEFAULT_REMOTE.into(),
            commit.clone().into(),
        ],
        &format!("Fetch latest origin/{branch}"),
    )
    .await?;
    Ok(commit)
}

fn parse_branch_owners(output: &str) -> HashMap<String, String> {
    let mut owners = HashMap::new();
    let mut checkout_path = None;

    for field in output.split('\0') {
        if field.is_empty() {
            checkout_path = None;
        } else if let Some(path) = field.strip_prefix("worktree ") {
            checkout_path = Some(path.to_string());
        } else if let (Some(path), Some(branch)) = (
            checkout_path.as_ref(),
            field
                .strip_prefix("branch ")
                .and_then(|reference| reference.strip_prefix("refs/heads/")),
        ) {
            owners.insert(branch.to_string(), path.clone());
        }
    }

    owners
}

/// Local branches available to the selected checkout, including the checkout
/// that currently owns each branch. Exact refs keep the switch unambiguous and
/// let the caller open an existing checkout instead of asking Git to violate
/// its one-branch-per-worktree invariant.
#[tauri::command]
pub async fn project_branch_list(
    project_path: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectBranch>, String> {
    if remote.is_some() {
        let executor = project_executor(remote, state.inner()).await?;
        return remote_ops::branch_list(executor.as_ref(), &project_path).await;
    }
    local_branch_list(&project_path).await
}

pub(crate) async fn local_branch_list(project_path: &str) -> Result<Vec<ProjectBranch>, String> {
    let repo_root = repository_root(project_path).await?;
    let (branches, worktrees) = tokio::try_join!(
        git_output(
            &repo_root,
            vec![
                "for-each-ref".into(),
                "--format=%(refname:short)".into(),
                "refs/heads".into(),
            ],
            "List branches",
        ),
        git_output(
            &repo_root,
            vec![
                "worktree".into(),
                "list".into(),
                "--porcelain".into(),
                "-z".into(),
            ],
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

/// Switch a clean selected checkout to an existing local branch. Git receives
/// arguments directly (never through a shell).
///
/// The clean-tree gate is best-effort, not a guarantee: `git switch` carries
/// non-conflicting local modifications by design, so a file written between
/// the status check and the switch (an agent editing concurrently) rides
/// along, exactly as it would under plain git. Closing that window would
/// require stash-like machinery this product deliberately never uses.
#[tauri::command]
pub async fn project_branch_switch(
    project_path: String,
    branch: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if remote.is_some() {
        let executor = project_executor(remote, state.inner()).await?;
        return remote_ops::branch_switch(executor.as_ref(), &project_path, &branch).await;
    }
    local_branch_switch(&project_path, &branch).await
}

async fn local_branch_switch(project_path: &str, branch: &str) -> Result<(), String> {
    let repo_root = repository_root(project_path).await?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Choose a branch.".into());
    }
    let branches = local_branch_list(&repo_root.to_string_lossy()).await?;
    let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
        return Err(format!("Local branch {branch} no longer exists."));
    };

    let current = git_output(
        &repo_root,
        vec!["branch".into(), "--show-current".into()],
        "Read current branch",
    )
    .await?;
    if current == branch {
        return Ok(());
    }

    if let Some(owner) = target.checkout_path.as_deref() {
        let owner_path = Path::new(owner).canonicalize().map_err(|_| {
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
        &repo_root,
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "--untracked-files=normal".into(),
        ],
        "Check working tree",
    )
    .await?;
    if !status.is_empty() {
        return Err("Commit or remove local changes before switching branches.".into());
    }

    git_output(
        &repo_root,
        vec![
            "switch".into(),
            "--no-guess".into(),
            "--".into(),
            branch.into(),
        ],
        "Switch branch",
    )
    .await?;
    Ok(())
}

/// Create a durable sibling checkout from the latest advertised remote default branch.
/// The explicit name becomes both the folder suffix and a `agent/<name>` branch;
/// no shell is involved, and validation prevents either value becoming an option
/// or path traversal payload. The source checkout's HEAD, index, files, and
/// remote-tracking refs are left untouched.
#[tauri::command]
pub async fn project_worktree_create(
    project_path: String,
    name: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if remote.is_some() {
        let executor = project_executor(remote, state.inner()).await?;
        return remote_ops::worktree_create(executor.as_ref(), &project_path, &name).await;
    }
    local_worktree_create(&project_path, &name).await
}

async fn local_worktree_create(project_path: &str, name: &str) -> Result<String, String> {
    let clean_name = validate_name(name)?.to_string();
    let repo_root = repository_root(project_path).await?;
    let destination = destination_for(&repo_root, &clean_name)?;
    if destination.exists() {
        return Err(format!(
            "A folder already exists at {}. Choose another name.",
            destination.display()
        ));
    }

    let latest_default = fetch_latest_default(&repo_root).await?;
    let branch = format!("agent/{clean_name}");
    git_output(
        &repo_root,
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            destination.as_os_str().to_os_string(),
            latest_default.into(),
        ],
        "Create permanent worktree",
    )
    .await?;

    Ok(destination.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        destination_for, local_branch_list, local_branch_switch, local_worktree_create,
        parse_branch_owners, parse_remote_default,
        remote_ops::{
            branch_list as remote_branch_list, branch_switch as remote_branch_switch,
            worktree_create as remote_worktree_create,
        },
        validate_name,
    };
    use provider_local::LocalExecutor;
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_text(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn worktree_names_are_safe_path_and_branch_segments() {
        assert_eq!(validate_name("feature-123").unwrap(), "feature-123");
        assert!(validate_name("../escape").is_err());
        assert!(validate_name("feature/name").is_err());
        assert!(validate_name("-force").is_err());
        assert!(validate_name("with spaces").is_err());
    }

    #[test]
    fn destination_is_a_named_sibling_of_the_repository() {
        assert_eq!(
            destination_for(Path::new("/projects/project"), "menu").unwrap(),
            PathBuf::from("/projects/project-menu")
        );
    }

    #[test]
    fn remote_default_requires_a_symbolic_head_and_exact_commit() {
        assert_eq!(
            parse_remote_default(
                "ref: refs/heads/trunk\tHEAD\n0123456789abcdef0123456789abcdef01234567\tHEAD\n"
            )
            .unwrap(),
            (
                "trunk".to_string(),
                "0123456789abcdef0123456789abcdef01234567".to_string(),
            )
        );
        assert!(parse_remote_default("").is_err());
        assert!(parse_remote_default("not-a-commit\tHEAD\n").is_err());
        assert!(parse_remote_default("0123456789abcdef0123456789abcdef01234567\tHEAD\n").is_err());
    }

    #[test]
    fn parses_branch_owners_from_nul_delimited_porcelain() {
        let owners = parse_branch_owners(
            "worktree /repo\0HEAD abc123\0branch refs/heads/feature/local\0\0\
             worktree /repo-main\0HEAD def456\0branch refs/heads/main\0\0\
             worktree /repo-detached\0HEAD 012345\0detached\0\0",
        );

        assert_eq!(
            owners.get("feature/local").map(String::as_str),
            Some("/repo")
        );
        assert_eq!(owners.get("main").map(String::as_str), Some("/repo-main"));
        assert_eq!(owners.len(), 2);
    }

    #[tokio::test]
    async fn lists_and_switches_local_branches_only_when_the_checkout_is_clean() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "test@example.local"]);
        git(&repo, &["config", "user.name", "Agent Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["branch", "feature/context-bar"]);

        let branches = local_branch_list(&repo.to_string_lossy()).await.unwrap();
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            vec!["feature/context-bar", "main"]
        );
        assert_eq!(branches[0].checkout_path, None);
        let repo_path = repo.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            branches[1].checkout_path.as_deref(),
            Some(repo_path.as_str())
        );
        local_branch_switch(&repo.to_string_lossy(), "feature/context-bar")
            .await
            .unwrap();
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            "feature/context-bar"
        );

        std::fs::write(repo.join("README.md"), "dirty\n").unwrap();
        let error = local_branch_switch(&repo.to_string_lossy(), "main")
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "Commit or remove local changes before switching branches."
        );
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            "feature/context-bar"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "dirty\n"
        );
    }

    #[tokio::test]
    async fn executor_backed_branch_operations_run_on_the_target_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("remote-project");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "test@example.local"]);
        git(&repo, &["config", "user.name", "Agent Test"]);
        std::fs::write(repo.join("README.md"), "remote\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["branch", "feature/remote's-context"]);

        let branches = remote_branch_list(&LocalExecutor, &repo.to_string_lossy())
            .await
            .unwrap();
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            vec!["feature/remote's-context", "main"]
        );

        remote_branch_switch(
            &LocalExecutor,
            &repo.to_string_lossy(),
            "feature/remote's-context",
        )
        .await
        .unwrap();
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            "feature/remote's-context"
        );

        std::fs::write(repo.join("README.md"), "dirty remote\n").unwrap();
        let error = remote_branch_switch(&LocalExecutor, &repo.to_string_lossy(), "main")
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "Commit or remove local changes before switching branches."
        );
    }

    #[tokio::test]
    async fn executor_backed_worktree_uses_the_remote_default_branch() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote.git");
        let repo = temp.path().join("remote-project");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-q", "--initial-branch=trunk"]);
        git(
            temp.path(),
            &[
                "clone",
                "-q",
                remote.to_string_lossy().as_ref(),
                repo.to_string_lossy().as_ref(),
            ],
        );
        git(&repo, &["config", "user.email", "test@example.local"]);
        git(&repo, &["config", "user.name", "Agent Test"]);
        std::fs::write(repo.join("README.md"), "remote trunk\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["push", "-qu", "origin", "trunk"]);

        let created =
            remote_worktree_create(&LocalExecutor, &repo.to_string_lossy(), "remote-default")
                .await
                .unwrap();
        let created = PathBuf::from(created);
        assert_eq!(
            std::fs::read_to_string(created.join("README.md")).unwrap(),
            "remote trunk\n"
        );
        assert_eq!(
            git_text(&created, &["branch", "--show-current"]),
            "agent/remote-default"
        );
    }

    #[tokio::test]
    async fn reports_the_checkout_that_already_owns_a_branch() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        let main_checkout = temp.path().join("project-main");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "test@example.local"]);
        git(&repo, &["config", "user.name", "Agent Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["switch", "-qc", "feature/local"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                main_checkout.to_string_lossy().as_ref(),
                "main",
            ],
        );

        let branches = local_branch_list(&repo.to_string_lossy()).await.unwrap();
        let main = branches
            .iter()
            .find(|branch| branch.name == "main")
            .unwrap();
        let main_checkout_path = main_checkout
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            main.checkout_path.as_deref(),
            Some(main_checkout_path.as_str())
        );

        let error = local_branch_switch(&repo.to_string_lossy(), "main")
            .await
            .unwrap_err();
        assert_eq!(
            error,
            format!(
                "Branch main is already checked out at {}. Open that checkout instead.",
                main_checkout.canonicalize().unwrap().display()
            )
        );
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            "feature/local"
        );
    }

    #[tokio::test]
    async fn creates_a_real_sibling_worktree_from_remote_default_without_touching_source() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote.git");
        let repo = temp.path().join("project");
        let publisher = temp.path().join("publisher");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-q", "--initial-branch=trunk"]);
        let remote_arg = remote.to_string_lossy();
        let repo_arg = repo.to_string_lossy();
        git(temp.path(), &["clone", "-q", &remote_arg, &repo_arg]);
        git(&repo, &["config", "user.email", "test@example.local"]);
        git(&repo, &["config", "user.name", "Agent Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["branch", "-m", "trunk"]);
        git(&repo, &["push", "-qu", "origin", "trunk"]);
        git(&repo, &["switch", "-qc", "feature/local"]);

        let publisher_arg = publisher.to_string_lossy();
        git(temp.path(), &["clone", "-q", &remote_arg, &publisher_arg]);
        git(&publisher, &["config", "user.email", "test@example.local"]);
        git(&publisher, &["config", "user.name", "Agent Test"]);
        std::fs::write(publisher.join("README.md"), "latest trunk\n").unwrap();
        git(&publisher, &["add", "README.md"]);
        git(&publisher, &["commit", "-qm", "advance trunk"]);
        git(&publisher, &["push", "-q", "origin", "trunk"]);
        let latest_default = git_text(&publisher, &["rev-parse", "HEAD"]);

        std::fs::write(repo.join("README.md"), "local dirty change\n").unwrap();
        std::fs::write(repo.join("notes.txt"), "untracked\n").unwrap();
        let source_branch = git_text(&repo, &["branch", "--show-current"]);
        let source_head = git_text(&repo, &["rev-parse", "HEAD"]);
        let source_status = git_text(&repo, &["status", "--short"]);
        let source_origin_default = git_text(&repo, &["rev-parse", "refs/remotes/origin/trunk"]);

        let created = local_worktree_create(&repo.to_string_lossy(), "sidebar-menu")
            .await
            .unwrap();
        let created = PathBuf::from(created);

        assert_eq!(
            created,
            temp.path()
                .canonicalize()
                .unwrap()
                .join("project-sidebar-menu")
        );
        assert_eq!(
            std::fs::read_to_string(created.join("README.md")).unwrap(),
            "latest trunk\n"
        );
        assert_eq!(
            git_text(&created, &["branch", "--show-current"]),
            "agent/sidebar-menu"
        );
        assert_eq!(git_text(&created, &["rev-parse", "HEAD"]), latest_default);

        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            source_branch
        );
        assert_eq!(git_text(&repo, &["rev-parse", "HEAD"]), source_head);
        assert_eq!(git_text(&repo, &["status", "--short"]), source_status);
        assert_eq!(
            git_text(&repo, &["rev-parse", "refs/remotes/origin/trunk"]),
            source_origin_default
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "local dirty change\n"
        );
    }
}
