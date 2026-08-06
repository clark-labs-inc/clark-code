//! Local filesystem tools: read, list, glob, write, edit. All paths resolve
//! through the [`Sandbox`](crate::sandbox::Sandbox) so they cannot escape the
//! project root.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};

pub struct ReadFile;

#[async_trait]
impl ToolExecutor for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a UTF-8 text file from the project. Returns the content with 1-based line numbers. Use `offset`/`limit` to page through large files. It is fine to attempt reading a file that may not exist — an error is returned if so. Read a file before editing or overwriting it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the project root."},
                "offset": {"type": "integer", "description": "1-based first line to return (optional)."},
                "limit": {"type": "integer", "description": "Maximum number of lines to return (optional)."}
            },
            "required": ["path"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let path = match arg_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let resolved = match ctx.sandbox.resolve_existing(&path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let bytes = match ctx.executor.read(&resolved).await {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(format!("{path}: {e}")),
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) if !text.contains('\0') => text,
            _ => {
                return ToolOutcome::error(format!(
                    "{path} is binary, not a UTF-8 text file; use an image or binary-aware tool"
                ));
            }
        };

        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize);

        let mut out = String::new();
        let mut shown = 0usize;
        for (i, line) in text.lines().enumerate() {
            let lineno = i + 1;
            if lineno < offset {
                continue;
            }
            if let Some(limit) = limit {
                if shown >= limit {
                    break;
                }
            }
            out.push_str(&format!("{lineno:>6}\t{line}\n"));
            shown += 1;
        }
        if out.is_empty() {
            out.push_str("(empty or no lines in range)");
        }
        // Mark the file read so the model may now edit/overwrite it.
        ctx.note_read(&resolved).await;
        let loc = ctx.sandbox.display(&resolved);
        ToolOutcome::ok(out).with_location(loc, Some(offset as u32))
    }
}

pub struct ListDir;

#[async_trait]
impl ToolExecutor for ListDir {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        "List the immediate entries of a directory in the project. Directories are suffixed with `/`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory path relative to the project root (defaults to root)."}
            },
            "required": []
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let path = arg_str_opt(&args, "path").unwrap_or_else(|| ".".to_string());
        let resolved = match ctx.sandbox.resolve_existing(&path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let entries = match ctx.executor.read_dir(&resolved).await {
            Ok(e) => e,
            Err(e) => return ToolOutcome::error(format!("{path}: {e}")),
        };
        let mut names: Vec<String> = Vec::new();
        for entry in entries {
            names.push(if entry.is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name
            });
        }
        names.sort();
        let body = if names.is_empty() {
            "(empty directory)".to_string()
        } else {
            names.join("\n")
        };
        ToolOutcome::ok(body).with_location(path, None)
    }
}

pub struct Glob;

#[async_trait]
impl ToolExecutor for Glob {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Find files by glob pattern (e.g. `**/*.rs`, `src/**/mod.rs`), rooted at the project. Returns matching paths, most recently modified first. Prefer this over `bash` with `find`/`ls`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern relative to the project root."}
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
        let root = ctx.sandbox.root().to_path_buf();
        // Match the pattern (relative to the root) against the project's files.
        // `walk` already skips ignored dirs; this also makes glob work remotely.
        let matcher = match glob::Pattern::new(&pattern) {
            Ok(m) => m,
            Err(e) => return ToolOutcome::error(format!("invalid glob `{pattern}`: {e}")),
        };
        let entries = match ctx.executor.walk(&root).await {
            Ok(e) => e,
            Err(e) => return ToolOutcome::error(e),
        };
        let mut hits: Vec<(std::time::SystemTime, String)> = Vec::new();
        for entry in entries {
            let Ok(rel) = entry.path.strip_prefix(&root) else {
                continue;
            };
            if !matcher.matches_path(rel) {
                continue;
            }
            let mtime = entry.modified.unwrap_or(std::time::UNIX_EPOCH);
            hits.push((mtime, crate::sandbox::model_path(rel.display().to_string())));
        }
        // Most recently modified first — the files an agent most likely wants.
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let body = if hits.is_empty() {
            format!("(no files match `{pattern}`)")
        } else {
            hits.into_iter()
                .map(|(_, p)| p)
                .collect::<Vec<_>>()
                .join("\n")
        };
        ToolOutcome::ok(body)
    }
}

