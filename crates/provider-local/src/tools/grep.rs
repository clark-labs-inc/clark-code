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

mod ripgrep;

/// How many leading bytes to sniff for a NUL byte (ripgrep's own binary-file
/// heuristic) before deciding a file is binary and skipping it whole.
const BINARY_SNIFF_BYTES: usize = 8_000;

/// Keep one broad search from consuming the model's entire context window.
/// The complete match count is still reported so the model can narrow the
/// query deliberately instead of mistaking the bounded prefix for completeness.
pub(super) const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const TRUNCATION_NOTICE_RESERVE_BYTES: usize = 512;

#[derive(Default)]
pub(super) struct BoundedGrepOutput {
    content: String,
    kept_rows: usize,
    total_rows: usize,
    truncated: bool,
}

impl BoundedGrepOutput {
    pub(super) fn push(&mut self, line: String) {
        self.total_rows = self.total_rows.saturating_add(1);
        if self.truncated {
            return;
        }

        let budget = MAX_OUTPUT_BYTES.saturating_sub(TRUNCATION_NOTICE_RESERVE_BYTES);
        let separator = usize::from(!self.content.is_empty());
        if self
            .content
            .len()
            .saturating_add(separator)
            .saturating_add(line.len())
            <= budget
        {
            if separator > 0 {
                self.content.push('\n');
            }
            self.content.push_str(&line);
            self.kept_rows += 1;
            return;
        }

        if self.content.is_empty() {
            self.content.push_str(&bounded_row(&line, budget));
            self.kept_rows = 1;
        }
        self.truncated = true;
    }

    pub(super) fn total_rows(&self) -> usize {
        self.total_rows
    }

    pub(super) fn is_empty(&self) -> bool {
        self.total_rows == 0
    }

    pub(super) fn finish(mut self, match_count: usize, mode: &str) -> String {
        if !self.truncated {
            return self.content;
        }

        let omitted_rows = self.total_rows.saturating_sub(self.kept_rows);
        let refinement = match mode {
            "content" => {
                "Narrow `path`/`glob` or the pattern, or use `files_with_matches`/`count`."
            }
            _ => "Narrow `path`/`glob` or the pattern.",
        };
        self.content.push_str(&format!(
            "\n\n[grep output bounded at {MAX_OUTPUT_BYTES} bytes: showing {} of {} result rows; \
             {omitted_rows} omitted; {match_count} total matches. {refinement}]",
            self.kept_rows, self.total_rows,
        ));
        debug_assert!(self.content.len() <= MAX_OUTPUT_BYTES);
        self.content
    }
}

