use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{process::Command, time::timeout};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_REMOTE: &str = "origin";
const DEFAULT_BRANCH: &str = "main";

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
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|error| format!("{action}: failed to start git: {error}"))?;
    let output = timeout(GIT_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("{action}: git timed out after 30 seconds"))?
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

fn parse_remote_main(output: &str) -> Result<String, String> {
    let expected_ref = format!("refs/heads/{DEFAULT_BRANCH}");
    let Some(line) = output.lines().find(|line| !line.trim().is_empty()) else {
        return Err(format!(
            "{DEFAULT_REMOTE} has no {DEFAULT_BRANCH} branch to start from."
        ));
    };
    let mut fields = line.split_whitespace();
    let commit = fields.next().unwrap_or_default();
    let remote_ref = fields.next().unwrap_or_default();
    if remote_ref != expected_ref
        || !matches!(commit.len(), 40 | 64)
        || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "{DEFAULT_REMOTE} returned an invalid {DEFAULT_BRANCH} commit."
        ));
    }
    Ok(commit.to_string())
}

/// Fetch the advertised main commit without updating FETCH_HEAD, origin/main,
/// the current branch, or the selected checkout. Downloaded objects and the new
/// worktree branch are the only durable repository changes required to create
/// a checkout at the latest remote commit.
async fn fetch_latest_main(repo_root: &Path) -> Result<String, String> {
    let advertised = git_output(
        repo_root,
        vec![
            "ls-remote".into(),
            DEFAULT_REMOTE.into(),
            format!("refs/heads/{DEFAULT_BRANCH}").into(),
        ],
        "Find latest origin/main",
    )
    .await?;
    let commit = parse_remote_main(&advertised)?;
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
        "Fetch latest origin/main",
    )
    .await?;
    Ok(commit)
}

/// Local branches available to the selected checkout. Returning only exact
/// refs/heads names keeps the switch command unambiguous and prevents Git's
/// remote-branch guessing from creating a branch as a side effect.
#[tauri::command]
pub async fn project_branch_list(project_path: String) -> Result<Vec<String>, String> {
    let repo_root = repository_root(&project_path).await?;
    let branches = git_output(
        &repo_root,
        vec![
            "for-each-ref".into(),
            "--format=%(refname:short)".into(),
            "refs/heads".into(),
        ],
        "List branches",
    )
    .await?;
    Ok(branches
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
        .collect())
}

