use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::exec::Executor;
use crate::memory::{self, ImportedMemoryChange, MemoryType};

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct MemoryMigrationReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failures: Vec<String>,
}

impl MemoryMigrationReport {
    pub fn discovered(&self) -> usize {
        self.created + self.updated + self.unchanged
    }

    fn record(&mut self, change: ImportedMemoryChange) {
        match change {
            ImportedMemoryChange::Created => self.created += 1,
            ImportedMemoryChange::Updated => self.updated += 1,
            ImportedMemoryChange::Unchanged => self.unchanged += 1,
        }
    }
}

/// Import the durable, user-visible memory stores maintained by compatible
/// desktop coding agents. Sources remain untouched. Project memories land in
/// this checkout's `.agent/memory`; cross-project summaries land only in the
/// signed-in account's opaque global-memory partition.
pub(crate) async fn migrate(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
    global_dir: Option<&Path>,
) -> MemoryMigrationReport {
    let mut report = MemoryMigrationReport::default();
    let project_dir = memory::memory_dir(project_root);
    let mut seen = HashSet::new();

    for directory in [
        project_root.join(".claude/memory"),
        project_root.join(".codex/memory"),
        project_root.join(".codex/memories"),
    ] {
        import_directory(
            exec,
            &directory,
            &project_dir,
            source_for_path(&directory),
            MemoryType::Project,
            &mut seen,
            &mut report,
        )
        .await;
    }

    let Some(home) = home else {
        return report;
    };

    let claude_project = home
        .join(".claude/projects")
        .join(claude_project_directory_name(project_root))
        .join("memory");
    import_directory(
        exec,
        &claude_project,
        &project_dir,
        "claude",
        MemoryType::Project,
        &mut seen,
        &mut report,
    )
    .await;

    let Some(global_dir) = global_dir else {
        return report;
    };

    import_directory(
        exec,
        &home.join(".claude/memory"),
        global_dir,
        "claude",
        MemoryType::User,
        &mut seen,
        &mut report,
    )
    .await;
    import_file(
        exec,
        &home.join(".claude/MEMORY.md"),
        global_dir,
        "claude",
        MemoryType::User,
        &mut seen,
        &mut report,
    )
    .await;

    // Codex Desktop's compact synthesis is the import authority. The adjacent
    // MEMORY.md registry can be hundreds of kilobytes and is intentionally only
    // a fallback when no curated summary exists.
    let openai_candidates = [
        home.join(".codex/memories/memory_summary.md"),
        home.join(".codex/memory_summary.md"),
        home.join(".codex/memories/MEMORY.md"),
        home.join(".codex/MEMORY.md"),
    ];
    for candidate in openai_candidates {
        if import_file(
            exec,
            &candidate,
            global_dir,
            "openai",
            MemoryType::User,
            &mut seen,
            &mut report,
        )
        .await
        {
            break;
        }
    }

    report
}

fn source_for_path(path: &Path) -> &'static str {
    if path
        .components()
        .any(|component| component.as_os_str() == ".claude")
    {
        "claude"
    } else {
        "openai"
    }
}

