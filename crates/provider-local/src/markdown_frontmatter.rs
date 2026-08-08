//! Small shared helpers for reading project config through the session
//! process-local [`Executor`] and
//! parsing the `---`-fenced YAML-ish frontmatter Claude/Agent Desktop markdown
//! conventions use (`SKILL.md`, `.claude/commands/*.md`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::exec::Executor;

pub async fn read_text(exec: &dyn Executor, path: &Path) -> Option<String> {
    String::from_utf8(exec.read(path).await.ok()?).ok()
}

pub async fn read_json(exec: &dyn Executor, path: &Path) -> Option<Value> {
    serde_json::from_str(&read_text(exec, path).await?).ok()
}

/// `$HOME` on the executor's target machine (local or remote).
pub async fn resolve_home(exec: &dyn Executor, cwd: &Path) -> Option<PathBuf> {
    exec.home_dir(cwd).await.ok()
}

/// The YAML-ish frontmatter between the leading `---` fences.
pub fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

pub fn fm_field(fm: &str, key: &str) -> Option<String> {
    for line in fm.lines() {
        if let Some(v) = line.trim().strip_prefix(&format!("{key}:")) {
            let v = v.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// The markdown body after the frontmatter fence (or the whole text if there
/// is none).
pub fn body_after_frontmatter(text: &str) -> &str {
    let Some(fm) = frontmatter(text) else {
        return text.trim();
    };
    // `frontmatter` finds `\n---` right after the fenced block; skip past that
    // closing fence line to get the body.
    let after_open = text.trim_start().strip_prefix("---").unwrap_or(text);
    let end = fm.len();
    after_open[end..]
        .strip_prefix("\n---")
        .unwrap_or(&after_open[end..])
        .trim_start_matches(['\r', '\n'])
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_and_body_split_correctly() {
        let text = "---\nname: foo\ndescription: bar\n---\n\nThe body.\nMore.";
        let fm = frontmatter(text).unwrap();
        assert_eq!(fm_field(fm, "name").as_deref(), Some("foo"));
        assert_eq!(fm_field(fm, "description").as_deref(), Some("bar"));
        assert_eq!(body_after_frontmatter(text), "The body.\nMore.");
    }

    #[test]
    fn body_without_frontmatter_is_the_whole_text() {
        assert_eq!(body_after_frontmatter("just a body"), "just a body");
    }
}
