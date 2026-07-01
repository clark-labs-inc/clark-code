//! Durable agent memory — two scopes the agent reads and maintains through the
//! [`memory`](crate::tools) tool:
//!
//! - **project** — facts about *this* codebase, under `<root>/.clark/memory/`.
//!   Read/written through the session's [`Executor`], so it lives on the local
//!   disk for a local project and on the remote host for a remote one.
//! - **global** — facts about the *user* across every project, under
//!   `~/.clark/memory/` on the machine running the desktop app (always local).
//!
//! Each scope is an always-loaded `MEMORY.md` index plus optional per-fact
//! markdown files carrying `name` / `description` / `type` frontmatter. Design
//! adapted clean-room from Claude Code's `memdir`. There is no "extraction" step:
//! the agent curates memory itself via the tool as a conversation unfolds.

use std::path::{Path, PathBuf};

use crate::exec::Executor;

/// Directory (relative to a scope root) holding memory files.
pub const MEMORY_SUBDIR: &str = ".clark/memory";
/// Always-loaded index / project-memory file.
pub const INDEX_FILE: &str = "MEMORY.md";

const MAX_INDEX_BYTES: usize = 24_000;
const MAX_FACT_BODY_BYTES: usize = 4_000;

/// Memory taxonomy (mirrors the recall system's categories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" => Some(Self::User),
            "feedback" => Some(Self::Feedback),
            "project" => Some(Self::Project),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }
}

/// Parsed metadata for one memory file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryHeader {
    pub file: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<MemoryType>,
}

/// One per-fact memory file: its parsed header plus the body (frontmatter
/// stripped, capped for display).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryFact {
    pub header: MemoryHeader,
    pub body: String,
}

/// The memory directory for a scope root (`<root>/.clark/memory`).
pub fn memory_dir(root: &Path) -> PathBuf {
    root.join(MEMORY_SUBDIR)
}

/// The user's global memory directory (`~/.clark/memory`) on the local machine,
/// or `None` if the home directory can't be resolved.
pub fn global_memory_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(MEMORY_SUBDIR))
}

/// Parse the leading `--- ... ---` YAML-ish frontmatter for the fields we use.
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>, Option<MemoryType>) {
    let mut name = None;
    let mut description = None;
    let mut kind = None;
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (name, description, kind);
    };
    let Some(end) = rest.find("\n---") else {
        return (name, description, kind);
    };
    for line in rest[..end].lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(['"', '\'']).to_string();
            match key.trim() {
                "name" => name = Some(value),
                "description" => description = Some(value),
                "type" => kind = MemoryType::parse(&value),
                _ => {}
            }
        }
    }
    (name, description, kind)
}

/// Return the text after the leading `--- … ---` frontmatter block (or the whole
/// text if there is none).
fn strip_frontmatter(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return trimmed;
    };
    let Some(end) = rest.find("\n---") else {
        return text;
    };
    let after_marker = &rest[end + 1..];
    match after_marker.find('\n') {
        Some(nl) => &after_marker[nl + 1..],
        None => "",
    }
}

async fn read_text(exec: &dyn Executor, path: &Path) -> Option<String> {
    String::from_utf8(exec.read(path).await.ok()?).ok()
}

/// The scope's index (`MEMORY.md`), capped for prompt safety.
pub async fn load_index(exec: &dyn Executor, mem_dir: &Path) -> Option<String> {
    let text = read_text(exec, &mem_dir.join(INDEX_FILE)).await?;
    let text = text.trim();
    (!text.is_empty()).then(|| truncate_chars(text, MAX_INDEX_BYTES))
}