pub struct WriteFile;

#[async_trait]
impl ToolExecutor for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Create a new file, or overwrite an existing one with the given full content. If the file already exists you MUST read_file it first; this tool fails otherwise. Prefer edit_file for surgical changes to existing files."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the project root."},
                "content": {"type": "string", "description": "The complete new file contents."}
            },
            "required": ["path", "content"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn mutating(&self) -> bool {
        true
    }
    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Option<String> {
        let path = arg_str(args, "path").ok()?;
        let content = arg_str(args, "content").ok()?;
        let resolved = ctx.sandbox.resolve_for_write(&path).ok()?;
        let original = std::fs::read_to_string(&resolved).unwrap_or_default();
        unified_diff(&path, &original, &content)
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let path = match arg_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let content = match arg_str(&args, "content") {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };
        let resolved = match ctx.sandbox.resolve_for_write(&path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let existed = resolved.exists();
        // Capture the prior contents so the result is a real diff (added/removed
        // lines), matching edit_file. New files diff against empty → all adds.
        let original = if existed {
            ctx.executor
                .read(&resolved)
                .await
                .map(|b| String::from_utf8_lossy(&b).to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        // Overwriting an existing file requires having read it first; new files
        // are exempt. Key by the canonical path to match read_file's records.
        let key = if existed {
            resolved.canonicalize().unwrap_or_else(|_| resolved.clone())
        } else {
            resolved.clone()
        };
        if let Err(e) = ctx.guard_mutation(&key, false).await {
            return ToolOutcome::error(e);
        }
        if let Some(parent) = resolved.parent() {
            if let Err(e) = ctx.executor.create_dir_all(parent).await {
                return ToolOutcome::error(format!("creating {}: {e}", parent.display()));
            }
        }
        if let Err(e) = ctx.executor.write(&resolved, content.as_bytes()).await {
            return ToolOutcome::error(format!("{path}: {e}"));
        }
        // The file now reflects what the model wrote, so further edits are safe.
        if let Ok(canon) = resolved.canonicalize() {
            ctx.note_read(&canon).await;
        }
        let verb = if existed { "Overwrote" } else { "Created" };
        let summary = format!("{verb} {path} ({} bytes).", content.len());
        edit_result(&path, &original, &content, summary, 1)
    }
}

pub struct EditFile;

#[async_trait]
impl ToolExecutor for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Replace an exact substring in a file. You must read_file the file at least once before editing it; this tool fails otherwise. `old_string` must appear exactly once (include enough surrounding context to make it unique), unless `replace_all` is true."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the project root."},
                "old_string": {"type": "string", "description": "The exact text to find and replace."},
                "new_string": {"type": "string", "description": "The replacement text."},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence instead of requiring a unique match (default false)."}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn mutating(&self) -> bool {
        true
    }
    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Option<String> {
        let path = arg_str(args, "path").ok()?;
        let old = arg_str(args, "old_string").ok()?;
        let new = arg_str(args, "new_string").ok()?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let resolved = ctx.sandbox.resolve_existing(&path).ok()?;
        let original = std::fs::read_to_string(&resolved).ok()?;
        if !original.contains(&old) {
            return None;
        }
        let updated = if replace_all {
            original.replace(&old, &new)
        } else {
            original.replacen(&old, &new, 1)
        };
        unified_diff(&path, &original, &updated)
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let path = match arg_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let old = match arg_str(&args, "old_string") {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let new = match arg_str(&args, "new_string") {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if old == new {
            return ToolOutcome::error("old_string and new_string are identical");
        }
        let resolved = match ctx.sandbox.resolve_existing(&path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        if let Err(e) = ctx.guard_mutation(&resolved, true).await {
            return ToolOutcome::error(e);
        }
        let original = match ctx.executor.read(&resolved).await {
            Ok(b) => String::from_utf8_lossy(&b).to_string(),
            Err(e) => return ToolOutcome::error(format!("{path}: {e}")),
        };
        let count = original.matches(&old).count();
        if count == 0 {
            return ToolOutcome::error(format!("old_string not found in {path}"));
        }
        if count > 1 && !replace_all {
            return ToolOutcome::error(format!(
                "old_string occurs {count} times in {path}; add more context to make it unique or set replace_all=true"
            ));
        }
        let updated = if replace_all {
            original.replace(&old, &new)
        } else {
            original.replacen(&old, &new, 1)
        };
        if let Err(e) = ctx.executor.write(&resolved, updated.as_bytes()).await {
            return ToolOutcome::error(format!("{path}: {e}"));
        }
        // Refresh the read record so chained edits to the same file are allowed.
        ctx.note_read(&resolved).await;
        let line = original[..original.find(&old).unwrap_or(0)]
            .matches('\n')
            .count() as u32
            + 1;
        let n = if replace_all { count } else { 1 };
        let summary = format!(
            "Edited {path} ({n} replacement{}).",
            if n == 1 { "" } else { "s" }
        );
        edit_result(&path, &original, &updated, summary, line)
    }
}

/// A `diff <path>\n@@…` unified diff string, or None when there's no change.
fn unified_diff(path: &str, original: &str, updated: &str) -> Option<String> {
    let unified = similar::TextDiff::from_lines(original, updated)
        .unified_diff()
        .context_radius(3)
        .to_string();
    if unified.trim().is_empty() {
        None
    } else {
        Some(format!("diff {path}\n{unified}"))
    }
}

/// Result for a successful edit: a unified diff (so the model sees exactly what
/// changed and the UI renders it red/green), falling back to the plain summary
/// only when there is no textual change.
fn edit_result(
    path: &str,
    original: &str,
    updated: &str,
    summary: String,
    line: u32,
) -> ToolOutcome {
    let content = unified_diff(path, original, updated).unwrap_or(summary);
    ToolOutcome::ok(content).with_location(path.to_string(), Some(line))
}

/// Skip noisy/large directories that a coding agent rarely wants surfaced.
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
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
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

    async fn read(c: &ToolCtx, path: &str) {
        let out = ReadFile.invoke(json!({ "path": path }), c).await;
        assert!(!out.is_error, "setup read failed: {}", out.content);
    }

    #[tokio::test]
    async fn read_file_returns_numbered_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree").unwrap();
        let out = ReadFile
            .invoke(json!({"path": "a.txt"}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("     1\tone"));
        assert!(out.content.contains("     3\tthree"));
    }

    #[tokio::test]
    async fn read_file_preserves_content_beyond_the_old_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!("{}END_OF_FILE_SENTINEL", "x".repeat(210_000));
        std::fs::write(dir.path().join("large.txt"), content).unwrap();
        let out = ReadFile
            .invoke(json!({"path": "large.txt"}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("END_OF_FILE_SENTINEL"));
        assert!(!out.content.contains("truncated"));
    }

    #[tokio::test]
    async fn read_file_honors_offset_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "l1\nl2\nl3\nl4").unwrap();
        let out = ReadFile
            .invoke(
                json!({"path": "a.txt", "offset": 2, "limit": 2}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.content.contains("     2\tl2"));
        assert!(out.content.contains("     3\tl3"));
        assert!(!out.content.contains("l1"));
        assert!(!out.content.contains("l4"));
    }

    #[tokio::test]
    async fn read_file_rejects_binary_without_emitting_nul() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("image.png"), b"\x89PNG\r\n\x1a\n\0binary").unwrap();
        let out = ReadFile
            .invoke(json!({"path": "image.png"}), &ctx(dir.path()))
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("is binary"));
        assert!(!out.content.contains('\0'));
    }

    #[tokio::test]
    async fn list_dir_preserves_every_entry_beyond_the_old_cap() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..1_005 {
            std::fs::write(dir.path().join(format!("entry-{index:04}.txt")), "").unwrap();
        }
        let out = ListDir.invoke(json!({}), &ctx(dir.path())).await;
        assert!(!out.is_error);
        assert_eq!(out.content.lines().count(), 1_005);
        assert!(out.content.contains("entry-1004.txt"));
        assert!(!out.content.contains("truncated"));
    }

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let out = WriteFile
            .invoke(json!({"path": "sub/new.txt", "content": "hello"}), &c)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub/new.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn edit_requires_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x x x").unwrap();
        let c = ctx(dir.path());
        read(&c, "a.txt").await;
        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "x", "new_string": "y"}),
                &c,
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("occurs 3 times"));

        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "x", "new_string": "y", "replace_all": true}),
                &c,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "y y y"
        );
    }

    #[tokio::test]
    async fn edit_unique_match_succeeds_and_reports_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nbeta\ngamma").unwrap();
        let c = ctx(dir.path());
        read(&c, "a.txt").await;
        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "beta", "new_string": "BETA"}),
                &c,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.locations[0].line, Some(2));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "alpha\nBETA\ngamma"
        );
    }

    #[tokio::test]
    async fn edit_returns_a_unified_diff() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let c = ctx(dir.path());
        read(&c, "a.txt").await;
        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "beta", "new_string": "BETA"}),
                &c,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        // The result is a diff the UI renders red/green and the model can read.
        assert!(out.content.starts_with("diff a.txt"), "{}", out.content);
        assert!(out.content.contains("-beta"));
        assert!(out.content.contains("+BETA"));
    }

    #[tokio::test]
    async fn large_edit_returns_the_complete_unified_diff() {
        let dir = tempfile::tempdir().unwrap();
        let original = (0..1_000)
            .map(|index| format!("old-{index:04}\n"))
            .collect::<String>();
        let updated = (0..1_000)
            .map(|index| format!("new-{index:04}\n"))
            .collect::<String>();
        std::fs::write(dir.path().join("large.txt"), &original).unwrap();
        let c = ctx(dir.path());
        read(&c, "large.txt").await;
        let out = WriteFile
            .invoke(json!({"path": "large.txt", "content": updated}), &c)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.starts_with("diff large.txt"));
        assert!(out.content.contains("-old-0999"));
        assert!(out.content.contains("+new-0999"));
    }

    #[tokio::test]
    async fn edit_without_reading_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "hello", "new_string": "hi"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("has not been read"));
        // File is untouched.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn overwrite_existing_without_reading_is_rejected_but_new_file_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exists.txt"), "old").unwrap();
        let c = ctx(dir.path());
        // Overwriting an existing, unread file fails.
        let out = WriteFile
            .invoke(json!({"path": "exists.txt", "content": "new"}), &c)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("has not been read"));
        // But creating a brand-new file is allowed without a read.
        let out = WriteFile
            .invoke(json!({"path": "fresh.txt", "content": "hi"}), &c)
            .await;
        assert!(!out.is_error, "{}", out.content);
    }

    #[tokio::test]
    async fn reading_then_editing_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let c = ctx(dir.path());
        read(&c, "a.txt").await;
        let out = EditFile
            .invoke(
                json!({"path": "a.txt", "old_string": "hello", "new_string": "hi"}),
                &c,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hi"
        );
    }

    #[tokio::test]
    async fn read_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let out = ReadFile
            .invoke(json!({"path": "../../../etc/hosts"}), &ctx(dir.path()))
            .await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn glob_finds_files_and_skips_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("target/b.rs"), "").unwrap();
        let out = Glob
            .invoke(json!({"pattern": "**/*.rs"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("src/a.rs"));
        assert!(!out.content.contains("target/b.rs"));
    }

    #[tokio::test]
    async fn glob_preserves_every_match_beyond_the_old_cap() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..1_005 {
            std::fs::write(dir.path().join(format!("match-{index:04}.txt")), "").unwrap();
        }
        let out = Glob
            .invoke(json!({"pattern": "*.txt"}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error);
        assert_eq!(out.content.lines().count(), 1_005);
        assert!(out.content.contains("match-1004.txt"));
    }
}