fn bounded_row(line: &str, budget: usize) -> String {
    const MARKER: &str = "\n[matched row clipped]\n";
    if line.len() <= budget {
        return line.to_string();
    }
    if budget <= MARKER.len() {
        return MARKER[..budget].to_string();
    }

    let content_budget = budget - MARKER.len();
    let head_budget = content_budget / 2;
    let tail_budget = content_budget - head_budget;
    let head_end = floor_char_boundary(line, head_budget);
    let tail_start = ceil_char_boundary(line, line.len().saturating_sub(tail_budget));
    format!("{}{}{}", &line[..head_end], MARKER, &line[tail_start..])
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub struct Grep;

#[async_trait]
impl ToolExecutor for Grep {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search project file contents with a regular expression. Returns a bounded set of matching lines as `path:line: text` plus the complete match count. Scope with `path`, filter filenames with `glob`, or use `files_with_matches`/`count` for broad searches."
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

        // Packaged local sessions use the pinned ripgrep sidecar. The existing
        // library implementation remains the remote executor and source-build
        // fallback, preserving the same tool contract everywhere.
        if let Some(outcome) =
            ripgrep::search(&pattern, &base, name_filter.as_ref(), &mode, ctx).await
        {
            return outcome;
        }

        let rel = |p: &std::path::Path| -> String {
            crate::sandbox::model_path(
                p.strip_prefix(&root)
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|_| p.display().to_string()),
            )
        };

        let mut output = BoundedGrepOutput::default();
        let mut total = 0usize;

        // The executor yields files only, already skipping ignored dirs — on the
        // local machine, or on the remote host for a remote project.
        let entries = match ctx.executor.walk(&base).await {
            Ok(e) => e,
            Err(e) => return ToolOutcome::error(e),
        };

        // Pre-filter by name so the read stage touches only candidates.
        let candidates: Vec<_> = entries
            .iter()
            .filter(|entry| match &name_filter {
                Some(filter) => entry
                    .path
                    .file_name()
                    .map(|n| filter.matches(&n.to_string_lossy()))
                    .unwrap_or(false),
                None => true,
            })
            .collect();
        let candidate_count = candidates.len();

        // Read files concurrently. For a remote project each read is a network
        // round-trip through the remote worker — awaiting them one at a time made a
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
        while let Some((entry, read)) = reads.next().await {
            if ctx.cancel.is_cancelled() {
                break;
            }
            scanned += 1;
            if scanned.is_multiple_of(64) {
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
            let search_result = searcher.search_slice(
                &matcher,
                &bytes,
                sinks::Lossy(|line_number: u64, line: &str| {
                    file_count += 1;
                    total += 1;
                    if mode == "content" {
                        output.push(format!(
                            "{}:{}: {}",
                            rel(path),
                            line_number,
                            line.trim_end()
                        ));
                    }
                    Ok(true)
                }),
            );
            if search_result.is_err() {
                continue; // treat a stream error like today's "skip unreadable"
            }
            if file_count > 0 {
                match mode.as_str() {
                    "files_with_matches" => output.push(rel(path)),
                    "count" => output.push(format!("{}: {file_count}", rel(path))),
                    _ => {}
                }
            }
        }

        if total == 0 {
            return ToolOutcome::ok(format!("(no matches for `{pattern}`)"));
        }
        ToolOutcome::ok(output.finish(total, &mode))
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
            agent_progress: None,
            call_progress: None,
            model_override: None,
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
    async fn preserves_all_matches_and_complete_long_lines_within_bound() {
        let dir = tempfile::tempdir().unwrap();
        let mut content = format!("needle {} LONG_LINE_SENTINEL\n", "x".repeat(450));
        for index in 0..205 {
            content.push_str(&format!("needle match-{index:03}\n"));
        }
        std::fs::write(dir.path().join("many.txt"), content).unwrap();
        let out = Grep
            .invoke(json!({"pattern": "needle"}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content.lines().count(), 206);
        assert!(out.content.contains("LONG_LINE_SENTINEL"));
        assert!(out.content.contains("match-204"));
        assert!(!out.content.contains("truncated"));
    }

    #[tokio::test]
    async fn bounds_broad_results_and_tells_the_model_how_to_refine() {
        let dir = tempfile::tempdir().unwrap();
        let content = (0..2_000)
            .map(|index| format!("needle match-{index:04} {}\n", "x".repeat(80)))
            .collect::<String>();
        std::fs::write(dir.path().join("many.txt"), content).unwrap();

        let out = Grep
            .invoke(json!({"pattern": "needle"}), &ctx(dir.path()))
            .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.len() <= MAX_OUTPUT_BYTES);
        assert!(out.content.contains("needle match-0000"));
        assert!(out.content.contains("showing"));
        assert!(out.content.contains("2000 total matches"));
        assert!(out.content.contains("Narrow `path`/`glob`"));
        assert!(!out.content.contains("needle match-1999"));
    }

    #[test]
    fn a_single_giant_utf8_row_is_clipped_inside_the_same_bound() {
        let mut output = BoundedGrepOutput::default();
        output.push(format!(
            "src/data.jsonl:1: {}",
            "é".repeat(MAX_OUTPUT_BYTES)
        ));
        let rendered = output.finish(1, "content");

        assert!(rendered.len() <= MAX_OUTPUT_BYTES);
        assert!(rendered.contains("[matched row clipped]"));
        assert!(rendered.contains("1 total matches"));
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
    async fn honors_repository_ignore_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("generated")).unwrap();
        std::fs::write(dir.path().join("generated/decoy.rs"), "needle").unwrap();
        std::fs::write(dir.path().join("source.rs"), "needle").unwrap();

        let out = Grep
            .invoke(json!({"pattern": "needle"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("source.rs"));
        assert!(!out.content.contains("decoy.rs"));
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