/// Every per-fact memory file (everything but the index) with its body, newest
/// first. Bodies are capped so the result is safe to hand to the UI or model.
pub async fn load_facts(exec: &dyn Executor, mem_dir: &Path) -> Vec<MemoryFact> {
    let Ok(entries) = exec.read_dir(mem_dir).await else {
        return Vec::new();
    };
    let mut out: Vec<(std::time::SystemTime, MemoryFact)> = Vec::new();
    for e in entries {
        if e.is_dir || e.name == INDEX_FILE || !e.name.ends_with(".md") {
            continue;
        }
        let path = mem_dir.join(&e.name);
        let Some(text) = read_text(exec, &path).await else {
            continue;
        };
        let (n, d, k) = parse_frontmatter(&text);
        let body = truncate_chars(strip_frontmatter(&text).trim(), MAX_FACT_BODY_BYTES);
        let mtime = exec
            .metadata(&path)
            .await
            .ok()
            .and_then(|m| m.modified)
            .unwrap_or(std::time::UNIX_EPOCH);
        out.push((
            mtime,
            MemoryFact {
                header: MemoryHeader {
                    file: e.name,
                    name: n,
                    description: d,
                    kind: k,
                },
                body,
            },
        ));
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out.into_iter().map(|(_, f)| f).collect()
}

/// A compact listing of one scope for the system prompt: the index plus a
/// one-line-per-fact catalog (no bodies). `None` when the scope is empty.
pub async fn scope_listing(exec: &dyn Executor, mem_dir: &Path, label: &str) -> Option<String> {
    let index = load_index(exec, mem_dir).await;
    let facts = load_facts(exec, mem_dir).await;
    if index.is_none() && facts.is_empty() {
        return None;
    }
    let mut s = format!("## {label} memory\n");
    if let Some(idx) = index {
        s.push_str(&idx);
        s.push('\n');
    }
    if !facts.is_empty() {
        s.push_str("\nSaved notes (use `memory` → recall for full text):\n");
        for f in &facts {
            let kind = f
                .header
                .kind
                .map(|k| format!("[{}] ", k.label()))
                .unwrap_or_default();
            let name = f.header.name.clone().unwrap_or_else(|| f.header.file.clone());
            let desc = f.header.description.clone().unwrap_or_default();
            s.push_str(&format!("- {kind}{name} — {desc}\n"));
        }
    }
    Some(s)
}

/// Full recall of one scope for the `memory` tool: the index plus every fact's
/// body. `None` when the scope is empty.
pub async fn recall_scope(exec: &dyn Executor, mem_dir: &Path, label: &str) -> Option<String> {
    let index = load_index(exec, mem_dir).await;
    let facts = load_facts(exec, mem_dir).await;
    if index.is_none() && facts.is_empty() {
        return None;
    }
    let mut s = format!("## {label} memory\n");
    if let Some(idx) = index {
        s.push_str(&idx);
        s.push_str("\n\n");
    }
    for f in &facts {
        let name = f.header.name.clone().unwrap_or_else(|| f.header.file.clone());
        let kind = f
            .header
            .kind
            .map(|k| format!(" [{}]", k.label()))
            .unwrap_or_default();
        s.push_str(&format!("### {name}{kind}\n{}\n\n", f.body));
    }
    Some(s.trim_end().to_string())
}

/// Save one durable fact into a scope: write `<slug>.md` with frontmatter and add
/// a pointer line to the scope's `MEMORY.md` index (creating it if needed).
/// Returns the written file name.
pub async fn save_memory(
    exec: &dyn Executor,
    mem_dir: &Path,
    title: &str,
    content: &str,
    kind: Option<MemoryType>,
) -> Result<String, String> {
    let slug = slugify(title);
    if slug.is_empty() {
        return Err("title must contain letters or digits".into());
    }
    let file = format!("{slug}.md");
    let desc = content
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(title)
        .trim()
        .replace(['\n', '\r'], " ");
    let desc = truncate_chars(&desc, 200);
    let kind_line = kind
        .map(|k| format!("type: {}\n", k.label()))
        .unwrap_or_default();
    let body = format!(
        "---\nname: {}\ndescription: {}\n{}---\n\n{}\n",
        title.trim(),
        desc,
        kind_line,
        content.trim(),
    );
    exec.create_dir_all(mem_dir)
        .await
        .map_err(|e| format!("creating memory dir: {e}"))?;
    exec.write(&mem_dir.join(&file), body.as_bytes())
        .await
        .map_err(|e| format!("writing memory: {e}"))?;

    // Keep the index pointing at every fact (one line each), created on demand.
    let index_path = mem_dir.join(INDEX_FILE);
    let mut index = read_text(exec, &index_path)
        .await
        .unwrap_or_else(|| "# Memory index\n".to_string());
    if !index.contains(&format!("]({file})")) {
        if !index.ends_with('\n') {
            index.push('\n');
        }
        index.push_str(&format!("- [{}]({file}) — {}\n", title.trim(), desc));
        exec.write(&index_path, index.as_bytes())
            .await
            .map_err(|e| format!("writing index: {e}"))?;
    }
    Ok(file)
}

/// Kebab-case a title into a safe file stem.
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let slug = out.trim_matches('-').to_string();
    truncate_chars(&slug, 60).trim_matches('-').to_string()
}

