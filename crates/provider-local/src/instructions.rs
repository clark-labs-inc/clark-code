//! Executor-backed project instruction discovery.
//!
//! Instructions are loaded from the repository root down to the selected cwd,
//! so nested guidance wins by appearing later. Remote sessions use the same
//! executor filesystem instead of accidentally consulting the desktop disk.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::exec::Executor;

const MAX_TOTAL_BYTES: usize = 32_000;
const MAX_FILE_BYTES: usize = 8_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectInstructions {
    pub text: String,
    pub sources: Vec<String>,
    digest: String,
}

impl ProjectInstructions {
    pub fn render(&self) -> String {
        format!("# Project context\n{}", self.text)
    }

    pub fn changed_from(&self, previous: &Self) -> bool {
        self.digest != previous.digest
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

    // README remains useful product context, but unlike instruction files it
    // is loaded only from the selected checkout root.
    if remaining > 0 {
        let path = cwd.join("README.md");
        if let Ok(bytes) = exec.read(&path).await {
            append_source(&mut text, &mut sources, &mut remaining, &path, bytes);
        }
    }

    if sources.is_empty() {
        return Ok(None);
    }
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    Ok(Some(ProjectInstructions {
        text,
        sources,
        digest,
    }))
}

pub(crate) fn refresh_context(
    previous: Option<&ProjectInstructions>,
    current: Option<&ProjectInstructions>,
) -> Option<String> {
    let changed = match (previous, current) {
        (None, None) => false,
        (Some(previous), Some(current)) => current.changed_from(previous),
        _ => true,
    };
    if !changed {
        return None;
    }
    Some(match current {
        Some(current) => format!(
            "[runtime context — project instructions refreshed from the filesystem]\n{}",
            current.render()
        ),
        None => "[runtime context — project instructions refreshed from the filesystem]\n\
The previously provided project instructions no longer apply."
            .to_string(),
    })
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
    async fn refresh_reports_changes_and_removal_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("AGENTS.md");
        std::fs::write(&path, "first").unwrap();
        let first = load(&LocalExecutor, temp.path()).await.unwrap().unwrap();
        assert!(refresh_context(Some(&first), Some(&first)).is_none());

        std::fs::write(&path, "second").unwrap();
        let second = load(&LocalExecutor, temp.path()).await.unwrap().unwrap();
        assert!(refresh_context(Some(&first), Some(&second))
            .unwrap()
            .contains("second"));
        assert!(refresh_context(Some(&second), None)
            .unwrap()
            .contains("no longer apply"));
    }
}
