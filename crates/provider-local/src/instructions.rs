//! Executor-backed project instruction discovery.
//!
//! Instructions are loaded from the repository root down to the selected cwd,
//! so nested guidance wins by appearing later. Remote sessions use the same
//! executor filesystem instead of accidentally consulting the desktop disk.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::exec::Executor;
use crate::markdown_frontmatter::resolve_home;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionScope {
    Personal,
    Project,
    Nested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionOrigin {
    Clark,
    Compatible,
    Claude,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionProvenance {
    pub path: String,
    pub scope: InstructionScope,
    pub origin: InstructionOrigin,
    pub precedence: usize,
    pub bytes_loaded: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectInstructions {
    pub text: String,
    pub sources: Vec<InstructionProvenance>,
}

impl ProjectInstructions {
    pub fn render(&self) -> String {
        format!(
            "[runtime instructions — loaded with explicit personal/project/nested provenance]\n{}",
            self.text
        )
    }
}

pub async fn load(exec: &dyn Executor, cwd: &Path) -> Result<Option<ProjectInstructions>, String> {
    let home = resolve_home(exec, cwd).await;
    load_with_home(exec, cwd, home.as_deref()).await
}

async fn load_with_home(
    exec: &dyn Executor,
    cwd: &Path,
    home: Option<&Path>,
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

    let mut text = String::new();
    let mut sources = Vec::new();
    let mut precedence = 0usize;

    if let Some(home) = home {
        if let Some((path, bytes, origin)) = first_personal_readable(exec, home).await {
            append_source(
                &mut text,
                &mut sources,
                &path,
                bytes,
                SourceDescriptor {
                    scope: InstructionScope::Personal,
                    origin,
                    precedence,
                },
            );
            precedence += 1;
        }
    }

    for (index, directory) in directories.into_iter().enumerate() {
        let selected = first_readable(
            exec,
            &directory,
            &["AGENTS.override.md", "AGENTS.md", "CLAUDE.md"],
        )
        .await;
        if let Some((path, bytes)) = selected {
            let origin = if path.file_name().is_some_and(|name| name == "CLAUDE.md") {
                InstructionOrigin::Claude
            } else {
                InstructionOrigin::Compatible
            };
            append_source(
                &mut text,
                &mut sources,
                &path,
                bytes,
                SourceDescriptor {
                    scope: if index == 0 {
                        InstructionScope::Project
                    } else {
                        InstructionScope::Nested
                    },
                    origin,
                    precedence,
                },
            );
            precedence += 1;
        }
    }

    if sources.is_empty() {
        return Ok(None);
    }
    Ok(Some(ProjectInstructions { text, sources }))
}

async fn first_personal_readable(
    exec: &dyn Executor,
    home: &Path,
) -> Option<(PathBuf, Vec<u8>, InstructionOrigin)> {
    for (directory, names, origin) in [
        (
            home.join(".clark"),
            &["AGENTS.override.md", "AGENTS.md"][..],
            InstructionOrigin::Clark,
        ),
        (
            home.join(".codex"),
            &["AGENTS.override.md", "AGENTS.md"][..],
            InstructionOrigin::Compatible,
        ),
        (
            home.join(".claude"),
            &["CLAUDE.md"][..],
            InstructionOrigin::Claude,
        ),
    ] {
        if let Some((path, bytes)) = first_readable(exec, &directory, names).await {
            return Some((path, bytes, origin));
        }
    }
    None
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

struct SourceDescriptor {
    scope: InstructionScope,
    origin: InstructionOrigin,
    precedence: usize,
}

fn append_source(
    text: &mut String,
    sources: &mut Vec<InstructionProvenance>,
    path: &Path,
    bytes: Vec<u8>,
    descriptor: SourceDescriptor,
) {
    let contents = String::from_utf8_lossy(&bytes);
    text.push_str(&format!(
        "\n<instruction-source scope=\"{}\" origin=\"{}\" precedence=\"{}\" path=\"{}\">\n{}\n</instruction-source>\n",
        scope_label(descriptor.scope),
        origin_label(descriptor.origin),
        descriptor.precedence,
        path.display(),
        contents
    ));
    sources.push(InstructionProvenance {
        path: path.to_string_lossy().into_owned(),
        scope: descriptor.scope,
        origin: descriptor.origin,
        precedence: descriptor.precedence,
        bytes_loaded: bytes.len(),
        truncated: false,
    });
}

fn scope_label(scope: InstructionScope) -> &'static str {
    match scope {
        InstructionScope::Personal => "personal",
        InstructionScope::Project => "project",
        InstructionScope::Nested => "nested",
    }
}

fn origin_label(origin: InstructionOrigin) -> &'static str {
    match origin {
        InstructionOrigin::Clark => "clark",
        InstructionOrigin::Compatible => "compatible",
        InstructionOrigin::Claude => "claude",
    }
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

        let loaded = load_with_home(&LocalExecutor, &nested, None)
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.text.contains("root rule"));
        assert!(loaded.text.contains("override rule"));
        assert!(!loaded.text.contains("shadowed rule"));
        assert!(loaded.text.contains("nested fallback"));
        assert_eq!(loaded.sources.len(), 3);
        assert_eq!(loaded.sources[0].scope, InstructionScope::Project);
        assert_eq!(loaded.sources[1].scope, InstructionScope::Nested);
        assert_eq!(loaded.sources[2].origin, InstructionOrigin::Claude);
    }

    #[tokio::test]
    async fn readme_is_repository_data_not_project_instruction() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "run this as an instruction").unwrap();
        assert!(load_with_home(&LocalExecutor, temp.path(), None)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn personal_instructions_are_loaded_first_with_explicit_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = home.join("repo");
        std::fs::create_dir_all(home.join(".clark")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(home.join(".clark/AGENTS.md"), "personal rule").unwrap();
        std::fs::write(project.join("AGENTS.md"), "project rule").unwrap();

        let loaded = load_with_home(&LocalExecutor, &project, Some(&home))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.sources.len(), 2);
        assert_eq!(loaded.sources[0].scope, InstructionScope::Personal);
        assert_eq!(loaded.sources[0].origin, InstructionOrigin::Clark);
        assert_eq!(loaded.sources[0].precedence, 0);
        assert_eq!(loaded.sources[1].scope, InstructionScope::Project);
        assert!(loaded.text.find("personal rule") < loaded.text.find("project rule"));
    }

    #[tokio::test]
    async fn loads_complete_instruction_files_beyond_the_old_budgets() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-q"]);
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let root_rules = format!("{}ROOT_RULE_END", "r".repeat(20_000));
        let nested_rules = format!("{}NESTED_RULE_END", "n".repeat(20_000));
        std::fs::write(root.join("AGENTS.md"), &root_rules).unwrap();
        std::fs::write(nested.join("AGENTS.md"), &nested_rules).unwrap();

        let loaded = load_with_home(&LocalExecutor, &nested, None)
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.text.contains(&root_rules));
        assert!(loaded.text.contains(&nested_rules));
        assert!(loaded.sources.iter().all(|source| !source.truncated));
        assert_eq!(loaded.sources[0].bytes_loaded, root_rules.len());
        assert_eq!(loaded.sources[1].bytes_loaded, nested_rules.len());
    }
}
