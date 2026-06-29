//! Per-repository memory — a durable, project-scoped knowledge file the local
//! agent reads every session and can update, and that **Clark can extract
//! automatically** from the repo.
//!
//! Design adapted (clean-room) from Claude Code's `memdir`: an always-loaded
//! `MEMORY.md` index plus optional per-fact markdown files with `name` /
//! `description` / `type` frontmatter, all under `<root>/.clark/memory/`. Living
//! in the project root means the agent's own sandboxed file tools already read
//! and write them; nothing extra is needed for the model to maintain memory.
//!
//! Extraction (`extract_repo_memory`) builds a bounded digest of the repo
//! locally, then asks Clark's sandbox agent (web search + analysis) to distill a
//! concise project memory, and writes it to `MEMORY.md`.

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use crate::llm::LlmClient;

/// Directory (relative to the project root) holding memory files.
pub const MEMORY_SUBDIR: &str = ".clark/memory";
/// Always-loaded index / project-memory file.
pub const INDEX_FILE: &str = "MEMORY.md";

const MAX_INDEX_BYTES: usize = 24_000;
const MAX_DIGEST_BYTES: usize = 28_000;
const MAX_KEY_FILE_BYTES: usize = 4_000;
const MAX_TREE_ENTRIES: usize = 400;

/// Root files worth feeding to the extractor verbatim.
const KEY_FILES: &[&str] = &[
    "README.md",
    "README",
    "AGENTS.md",
    "CLAUDE.md",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "Makefile",
    "tsconfig.json",
    "requirements.txt",
    "Gemfile",
];

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

pub fn memory_dir(root: &Path) -> PathBuf {
    root.join(MEMORY_SUBDIR)
}

pub fn index_path(root: &Path) -> PathBuf {
    memory_dir(root).join(INDEX_FILE)
}

/// True once a project memory has been written (extraction is idempotent).
pub fn has_memory(root: &Path) -> bool {
    index_path(root).is_file()
}

/// Read the project-memory index, capped for prompt safety.
pub fn load_index(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(index_path(root)).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(truncate_chars(text, MAX_INDEX_BYTES))
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

/// Scan per-fact memory files (everything but the index), newest first.
pub fn scan(root: &Path) -> Vec<MemoryHeader> {
    load_facts(root).into_iter().map(|f| f.header).collect()
}

/// Read every per-fact memory file (everything but the index) with its body,
/// newest first. Bodies are capped so the result is safe to hand to the UI.
pub fn load_facts(root: &Path) -> Vec<MemoryFact> {
    let dir = memory_dir(root);
    let mut out: Vec<(std::time::SystemTime, MemoryFact)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == INDEX_FILE || !name.ends_with(".md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (n, d, k) = parse_frontmatter(&text);
        let body = truncate_chars(strip_frontmatter(&text).trim(), MAX_KEY_FILE_BYTES);
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        out.push((
            mtime,
            MemoryFact {
                header: MemoryHeader {
                    file: name,
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

/// Return the text after the leading `--- … ---` frontmatter block (or the
/// whole text if there is none).
fn strip_frontmatter(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return trimmed;
    };
    // `rest` begins right after the opening `---`; find the closing delimiter.
    let Some(end) = rest.find("\n---") else {
        return text;
    };
    // Skip past the closing `---` line to the start of the body.
    let after_marker = &rest[end + 1..];
    match after_marker.find('\n') {
        Some(nl) => &after_marker[nl + 1..],
        None => "",
    }
}

/// The system-prompt section: maintenance instructions plus the current index.
pub fn system_prompt_section(root: &Path) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Durable, project-specific facts live in `{}/` ({} is the index). \
Maintain them with your file tools: when you learn something lasting about this \
project (architecture, conventions, build/test commands, gotchas, decisions), \
record it there so future sessions benefit. Treat these as point-in-time notes — \
verify against the current code before relying on them.\n",
        MEMORY_SUBDIR, INDEX_FILE
    ));
    if let Some(index) = load_index(root) {
        s.push_str("\nCurrent project memory:\n\n");
        s.push_str(&index);
        s.push('\n');
    }
    // List any additional per-fact memory files so the agent knows to read them.
    let facts = scan(root);
    if !facts.is_empty() {
        s.push_str("\nAdditional memory files (read on demand):\n");
        for f in facts {
            let kind = f
                .kind
                .map(|k| format!("[{}] ", k.label()))
                .unwrap_or_default();
            let desc = f.description.unwrap_or_else(|| "(no description)".into());
            s.push_str(&format!("- {kind}{}/{} — {desc}\n", MEMORY_SUBDIR, f.file));
        }
    }
    s
}

/// Build a bounded, text digest of the repo for the extractor: a file tree plus
/// the contents of well-known root files.
pub fn build_repo_digest(root: &Path) -> String {
    let mut digest = String::new();
    digest.push_str("# Repository digest\n\n## File tree (partial)\n");

    let mut count = 0usize;
    for entry in WalkDir::new(root)
        .max_depth(6)
        .into_iter()
        .filter_entry(|e| !is_skippable(e.path()))
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            digest.push_str(rel.to_string_lossy().as_ref());
            digest.push('\n');
            count += 1;
            if count >= MAX_TREE_ENTRIES {
                digest.push_str("… [tree truncated]\n");
                break;
            }
        }
    }

    for name in KEY_FILES {
        let path = root.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            digest.push_str(&format!("\n## {name}\n```\n"));
            digest.push_str(&truncate_chars(&text, MAX_KEY_FILE_BYTES));
            digest.push_str("\n```\n");
        }
        if digest.len() >= MAX_DIGEST_BYTES {
            break;
        }
    }
    truncate_chars(&digest, MAX_DIGEST_BYTES)
}