fn claude_project_directory_name(project_root: &Path) -> String {
    project_root
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

async fn import_directory(
    exec: &dyn Executor,
    source_dir: &Path,
    destination: &Path,
    source: &str,
    kind: MemoryType,
    seen: &mut HashSet<PathBuf>,
    report: &mut MemoryMigrationReport,
) {
    if exec.metadata(source_dir).await.is_err() {
        return;
    }
    let entries = match exec.read_dir(source_dir).await {
        Ok(entries) => entries,
        Err(error) => {
            report
                .failures
                .push(format!("could not read {}: {error}", source_dir.display()));
            return;
        }
    };
    let mut markdown = entries
        .into_iter()
        .filter(|entry| !entry.is_dir && !entry.is_symlink && entry.name.ends_with(".md"))
        .map(|entry| source_dir.join(entry.name))
        .collect::<Vec<_>>();
    markdown.sort();
    if markdown.len() > 1 {
        markdown.retain(|path| path.file_name().is_none_or(|name| name != "MEMORY.md"));
    }
    for path in markdown {
        import_file(exec, &path, destination, source, kind, seen, report).await;
    }
}

async fn import_file(
    exec: &dyn Executor,
    source_path: &Path,
    destination: &Path,
    source: &str,
    kind: MemoryType,
    seen: &mut HashSet<PathBuf>,
    report: &mut MemoryMigrationReport,
) -> bool {
    if !seen.insert(source_path.to_path_buf()) {
        return false;
    }
    if exec.metadata(source_path).await.is_err() {
        return false;
    }
    let bytes = match exec.read(source_path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            report
                .failures
                .push(format!("could not read {}: {error}", source_path.display()));
            return false;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            report.failures.push(format!(
                "compatible memory is not UTF-8: {}",
                source_path.display()
            ));
            return false;
        }
    };
    let content = markdown_body(&text).trim();
    if content.is_empty() {
        return false;
    }
    let title = imported_title(&text, source_path, source);
    match memory::upsert_imported_memory(
        exec,
        destination,
        source,
        &source_path.to_string_lossy(),
        &title,
        content,
        kind,
    )
    .await
    {
        Ok(change) => report.record(change),
        Err(error) => report.failures.push(format!(
            "could not import {}: {error}",
            source_path.display()
        )),
    }
    true
}

fn imported_title(text: &str, path: &Path, source: &str) -> String {
    let metadata = memory::parse_frontmatter(text);
    let name = metadata.name.or_else(|| {
        markdown_body(text)
            .lines()
            .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
            .filter(|line| !line.is_empty())
            .map(str::to_string)
    });
    let name = name.unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .replace(['-', '_'], " ")
    });
    let label = if source == "claude" {
        "Claude"
    } else {
        "OpenAI Codex"
    };
    format!("{label}: {name}")
}

fn markdown_body(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return trimmed;
    };
    let Some(end) = rest.find("\n---") else {
        return trimmed;
    };
    rest[end + 4..].trim_start_matches(['\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn imports_claude_project_notes_and_curated_openai_global_summary() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("work/project");
        std::fs::create_dir_all(&project).unwrap();
        let claude_memory = home
            .join(".claude/projects")
            .join(claude_project_directory_name(&project))
            .join("memory");
        write(
            &claude_memory.join("build.md"),
            "---\nname: Build contract\n---\n\nUse cargo nextest.",
        );
        write(
            &claude_memory.join("MEMORY.md"),
            "# Claude index\n- [Build](build.md)",
        );
        write(
            &home.join(".codex/memories/memory_summary.md"),
            "# User profile\nPrefers typed contracts.",
        );
        write(
            &home.join(".codex/memories/MEMORY.md"),
            "# Large registry\nThis fallback must not be imported.",
        );
        let global = temp.path().join("account-memory");

        let first = migrate(&LocalExecutor, &project, Some(&home), Some(&global)).await;
        assert_eq!(first.created, 2);
        assert_eq!(first.updated, 0);
        assert!(first.failures.is_empty(), "{:?}", first.failures);

        let project_facts = memory::load_facts(&LocalExecutor, &memory::memory_dir(&project)).await;
        assert_eq!(project_facts.len(), 1);
        assert!(project_facts[0].body.contains("nextest"));
        assert_eq!(
            project_facts[0].header.source.as_deref(),
            Some("imported-claude")
        );
        let global_facts = memory::load_facts(&LocalExecutor, &global).await;
        assert_eq!(global_facts.len(), 1);
        assert!(global_facts[0].body.contains("typed contracts"));
        assert!(!global_facts[0].body.contains("fallback"));

        let second = migrate(&LocalExecutor, &project, Some(&home), Some(&global)).await;
        assert_eq!(second.created, 0);
        assert_eq!(second.unchanged, 2);
    }
}
