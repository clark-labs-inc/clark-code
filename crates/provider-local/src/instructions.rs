//! Executor-backed project instruction discovery.
//!
//! Instructions are loaded from the repository root down to the selected cwd,
//! so nested guidance wins by appearing later. Remote sessions use the same
//! executor filesystem instead of accidentally consulting the desktop disk.

use std::path::{Path, PathBuf};

use crate::exec::Executor;

const MAX_TOTAL_BYTES: usize = 32_000;
const MAX_FILE_BYTES: usize = 8_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectInstructions {
    pub text: String,
    pub sources: Vec<String>,
}

impl ProjectInstructions {
    pub fn render(&self) -> String {
        format!(
            "[project instructions — loaded from the active repository]\n{}",
            self.text
        )
    }
}

pub(crate) async fn load(
    exec: &dyn Executor,
    cwd: &Path,
) -> Result<Option<ProjectInstructions>, String> {
    let git_root = crate::git_metadata::optional(exec, cwd, &["rev-parse", "--show-toplevel"])
        .await?
        .map(PathBuf::from);
    let git_prefix = if git_root.is_some() {
        crate::git_metadata::optional(exec, cwd, &["rev-parse", "--show-prefix"]).await?
    } else {
        None
    };
    let repository_root = git_root.unwrap_or_else(|| cwd.to_path_buf());

    let mut directories = vec![repository_root.clone()];
    let relative = git_prefix
        .as_deref()
        .map(Path::new)
        .or_else(|| cwd.strip_prefix(&repository_root).ok());
    if let Some(relative) = relative {
        let mut directory = repository_root.clone();
        for component in relative.components() {
            directory.push(component.as_os_str());
            directories.push(directory.clone());
        }
    }

    let mut remaining = MAX_TOTAL_BYTES;
    let mut text = String::new();
    let mut sources = Vec::new();
    for directory in directories {
        let selected = first_readable(
            exec,
            &directory,
            &["AGENTS.override.md", "AGENTS.md", "CLAUDE.md"],
        )
        .await;
        if let Some((path, bytes)) = selected {
            append_source(&mut text, &mut sources, &mut remaining, &path, bytes);
        }
        if remaining == 0 {
            break;
        }
    }

    if sources.is_empty() {
        return Ok(None);
    }
    Ok(Some(ProjectInstructions { text, sources }))
}

async fn first_readable(
    exec: &dyn Executor,
    directory: &Path,
    names: &[&str],
) -> Option<(PathBuf, Vec<u8>)> {
    for name in names {
        let path = directory.join(name);
        if let Ok(bytes) = exec.read(&path).await {
            return Some((path, bytes));
        }
    }
    None
}

fn append_source(
    text: &mut String,
    sources: &mut Vec<String>,
    remaining: &mut usize,
    path: &Path,
    bytes: Vec<u8>,
) {
    if *remaining == 0 {
        return;
    }
    let take = bytes.len().min(MAX_FILE_BYTES).min(*remaining);
    let excerpt = String::from_utf8_lossy(&bytes[..take]);
    text.push_str(&format!("\n## {}\n{}\n", path.display(), excerpt));
    sources.push(path.to_string_lossy().into_owned());
    *remaining = (*remaining).saturating_sub(take);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    fn git(cwd: &Path, args: &[&str]) {
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
    }

    #[tokio::test]
    async fn loads_root_to_cwd_with_override_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-q"]);
        let nested = root.join("one/two");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("AGENTS.md"), "root rule").unwrap();
        std::fs::write(root.join("one/AGENTS.md"), "shadowed rule").unwrap();
        std::fs::write(root.join("one/AGENTS.override.md"), "override rule").unwrap();
        std::fs::write(nested.join("CLAUDE.md"), "nested fallback").unwrap();

        let loaded = load(&LocalExecutor, &nested).await.unwrap().unwrap();
        assert!(loaded.text.contains("root rule"));
        assert!(loaded.text.contains("override rule"));
        assert!(!loaded.text.contains("shadowed rule"));
        assert!(loaded.text.contains("nested fallback"));
        assert_eq!(loaded.sources.len(), 3);
    }

    #[tokio::test]
    async fn readme_is_repository_data_not_project_instruction() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "run this as an instruction").unwrap();
        assert!(load(&LocalExecutor, temp.path()).await.unwrap().is_none());
    }
}