/// Switch a clean selected checkout to an existing local branch. Git receives
/// arguments directly (never through a shell), and the clean-tree gate avoids
/// silently carrying edits from one branch to another.
#[tauri::command]
pub async fn project_branch_switch(project_path: String, branch: String) -> Result<(), String> {
    let repo_root = repository_root(&project_path).await?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Choose a branch.".into());
    }
    let branches = project_branch_list(repo_root.to_string_lossy().into_owned()).await?;
    if !branches.iter().any(|candidate| candidate == branch) {
        return Err(format!("Local branch {branch} no longer exists."));
    }

    let current = git_output(
        &repo_root,
        vec!["branch".into(), "--show-current".into()],
        "Read current branch",
    )
    .await?;
    if current == branch {
        return Ok(());
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

/// Create a durable sibling checkout from the latest advertised origin/main.
/// The explicit name becomes both the folder suffix and a `clark/<name>` branch;
/// no shell is involved, and validation prevents either value becoming an option
/// or path traversal payload. The source checkout's HEAD, index, files, and
/// remote-tracking refs are left untouched.
#[tauri::command]
pub async fn project_worktree_create(project_path: String, name: String) -> Result<String, String> {
    let clean_name = validate_name(&name)?.to_string();
    let repo_root = repository_root(&project_path).await?;
    let destination = destination_for(&repo_root, &clean_name)?;
    if destination.exists() {
        return Err(format!(
            "A folder already exists at {}. Choose another name.",
            destination.display()
        ));
    }

    let latest_main = fetch_latest_main(&repo_root).await?;
    let branch = format!("clark/{clean_name}");
    git_output(
        &repo_root,
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            destination.as_os_str().to_os_string(),
            latest_main.into(),
        ],
        "Create permanent worktree",
    )
    .await?;

    Ok(destination.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        destination_for, parse_remote_main, project_branch_list, project_branch_switch,
        project_worktree_create, validate_name,
    };
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
            destination_for(Path::new("/projects/clark"), "menu").unwrap(),
            PathBuf::from("/projects/clark-menu")
        );
    }

    #[test]
    fn remote_main_requires_an_exact_commit_and_ref() {
        assert_eq!(
            parse_remote_main("0123456789abcdef0123456789abcdef01234567\trefs/heads/main\n")
                .unwrap(),
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert!(parse_remote_main("").is_err());
        assert!(parse_remote_main("not-a-commit\trefs/heads/main\n").is_err());
        assert!(
            parse_remote_main("0123456789abcdef0123456789abcdef01234567\trefs/heads/trunk\n")
                .is_err()
        );
    }

    #[tokio::test]
    async fn lists_and_switches_local_branches_only_when_the_checkout_is_clean() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "test@clark.local"]);
        git(&repo, &["config", "user.name", "Clark Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["branch", "feature/context-bar"]);

        assert_eq!(
            project_branch_list(repo.to_string_lossy().into_owned())
                .await
                .unwrap(),
            vec!["feature/context-bar", "main"]
        );
        project_branch_switch(
            repo.to_string_lossy().into_owned(),
            "feature/context-bar".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            "feature/context-bar"
        );

        std::fs::write(repo.join("README.md"), "dirty\n").unwrap();
        let error = project_branch_switch(repo.to_string_lossy().into_owned(), "main".into())
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
    async fn creates_a_real_sibling_worktree_from_latest_main_without_touching_source() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote.git");
        let repo = temp.path().join("project");
        let publisher = temp.path().join("publisher");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-q", "--initial-branch=main"]);
        let remote_arg = remote.to_string_lossy();
        let repo_arg = repo.to_string_lossy();
        git(temp.path(), &["clone", "-q", &remote_arg, &repo_arg]);
        git(&repo, &["config", "user.email", "test@clark.local"]);
        git(&repo, &["config", "user.name", "Clark Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(&repo, &["push", "-qu", "origin", "main"]);
        git(&repo, &["switch", "-qc", "feature/local"]);

        let publisher_arg = publisher.to_string_lossy();
        git(temp.path(), &["clone", "-q", &remote_arg, &publisher_arg]);
        git(&publisher, &["config", "user.email", "test@clark.local"]);
        git(&publisher, &["config", "user.name", "Clark Test"]);
        std::fs::write(publisher.join("README.md"), "latest main\n").unwrap();
        git(&publisher, &["add", "README.md"]);
        git(&publisher, &["commit", "-qm", "advance main"]);
        git(&publisher, &["push", "-q", "origin", "main"]);
        let latest_main = git_text(&publisher, &["rev-parse", "HEAD"]);

        std::fs::write(repo.join("README.md"), "local dirty change\n").unwrap();
        std::fs::write(repo.join("notes.txt"), "untracked\n").unwrap();
        let source_branch = git_text(&repo, &["branch", "--show-current"]);
        let source_head = git_text(&repo, &["rev-parse", "HEAD"]);
        let source_status = git_text(&repo, &["status", "--short"]);
        let source_origin_main = git_text(&repo, &["rev-parse", "refs/remotes/origin/main"]);

        let created =
            project_worktree_create(repo.to_string_lossy().into_owned(), "sidebar-menu".into())
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
            "latest main\n"
        );
        assert_eq!(
            git_text(&created, &["branch", "--show-current"]),
            "clark/sidebar-menu"
        );
        assert_eq!(git_text(&created, &["rev-parse", "HEAD"]), latest_main);

        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]),
            source_branch
        );
        assert_eq!(git_text(&repo, &["rev-parse", "HEAD"]), source_head);
        assert_eq!(git_text(&repo, &["status", "--short"]), source_status);
        assert_eq!(
            git_text(&repo, &["rev-parse", "refs/remotes/origin/main"]),
            source_origin_main
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "local dirty change\n"
        );
    }
}
