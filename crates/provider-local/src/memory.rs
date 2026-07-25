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
    /// ISO date the note was saved (`saved:` frontmatter), when present.
    pub saved: Option<String>,
    /// Provenance: "user-stated" or "inferred" (`source:` frontmatter).
    pub source: Option<String>,
}

/// One per-fact memory file: its parsed header plus the body (frontmatter
/// stripped, capped for display).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryFact {
    pub header: MemoryHeader,
    pub body: String,
    /// Filesystem mtime — the age fallback when `saved:` is absent.
    pub mtime: Option<std::time::SystemTime>,
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

/// Parsed frontmatter fields of one memory file.
#[derive(Default)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<MemoryType>,
    pub saved: Option<String>,
    pub source: Option<String>,
}

/// Parse the leading `--- ... ---` YAML-ish frontmatter for the fields we use.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return fm;
    };
    let Some(end) = rest.find("\n---") else {
        return fm;
    };
    for line in rest[..end].lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(['"', '\'']).to_string();
            match key.trim() {
                "name" => fm.name = Some(value),
                "description" => fm.description = Some(value),
                "type" => fm.kind = MemoryType::parse(&value),
                "saved" => fm.saved = Some(value),
                "source" => fm.source = Some(value),
                _ => {}
            }
        }
    }
    fm
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
        let fm = parse_frontmatter(&text);
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
                    name: fm.name,
                    description: fm.description,
                    kind: fm.kind,
                    saved: fm.saved,
                    source: fm.source,
                },
                body,
                mtime: Some(mtime),
            },
        ));
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out.into_iter().map(|(_, f)| f).collect()
}

/// Human age label for a note: prefers the `saved:` frontmatter date, falls
/// back to the file mtime, admits ignorance otherwise.
fn age_label(fact: &MemoryFact) -> String {
    let days = fact
        .header
        .saved
        .as_deref()
        .and_then(days_since_iso_date)
        .or_else(|| {
            fact.mtime.and_then(|m| {
                std::time::SystemTime::now()
                    .duration_since(m)
                    .ok()
                    .map(|d| (d.as_secs() / 86_400) as i64)
            })
        });
    match days {
        None => "age unknown".to_string(),
        Some(0) => "saved today".to_string(),
        Some(d) if d < 0 => "age unknown".to_string(),
        Some(1) => "saved yesterday".to_string(),
        Some(d) if d < 60 => format!("saved {d} days ago"),
        Some(d) => format!("saved ~{} months ago", d / 30),
    }
}

/// `[inferred]` marker for facts the agent concluded rather than was told.
fn source_marker(fact: &MemoryFact) -> &'static str {
    match fact.header.source.as_deref() {
        Some("inferred") => " [inferred — verify before relying]",
        _ => "",
    }
}

/// A compact listing of one scope for the system prompt: the index plus a
/// one-line-per-fact catalog (no bodies) with age and provenance. When
/// `verify_root` is set (project scope), facts referencing missing paths or
/// unknown npm scripts get a ⚠ annotation. `None` when the scope is empty.
pub async fn scope_listing(
    exec: &dyn Executor,
    mem_dir: &Path,
    label: &str,
    verify_root: Option<&Path>,
) -> Option<String> {
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
        s.push_str("\nSaved notes (use `memory` → recall for full text; re-verify CODE facts against the repo — the user's own decisions and preferences stay binding until they change them):\n");
        for f in &facts {
            let kind = f
                .header
                .kind
                .map(|k| format!("[{}] ", k.label()))
                .unwrap_or_default();
            let name = f
                .header
                .name
                .clone()
                .unwrap_or_else(|| f.header.file.clone());
            let desc = f.header.description.clone().unwrap_or_default();
            s.push_str(&format!(
                "- {kind}{name} — {desc} ({}){}\n",
                age_label(f),
                source_marker(f)
            ));
            if let Some(root) = verify_root {
                for warn in verify_fact(exec, root, &f.body).await {
                    s.push_str(&format!("  {warn}\n"));
                }
            }
        }
        // Decisions and standing rules get restated imperatively at the end
        // of the section (recency + command framing): models reliably *recall*
        // notes but under-*apply* them when producing content.
        let decisions: Vec<&MemoryFact> = facts.iter().filter(|f| is_decision(f)).collect();
        if !decisions.is_empty() {
            s.push_str(
                "\nStanding decisions — apply these as written in everything you produce:\n",
            );
            for f in &decisions {
                let line = f
                    .body
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
                    .trim();
                s.push_str(&format!("- {}\n", single_line_ellipsis(line, 200)));
            }
        }
    }
    Some(s)
}