/// Maintenance instructions for the system prompt (present whenever memory is on).
pub fn memory_guidance() -> &'static str {
    "You have a durable memory via the `memory` tool. Call it with action \"recall\" to \
load saved facts before relying on them, and action \"remember\" to save a lasting fact — \
scope \"project\" for things specific to this codebase (architecture, conventions, \
build/test commands, gotchas, decisions), scope \"global\" for things true across all of \
the user's projects (their preferences, environment, how they like you to work). Save \
sparingly: durable, reusable facts only — never transient task details. Treat saved notes \
as point-in-time and verify against the current code before relying on them.\n"
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = (0..=max)
        .rev()
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(0);
    format!("{}\n… [truncated]", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    #[test]
    fn paths_are_under_dot_clark() {
        let root = Path::new("/proj");
        assert_eq!(memory_dir(root), Path::new("/proj/.clark/memory"));
    }

    #[test]
    fn parses_frontmatter_fields() {
        let text =
            "---\nname: build-cmd\ndescription: how to build\ntype: project\n---\n\nUse cargo.";
        let (n, d, k) = parse_frontmatter(text);
        assert_eq!(n.as_deref(), Some("build-cmd"));
        assert_eq!(d.as_deref(), Some("how to build"));
        assert_eq!(k, Some(MemoryType::Project));
    }

    #[test]
    fn slugify_kebabs_and_trims() {
        assert_eq!(slugify("Build & Test commands!"), "build-test-commands");
        assert_eq!(slugify("  Hello   World  "), "hello-world");
        assert_eq!(slugify("***"), "");
    }

    #[tokio::test]
    async fn save_then_recall_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory_dir(dir.path());
        let exec = LocalExecutor;
        let file = save_memory(
            &exec,
            &mem,
            "Build command",
            "Run `cargo build` from the repo root.",
            Some(MemoryType::Project),
        )
        .await
        .unwrap();
        assert_eq!(file, "build-command.md");

        // Index created + points at the fact.
        let index = load_index(&exec, &mem).await.unwrap();
        assert!(index.contains("build-command.md"));

        // Fact readable, frontmatter stripped from the body.
        let facts = load_facts(&exec, &mem).await;
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].header.name.as_deref(), Some("Build command"));
        assert_eq!(facts[0].header.kind, Some(MemoryType::Project));
        assert!(facts[0].body.contains("cargo build"));
        assert!(!facts[0].body.contains("description:"));

        // Recall bundles index + bodies; listing is index + catalog.
        let recall = recall_scope(&exec, &mem, "Project").await.unwrap();
        assert!(recall.contains("cargo build"));
        let listing = scope_listing(&exec, &mem, "Project").await.unwrap();
        assert!(listing.contains("Build command"));
    }

    #[tokio::test]
    async fn empty_scope_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        assert!(scope_listing(&exec, &memory_dir(dir.path()), "Project")
            .await
            .is_none());
        assert!(load_index(&exec, &memory_dir(dir.path())).await.is_none());
    }
}
