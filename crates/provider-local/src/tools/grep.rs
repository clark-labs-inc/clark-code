//! `grep` — regex search across project files. Uses `walkdir` to recurse and the
//! `regex` crate for matching; skips the same noisy directories as `glob`.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_MATCHES: usize = 200;
/// Skip files larger than this when scanning (likely binaries/assets).
const MAX_FILE_BYTES: u64 = 2_000_000;

pub struct Grep;

#[async_trait]
impl ToolExecutor for Grep {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search project file contents with a regular expression. Returns matching lines as `path:line: text`. Scope with `path` and filter filenames with `glob`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regular expression to search for."},
                "path": {"type": "string", "description": "Directory or file to scope the search to (defaults to project root)."},
                "glob": {"type": "string", "description": "Only search files whose name matches this glob (e.g. `*.rs`)."},
                "output_mode": {"type": "string", "enum": ["content", "files_with_matches", "count"], "description": "content (default): matching lines; files_with_matches: just paths; count: per-file counts."}
            },
            "required": ["pattern"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let pattern = match arg_str(&args, "pattern") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let re = match regex::Regex::new(&pattern) {
            Ok(re) => re,
            Err(e) => return ToolOutcome::error(format!("invalid regex `{pattern}`: {e}")),
        };
        let scope = arg_str_opt(&args, "path").unwrap_or_else(|| ".".to_string());
        let base = match ctx.sandbox.resolve_existing(&scope) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let name_filter = arg_str_opt(&args, "glob").and_then(|g| glob::Pattern::new(&g).ok());
        let mode = arg_str_opt(&args, "output_mode").unwrap_or_else(|| "content".to_string());
        let root = ctx.sandbox.root().to_path_buf();

        let rel = |p: &std::path::Path| -> String {
            p.strip_prefix(&root)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| p.display().to_string())
        };

        let mut content_lines: Vec<String> = Vec::new();
        let mut files_with_matches: Vec<String> = Vec::new();
        let mut counts: Vec<(String, usize)> = Vec::new();
        let mut total = 0usize;
        let mut truncated = false;

        // The executor yields files only, already skipping ignored dirs — on the
        // local machine today, on the remote host once `RemoteExecutor` lands.
        let entries = match ctx.executor.walk(&base).await {
            Ok(e) => e,
            Err(e) => return ToolOutcome::error(e),
        };

        'walk: for entry in &entries {
            if ctx.cancel.is_cancelled() {
                break;
            }
            let path = entry.path.as_path();
            if let Some(filter) = &name_filter {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string());
                if !name.map(|n| filter.matches(&n)).unwrap_or(false) {
                    continue;
                }
            }
            if entry.len > MAX_FILE_BYTES {
                continue;
            }
            let Ok(text) = ctx
                .executor
                .read(path)
                .await
                .and_then(|b| String::from_utf8(b).map_err(|_| "non-utf8".to_string()))
            else {
                continue; // skip non-UTF-8 / binary / unreadable
            };
            let mut file_count = 0usize;
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    file_count += 1;
                    total += 1;
                    if mode == "content" {
                        content_lines.push(format!(
                            "{}:{}: {}",
                            rel(path),
                            i + 1,
                            line.trim_end().chars().take(400).collect::<String>()
                        ));
                    }
                    if total >= MAX_MATCHES {
                        truncated = true;
                        if file_count > 0 && !files_with_matches.contains(&rel(path)) {
                            files_with_matches.push(rel(path));
                        }
                        break 'walk;
                    }
                }
            }
            if file_count > 0 {
                files_with_matches.push(rel(path));
                counts.push((rel(path), file_count));
            }
        }

        if total == 0 {
            return ToolOutcome::ok(format!("(no matches for `{pattern}`)"));
        }
        let mut body = match mode.as_str() {
            "files_with_matches" => files_with_matches.join("\n"),
            "count" => counts
                .iter()
                .map(|(p, c)| format!("{p}: {c}"))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => content_lines.join("\n"),
        };
        if truncated {
            body.push_str(&format!("\n… [truncated at {MAX_MATCHES} matches]"));
        }
        ToolOutcome::ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(dir).unwrap()),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            executor: Arc::new(crate::exec::LocalExecutor),
        }
    }

    #[tokio::test]
    async fn finds_matching_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn alpha() {}\nfn beta() {}").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "let x = 1;").unwrap();
        let out = Grep
            .invoke(json!({"pattern": "fn \\w+"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("src/a.rs:1: fn alpha"));
        assert!(out.content.contains("src/a.rs:2: fn beta"));
        assert!(!out.content.contains("b.rs"));
    }

    #[tokio::test]
    async fn files_with_matches_mode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle").unwrap();
        std::fs::write(dir.path().join("b.txt"), "hay").unwrap();
        let out = Grep
            .invoke(
                json!({"pattern": "needle", "output_mode": "files_with_matches"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.content.contains("a.txt"));
        assert!(!out.content.contains("b.txt"));
    }

    #[tokio::test]
    async fn glob_filter_restricts_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "target").unwrap();
        std::fs::write(dir.path().join("a.md"), "target").unwrap();
        let out = Grep
            .invoke(
                json!({"pattern": "target", "glob": "*.rs"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.content.contains("a.rs"));
        assert!(!out.content.contains("a.md"));
    }
}
