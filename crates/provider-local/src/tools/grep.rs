//! `grep` — regex search across project files. Uses `walkdir` (via the executor)
//! to recurse and the `grep-searcher`/`grep-regex` crates — the same matching
//! library ripgrep itself is built on — for matching; skips the same noisy
//! directories as `glob`.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use futures::StreamExt;
use grep_regex::RegexMatcher;
use grep_searcher::{sinks, BinaryDetection, SearcherBuilder};
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_MATCHES: usize = 200;
/// Skip files larger than this when scanning (likely binaries/assets).
const MAX_FILE_BYTES: u64 = 2_000_000;
/// How many leading bytes to sniff for a NUL byte (ripgrep's own binary-file
/// heuristic) before deciding a file is binary and skipping it whole.
const BINARY_SNIFF_BYTES: usize = 8_000;

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
        let matcher = match RegexMatcher::new(&pattern) {
            Ok(m) => m,
            Err(e) => return ToolOutcome::error(format!("invalid regex `{pattern}`: {e}")),
        };
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(0))
            .build();
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
        // local machine, or on the remote host for a remote project.
        let entries = match ctx.executor.walk(&base).await {
            Ok(e) => e,
            Err(e) => return ToolOutcome::error(e),
        };

        // Pre-filter by name/size so the read stage touches only candidates.
        let candidates: Vec<_> = entries
            .iter()
            .filter(|entry| {
                if entry.len > MAX_FILE_BYTES {
                    return false;
                }
                match &name_filter {
                    Some(filter) => entry
                        .path
                        .file_name()
                        .map(|n| filter.matches(&n.to_string_lossy()))
                        .unwrap_or(false),
                    None => true,
                }
            })
            .collect();
        let candidate_count = candidates.len();

        // Read files concurrently. For a remote project each read is a network
        // round-trip over the SSH tunnel — awaiting them one at a time made a
        // big search take N × RTT with zero UI progress ("working… then burst").
        // In-flight reads overlap; results are still processed in walk order.
        let read_concurrency = if ctx.executor.is_local() { 8 } else { 32 };
        let read_futures: Vec<_> = candidates
            .into_iter()
            .map(|entry| {
                let executor = ctx.executor.clone();
                async move { (entry, executor.read(&entry.path).await) }
            })
            .collect();
        let mut reads = futures::stream::iter(read_futures).buffered(read_concurrency);

        let mut scanned = 0usize;
        'walk: while let Some((entry, read)) = reads.next().await {
            if ctx.cancel.is_cancelled() {
                break;
            }
            scanned += 1;
            if scanned % 64 == 0 {
                ctx.report(format!(
                    "searched {scanned}/{candidate_count} files · {total} matches\n"
                ));
            }
            let path = entry.path.as_path();
            let Ok(bytes) = read else {
                continue; // unreadable
            };
            // Ripgrep's own binary-file heuristic: a NUL byte in the first few
            // KB means binary — skip the whole file rather than searching it.
            // This is the authoritative skip decision; `binary_detection`
            // above is defense in depth for a NUL byte appearing later in a
            // file that passed this upfront sniff.
            if bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
                continue;
            }

            let mut file_count = 0usize;
            let mut file_lines: Vec<String> = Vec::new();
            let search_result = searcher.search_slice(
                &matcher,
                &bytes,
                sinks::Lossy(|line_number: u64, line: &str| {
                    file_count += 1;
                    total += 1;
                    if mode == "content" {
                        file_lines.push(format!(
                            "{}:{}: {}",
                            rel(path),
                            line_number,
                            line.trim_end().chars().take(400).collect::<String>()
                        ));
                    }
                    // `false` stops the search for THIS file only; the outer
                    // 'walk loop is broken separately below once we know
                    // whether this file pushed us over MAX_MATCHES.
                    Ok(total < MAX_MATCHES)
                }),
            );
            if search_result.is_err() {
                continue; // treat a stream error like today's "skip unreadable"
            }
            content_lines.extend(file_lines);
            if file_count > 0 {
                files_with_matches.push(rel(path));
                counts.push((rel(path), file_count));
            }
            if total >= MAX_MATCHES {
                truncated = true;
                break 'walk;
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
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
            progress: None,
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

    #[tokio::test]
    async fn skips_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        // A NUL byte in the first few KB marks this as binary — ripgrep's own
        // heuristic — even though "needle" appears in it as plain bytes.
        let mut binary = b"needle".to_vec();
        binary.push(0);
        binary.extend_from_slice(b"more needle bytes");
        std::fs::write(dir.path().join("blob.bin"), &binary).unwrap();
        std::fs::write(dir.path().join("text.txt"), "needle").unwrap();
        let out = Grep
            .invoke(json!({"pattern": "needle"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("text.txt"));
        assert!(!out.content.contains("blob.bin"));
    }

    #[tokio::test]
    async fn lossily_decodes_non_utf8_matches_instead_of_skipping_the_file() {
        let dir = tempfile::tempdir().unwrap();
        // A stray invalid UTF-8 byte (no NUL) inside an otherwise-text file:
        // today's strict `String::from_utf8` would skip this file entirely,
        // losing the "needle" match. The lossy decoder should still find it,
        // with the bad byte replaced rather than causing an error.
        let mut text = b"needle before ".to_vec();
        text.push(0xFF); // invalid UTF-8 continuation byte, not a NUL
        text.extend_from_slice(b" after\n");
        std::fs::write(dir.path().join("mostly_text.rs"), &text).unwrap();
        let out = Grep
            .invoke(json!({"pattern": "needle"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("mostly_text.rs:1:"));
        assert!(out.content.contains("needle before"));
    }

    #[tokio::test]
    async fn count_mode_counts_every_match_not_just_first_per_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\nneedle\nneedle\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "needle\n").unwrap();
        let out = Grep
            .invoke(
                json!({"pattern": "needle", "output_mode": "count"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.content.contains("a.txt: 3"));
        assert!(out.content.contains("b.txt: 1"));
    }
}
