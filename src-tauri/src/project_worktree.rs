use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{process::Command, time::timeout};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Create a durable sibling checkout from the selected project's current HEAD.
/// The explicit name becomes both the folder suffix and a `clark/<name>` branch;
/// no shell is involved, and validation prevents either value becoming an option
/// or path traversal payload.
#[tauri::command]
pub async fn project_worktree_create(project_path: String, name: String) -> Result<String, String> {
    let clean_name = validate_name(&name)?.to_string();
    let source = PathBuf::from(project_path.trim())
        .canonicalize()
        .map_err(|error| format!("Project folder is unavailable: {error}"))?;
    if !source.is_dir() {
        return Err("Project path is not a folder.".into());
    }

    let repo_root_raw = git_output(
        &source,
        vec!["rev-parse".into(), "--show-toplevel".into()],
        "Find repository root",
    )
    .await?;
    let repo_root = PathBuf::from(repo_root_raw)
        .canonicalize()
        .map_err(|error| format!("Repository root is unavailable: {error}"))?;
    let destination = destination_for(&repo_root, &clean_name)?;
    if destination.exists() {
        return Err(format!(
            "A folder already exists at {}. Choose another name.",
            destination.display()
        ));
    }

    let branch = format!("clark/{clean_name}");
    git_output(
        &source,
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            destination.as_os_str().to_os_string(),
            "HEAD".into(),
        ],
        "Create permanent worktree",
    )
    .await?;

    Ok(destination.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{destination_for, project_worktree_create, validate_name};
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

    #[tokio::test]
    async fn creates_a_real_sibling_worktree_from_current_head() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "test@clark.local"]);
        git(&repo, &["config", "user.name", "Clark Test"]);
        std::fs::write(repo.join("README.md"), "original\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);

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
            "original\n"
        );
        let branch = Command::new("git")
            .current_dir(&created)
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&branch.stdout).trim(),
            "clark/sidebar-menu"
        );
    }
}