/// Whether a fact reads as a standing decision/rule rather than a code fact:
/// typed as user/feedback, or carrying rule-shaped wording.
fn is_decision(fact: &MemoryFact) -> bool {
    if matches!(
        fact.header.kind,
        Some(MemoryType::User) | Some(MemoryType::Feedback)
    ) {
        return true;
    }
    let body = fact.body.to_lowercase();
    [
        "always ",
        "never ",
        " must ",
        "are called",
        "is called",
        "call them",
        "rule:",
    ]
    .iter()
    .any(|cue| body.contains(cue))
}

/// Full recall of one scope for the `memory` tool: the index plus every fact's
/// body, stamped with age/provenance and (for the project scope) ⚠ staleness
/// annotations. `None` when the scope is empty.
pub async fn recall_scope(
    exec: &dyn Executor,
    mem_dir: &Path,
    label: &str,
    verify_root: Option<&Path>,
) -> Option<String> {
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
        let name = f
            .header
            .name
            .clone()
            .unwrap_or_else(|| f.header.file.clone());
        let kind = f
            .header
            .kind
            .map(|k| format!(" [{}]", k.label()))
            .unwrap_or_default();
        s.push_str(&format!(
            "### {name}{kind} ({}){}\n",
            age_label(f),
            source_marker(f)
        ));
        if let Some(root) = verify_root {
            for warn in verify_fact(exec, root, &f.body).await {
                s.push_str(&warn);
                s.push('\n');
            }
        }
        s.push_str(&format!("{}\n\n", f.body));
    }
    Some(s.trim_end().to_string())
}

/// Deterministic staleness probes for one fact body: file paths that no longer
/// exist and `npm run <script>` commands package.json doesn't define. Cheap,
/// conservative, capped.
async fn verify_fact(exec: &dyn Executor, root: &Path, body: &str) -> Vec<String> {
    const MAX_CANDIDATES: usize = 8;
    const MAX_WARNINGS: usize = 3;
    let mut warnings = Vec::new();

    // Path-shaped tokens: contain '/' or end in a common source extension.
    let mut checked = 0usize;
    let mut seen = std::collections::HashSet::new();
    for raw in body.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | ',' | ';')) {
        let tok = raw.trim_matches(|c: char| matches!(c, '.' | ':' | '"' | '\'' | '*'));
        let looks_pathy = (tok.contains('/') && tok.contains('.'))
            || [".js", ".ts", ".py", ".rs", ".md", ".json", ".toml", ".yaml"]
                .iter()
                .any(|ext| tok.ends_with(ext));
        if tok.is_empty()
            || tok.len() > 120
            || tok.starts_with("http")
            || tok.contains("://")
            || !looks_pathy
            || !seen.insert(tok.to_string())
        {
            continue;
        }
        if checked >= MAX_CANDIDATES || warnings.len() >= MAX_WARNINGS {
            break;
        }
        checked += 1;
        let rel = tok.trim_start_matches("./");
        if exec.metadata(&root.join(rel)).await.is_err() {
            warnings.push(format!(
                "⚠ this note references `{tok}`, which does not exist in the repo — the note may be stale"
            ));
        }
    }

    // `npm run <script>` claims vs. package.json scripts.
    if let Some(pkg) = read_text(exec, &root.join("package.json")).await {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg) {
            let scripts = &json["scripts"];
            let mut rest = body;
            while let Some(pos) = rest.find("npm run ") {
                rest = &rest[pos + "npm run ".len()..];
                let script: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
                    .collect();
                if !script.is_empty()
                    && scripts.get(&script).is_none()
                    && warnings.len() < MAX_WARNINGS
                {
                    warnings.push(format!(
                        "⚠ this note mentions `npm run {script}`, but package.json defines no such script — the note may be stale"
                    ));
                }
            }
        }
    }
    warnings
}

/// Days elapsed since an ISO `YYYY-MM-DD` date (negative if in the future).
fn days_since_iso_date(iso: &str) -> Option<i64> {
    let mut parts = iso.trim().splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let saved = days_from_civil(y, m, d)?;
    let now_days = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs()
        / 86_400) as i64;
    Some(now_days - saved)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Today as ISO `YYYY-MM-DD` (UTC), for `saved:` stamps.