/// The instruction wrapped around the digest for Clark.
fn extraction_prompt(digest: &str) -> String {
    format!(
        "You are bootstrapping a project-memory note for a coding agent that works \
in this repository. Below is a digest of the repo (file tree + key files). \
Produce a CONCISE markdown memory (aim for under 150 lines) that a future agent \
should know before working here. Cover, only where evident:\n\
- What the project is and its purpose\n\
- High-level architecture and how the main parts fit together\n\
- Key entry points / important files and directories\n\
- How to build, test, run, and lint (exact commands if you can infer them)\n\
- Conventions and constraints worth honoring\n\
- Non-obvious gotchas\n\
Where it helps, you may use web search to identify the frameworks/libraries in \
use and note current best practices. Do NOT invent facts not supported by the \
digest. Output only the markdown memory, starting with a `# <Project> — project \
memory` heading.\n\n{digest}"
    )
}

/// Extract a project memory via Clark's agentic Platform API and write it to
/// `<root>/.clark/memory/MEMORY.md`. Returns the written memory text. `base_url`
/// is the Platform API (`…/v1`), `api_key` the `ck_live_` key, `model` an
/// agentic Clark model.
pub async fn extract_repo_memory(
    root: &Path,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> Result<String, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let digest = build_repo_digest(root);
    let client = LlmClient::from_parts(
        base_url,
        model,
        api_key.map(str::to_string),
        Vec::new(),
        None,
    )?;
    let memory = client
        .complete(None, &extraction_prompt(&digest), &CancellationToken::new())
        .await
        .map_err(|e| e.to_string())?;
    if memory.trim().is_empty() {
        return Err("Clark returned an empty memory".into());
    }

    let dir = memory_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = index_path(root);
    let body = format!(
        "<!-- Auto-extracted by Clark. Edit freely; the agent maintains this. -->\n\n{}\n",
        memory.trim()
    );
    std::fs::write(&path, body).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(memory)
}

/// Skip the memory dir, VCS, and heavy build/vendor dirs while walking.
fn is_skippable(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_string_lossy().as_ref(),
            ".git" | "node_modules" | "target" | "dist" | ".next" | ".venv" | ".clark"
        )
    })
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

    #[test]
    fn paths_are_under_dot_clark() {
        let root = Path::new("/proj");
        assert_eq!(memory_dir(root), Path::new("/proj/.clark/memory"));
        assert_eq!(index_path(root), Path::new("/proj/.clark/memory/MEMORY.md"));
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
    fn frontmatter_absent_yields_none() {
        let (n, d, k) = parse_frontmatter("no frontmatter here");
        assert!(n.is_none() && d.is_none() && k.is_none());
    }

    #[test]
    fn load_index_and_has_memory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_memory(dir.path()));
        assert!(load_index(dir.path()).is_none());
        std::fs::create_dir_all(memory_dir(dir.path())).unwrap();
        std::fs::write(index_path(dir.path()), "# Project memory\n\nIt builds.").unwrap();
        assert!(has_memory(dir.path()));
        assert!(load_index(dir.path()).unwrap().contains("It builds."));
    }

    #[test]
    fn scan_reads_fact_files_not_index() {
        let dir = tempfile::tempdir().unwrap();
        let mdir = memory_dir(dir.path());
        std::fs::create_dir_all(&mdir).unwrap();
        std::fs::write(mdir.join("MEMORY.md"), "index").unwrap();
        std::fs::write(
            mdir.join("conventions.md"),
            "---\nname: conv\ndescription: style rules\ntype: project\n---\nbody",
        )
        .unwrap();
        let scanned = scan(dir.path());
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].name.as_deref(), Some("conv"));
        assert_eq!(scanned[0].kind, Some(MemoryType::Project));
    }

    #[test]
    fn load_facts_returns_body_without_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let mdir = memory_dir(dir.path());
        std::fs::create_dir_all(&mdir).unwrap();
        std::fs::write(mdir.join("MEMORY.md"), "index").unwrap();
        std::fs::write(
            mdir.join("conventions.md"),
            "---\nname: conv\ndescription: style rules\ntype: project\n---\n\nUse 2-space indent.",
        )
        .unwrap();
        let facts = load_facts(dir.path());
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].header.name.as_deref(), Some("conv"));
        assert_eq!(facts[0].body, "Use 2-space indent.");
        assert!(!facts[0].body.contains("style rules"));
    }

    #[test]
    fn strip_frontmatter_handles_missing_and_present() {
        assert_eq!(strip_frontmatter("no fm\nbody").trim(), "no fm\nbody");
        assert_eq!(
            strip_frontmatter("---\nname: x\n---\nbody here").trim(),
            "body here"
        );
    }

    #[test]
    fn system_prompt_section_includes_index_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let without = system_prompt_section(dir.path());
        assert!(without.contains(".clark/memory"));
        assert!(!without.contains("Current project memory"));

        std::fs::create_dir_all(memory_dir(dir.path())).unwrap();
        std::fs::write(index_path(dir.path()), "Key fact: uses Tauri.").unwrap();
        let with = system_prompt_section(dir.path());
        assert!(with.contains("Current project memory"));
        assert!(with.contains("uses Tauri"));
    }

    #[test]
    fn digest_includes_tree_and_key_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("README.md"), "My cool project").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/junk"), "ignore me").unwrap();
        let digest = build_repo_digest(dir.path());
        assert!(digest.contains("src/main.rs"));
        assert!(digest.contains("## README.md"));
        assert!(digest.contains("My cool project"));
        assert!(!digest.contains("target/junk"));
    }
}