pub fn iso_date_today() -> String {
    let days = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)) as i64;
    // Inverse of days_from_civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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
    source: Option<&str>,
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
    // Single-line cap: `truncate_chars` appends a "… [truncated]" line, which
    // inside a frontmatter field would split it across lines and corrupt the
    // header — so cap with an inline ellipsis instead.
    let desc = single_line_ellipsis(&desc, 160);
    let kind_line = kind
        .map(|k| format!("type: {}\n", k.label()))
        .unwrap_or_default();
    let source_line = source
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("source: {}\n", s.trim()))
        .unwrap_or_default();
    let body = format!(
        "---\nname: {}\ndescription: {}\nsaved: {}\n{}{}---\n\n{}\n",
        title.trim(),
        desc,
        iso_date_today(),
        source_line,
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

/// Delete one note by title/slug/filename match: remove the file and its
/// index line. Returns the removed file name, or `None` if nothing matched.
pub async fn delete_memory(
    exec: &dyn Executor,
    mem_dir: &Path,
    query: &str,
) -> Result<Option<String>, String> {
    let want = slugify(query);
    if want.is_empty() {
        return Err("give the title of the note to forget".into());
    }
    let facts = load_facts(exec, mem_dir).await;
    let matched = facts.into_iter().find(|f| {
        let stem = f.header.file.trim_end_matches(".md");
        stem == want
            || stem.contains(&want)
            || f.header
                .name
                .as_deref()
                .map(|n| slugify(n) == want || slugify(n).contains(&want))
                .unwrap_or(false)
    });
    let Some(fact) = matched else {
        return Ok(None);
    };
    let file = fact.header.file.clone();
    exec.remove_file(&mem_dir.join(&file))
        .await
        .map_err(|e| format!("removing note: {e}"))?;
    // Drop the index line pointing at the removed file.
    let index_path = mem_dir.join(INDEX_FILE);
    if let Some(index) = read_text(exec, &index_path).await {
        let kept: Vec<&str> = index
            .lines()
            .filter(|l| !l.contains(&format!("]({file})")))
            .collect();
        let _ = exec
            .write(&index_path, (kept.join("\n") + "\n").as_bytes())
            .await;
    }
    Ok(Some(file))
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
    "You have a durable memory via the `memory` tool. Its schema is deferred: before the \
first memory action in a conversation, call `tool_search` with query `memory`; use the \
activated `memory` tool on the next model call. Use it proactively, not only when asked:\n\
- When the user tells you who they are, what they're building, who it's for, how they \
like to work, or vocabulary they want used, save it with `memory` IN THAT SAME TURN — \
before you finish the coding task — or it is lost when the session ends. Scope \
\"global\" if it holds across their projects, scope \"project\" if it's specific to \
this codebase.\n\
- In a project with no saved notes yet, save the basics early: the stack/framework, the \
build and test commands, and where the entrypoint lives.\n\
- When the user changes or reverses an earlier decision, fix the memory IN THAT SAME \
TURN: save the new fact and use action \"forget\" on the superseded note (or rewrite it). \
Never leave both versions standing — a stale decision that keeps getting recalled is \
worse than no note at all.\n\
- Set source \"user-stated\" for things the user actually said, \"inferred\" for \
conclusions you drew yourself. Never record an inference, a maybe, or someone else's \
opinion as a flat fact — keep attribution and open questions in the note's wording.\n\
- Call action \"recall\" to load saved facts before starting sizable work. Notes carry \
their age and ⚠ markers flag references the current repo no longer has. Apply that \
skepticism to CODE facts only (paths, commands, stack — verify against the repo before \
relying on them). The user's own decisions and preferences — vocabulary, tone, rules, \
who their product is for — are not verifiable in code and stay binding until the user \
changes them: apply them as written.\n\
- Notes record what THIS user said HERE. Never blend in facts from Clark's cloud \
profile, other projects, or your own guesses — a note that mixes sources becomes \
impossible to trust or correct.\n\
- Precedence: what the user says in this conversation outranks local saved notes, which \
outrank Clark's cloud profile (\"Personal memory\"). Cloud-profile facts were extracted \
from the user's other work — when you cite them, attribute them to Clark's profile, and \
never let them override what the user tells you here. Never use cloud-profile facts to \
fill gaps in THIS project either — its product, audience, and copy come from this repo \
and this user's own words; if that context is missing, ask instead of borrowing.\n\
Keep each note a durable, reusable fact — never transient task state.\n"
}

/// Cap `s` at `max` chars on one line, ending with an ellipsis when cut.
fn single_line_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
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
#[path = "memory_tests.rs"]
mod tests;
