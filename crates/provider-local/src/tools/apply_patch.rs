//! Atomic-preflight multi-file patch tool using Agent Desktop's `*** Begin Patch`
//! envelope. All existing files must satisfy the same read-before-edit guard as
//! `edit_file`; parsing and replacement computation finish before any write.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_PATCH_BYTES: usize = 256_000;
const PATCH_DESCRIPTION: &str = "The complete Agent Desktop patch body. Every operation header includes its leading `***`. For an update, use `*** Update File: path`, then `@@` without unified-diff line ranges, then lines prefixed with space, `-`, or `+`. Example: `*** Begin Patch\n*** Update File: path.txt\n@@\n-old\n+new\n*** End Patch`. For an add, use `*** Add File: path` and prefix every content line with `+`.";

pub struct ApplyPatch;

#[async_trait]
impl ToolExecutor for ApplyPatch {
    fn name(&self) -> &str {
        "apply_patch"
    }
    fn description(&self) -> &str {
        "Apply one bounded multi-file patch using the exact Agent Desktop patch grammar described by the `patch` parameter. Existing files must be read first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {"type": "string", "description": PATCH_DESCRIPTION}
            },
            "required": ["patch"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn mutating(&self) -> bool {
        true
    }
    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        arg_str(args, "patch").ok()
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let patch = match arg_str(&args, "patch") {
            Ok(patch) => patch,
            Err(error) => return ToolOutcome::error(error),
        };
        if patch.len() > MAX_PATCH_BYTES {
            return ToolOutcome::error(format!("patch exceeds {MAX_PATCH_BYTES} bytes"));
        }
        let operations = match parse_patch(&patch) {
            Ok(operations) => operations,
            Err(error) => return ToolOutcome::error(error),
        };
        let prepared = match prepare(ctx, operations).await {
            Ok(prepared) => prepared,
            Err(error) => return ToolOutcome::error(error),
        };
        match commit(ctx, prepared).await {
            Ok(outcome) => outcome,
            Err(error) => ToolOutcome::error(error),
        }
    }
}

#[derive(Debug, Clone)]
enum Operation {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

#[derive(Debug, Clone, Default)]
struct Chunk {
    context: Option<String>,
    old: Vec<String>,
    new: Vec<String>,
    eof: bool,
}

fn parse_patch(patch: &str) -> Result<Vec<Operation>, String> {
    let lines = patch.trim().lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch")
        || lines.last().map(|line| line.trim()) != Some("*** End Patch")
    {
        return Err("patch must start with `*** Begin Patch` and end with `*** End Patch`".into());
    }
    let mut index = 1;
    let mut operations = Vec::new();
    while index + 1 < lines.len() {
        let line = lines[index].trim_end();
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = safe_path(path)?;
            index += 1;
            let mut content = String::new();
            while index + 1 < lines.len() && !lines[index].starts_with("*** ") {
                let added = lines[index]
                    .strip_prefix('+')
                    .ok_or_else(|| format!("add-file line {} must start with +", index + 1))?;
                content.push_str(added);
                content.push('\n');
                index += 1;
            }
            operations.push(Operation::Add { path, content });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(Operation::Delete {
                path: safe_path(path)?,
            });
            index += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = safe_path(path)?;
            index += 1;
            let move_to = lines
                .get(index)
                .and_then(|line| line.strip_prefix("*** Move to: "))
                .map(safe_path)
                .transpose()?;
            if move_to.is_some() {
                index += 1;
            }
            let mut chunks = Vec::new();
            let mut chunk = Chunk::default();
            while index + 1 < lines.len() && !is_file_header(lines[index]) {
                let value = lines[index];
                if value == "@@" || value.starts_with("@@ ") {
                    push_chunk(&mut chunks, &mut chunk)?;
                    chunk.context = value.strip_prefix("@@ ").map(String::from);
                } else if value == "*** End of File" {
                    chunk.eof = true;
                } else if let Some(value) = value.strip_prefix('+') {
                    chunk.new.push(value.to_string());
                } else if let Some(value) = value.strip_prefix('-') {
                    chunk.old.push(value.to_string());
                } else if let Some(value) = value.strip_prefix(' ') {
                    chunk.old.push(value.to_string());
                    chunk.new.push(value.to_string());
                } else {
                    return Err(format!("invalid update line {}: {value}", index + 1));
                }
                index += 1;
            }
            push_chunk(&mut chunks, &mut chunk)?;
            if chunks.is_empty() && move_to.is_none() {
                return Err(format!("update for {path} is empty"));
            }
            operations.push(Operation::Update {
                path,
                move_to,
                chunks,
            });
        } else {
            return Err(format!(
                "invalid patch header at line {}: {line}",
                index + 1
            ));
        }
    }
    if operations.is_empty() {
        return Err("patch contains no file operations".into());
    }
    Ok(operations)
}

fn is_file_header(line: &str) -> bool {
    line.starts_with("*** Add File: ")
        || line.starts_with("*** Delete File: ")
        || line.starts_with("*** Update File: ")
        || line.trim() == "*** End Patch"
}

fn push_chunk(chunks: &mut Vec<Chunk>, chunk: &mut Chunk) -> Result<(), String> {
    if chunk.context.is_some() || !chunk.old.is_empty() || !chunk.new.is_empty() || chunk.eof {
        if chunk.old == chunk.new && !chunk.old.is_empty() {
            return Err("update chunk contains no changes".into());
        }
        chunks.push(std::mem::take(chunk));
    }
    Ok(())
}

fn safe_path(path: &str) -> Result<String, String> {
    let path = path.trim();
    let parsed = Path::new(path);
    if path.is_empty()
        || parsed.is_absolute()
        || !parsed
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe patch path: {path}"));
    }
    Ok(path.to_string())
}

struct Prepared {
    source: PathBuf,
    destination: PathBuf,
    display: String,
    old: Option<String>,
    new: Option<String>,
    kind: &'static str,
    remove_source: bool,
}

async fn prepare(ctx: &ToolCtx, operations: Vec<Operation>) -> Result<Vec<Prepared>, String> {
    let mut seen = HashSet::new();
    let mut prepared = Vec::new();
    for operation in operations {
        let (path, move_to) = match &operation {
            Operation::Add { path, .. } | Operation::Delete { path } => (path.clone(), None),
            Operation::Update { path, move_to, .. } => (path.clone(), move_to.clone()),
        };
        if !seen.insert(path.clone())
            || move_to
                .as_ref()
                .is_some_and(|path| !seen.insert(path.clone()))
        {
            return Err(format!("patch touches {path} more than once"));
        }
        let source = ctx.sandbox.resolve_for_write(&path)?;
        let metadata = ctx.executor.metadata(&source).await.ok();
        if metadata
            .as_ref()
            .is_some_and(|meta| meta.is_dir && !meta.is_symlink)
        {
            return Err(format!("{path} is a directory"));
        }
        match operation {
            Operation::Add { content, .. } => {
                if metadata.is_some() {
                    return Err(format!("cannot add {path}: file already exists"));
                }
                prepared.push(Prepared {
                    source: source.clone(),
                    destination: source,
                    display: path.clone(),
                    old: None,
                    new: Some(content),
                    kind: "add",
                    remove_source: false,
                });
            }
            Operation::Delete { .. } => {
                let original = read_existing(ctx, &source, &path).await?;
                prepared.push(Prepared {
                    source: source.clone(),
                    destination: source,
                    display: path.clone(),
                    old: Some(original),
                    new: None,
                    kind: "delete",
                    remove_source: true,
                });
            }
            Operation::Update { chunks, .. } => {
                let original = read_existing(ctx, &source, &path).await?;
                let updated = apply_chunks(&original, &chunks, &path)?;
                let moved = move_to.is_some();
                let destination = match move_to.as_ref() {
                    Some(destination) => {
                        let resolved = ctx.sandbox.resolve_for_write(destination)?;
                        if ctx.executor.metadata(&resolved).await.is_ok() {
                            return Err(format!("move destination {destination} already exists"));
                        }
                        resolved
                    }
                    None => source.clone(),
                };
                prepared.push(Prepared {
                    source: source.clone(),
                    destination,
                    display: move_to.unwrap_or_else(|| path.clone()),
                    old: Some(original),
                    new: Some(updated),
                    kind: if moved { "move" } else { "update" },
                    remove_source: moved,
                });
            }
        }
    }
    Ok(prepared)
}

async fn read_existing(ctx: &ToolCtx, path: &Path, display: &str) -> Result<String, String> {
    ctx.guard_mutation(path, true).await?;
    let bytes = ctx
        .executor
        .read(path)
        .await
        .map_err(|error| format!("{display}: {error}"))?;
    String::from_utf8(bytes).map_err(|_| format!("{display} is not UTF-8 text"))
}

fn apply_chunks(original: &str, chunks: &[Chunk], path: &str) -> Result<String, String> {
    let mut text = original.to_string();
    let mut cursor = 0;
    for chunk in chunks {
        let search_from = if let Some(context) = &chunk.context {
            let relative = text[cursor..]
                .find(context)
                .ok_or_else(|| format!("context not found in {path}: {context}"))?;
            cursor + relative
        } else {
            cursor
        };
        let old = lines_text(&chunk.old);
        let new = lines_text(&chunk.new);
        let position = if old.is_empty() {
            text[search_from..]
                .find('\n')
                .map(|offset| search_from + offset + 1)
                .unwrap_or(text.len())
        } else {
            let exact = text[search_from..]
                .find(&old)
                .map(|offset| search_from + offset);
            let without_final_newline = old.strip_suffix('\n').and_then(|old| {
                text[search_from..]
                    .find(old)
                    .map(|offset| search_from + offset)
            });
            exact
                .or(without_final_newline)
                .ok_or_else(|| format!("expected lines not found in {path}"))?
        };
        let matched = if old.is_empty() || text[position..].starts_with(&old) {
            old.len()
        } else {
            old.trim_end_matches('\n').len()
        };
        if chunk.eof && !text[position + matched..].trim_matches('\n').is_empty() {
            return Err(format!("end-of-file hunk did not match the end of {path}"));
        }
        text.replace_range(position..position + matched, &new);
        cursor = position + new.len();
    }
    Ok(text)
}

fn lines_text(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

async fn commit(ctx: &ToolCtx, prepared: Vec<Prepared>) -> Result<ToolOutcome, String> {
    let mut receipt = String::new();
    let mut changes = Vec::new();
    let mut locations = Vec::new();
    for change in &prepared {
        if let Some(content) = &change.new {
            if let Some(parent) = change.destination.parent() {
                ctx.executor.create_dir_all(parent).await?;
            }
            ctx.executor
                .write(&change.destination, content.as_bytes())
                .await?;
            ctx.note_read(&change.destination).await;
        }
        if change.remove_source {
            ctx.executor.remove_file(&change.source).await?;
        }
        let old = change.old.as_deref().unwrap_or("");
        let new = change.new.as_deref().unwrap_or("");
        let diff = similar::TextDiff::from_lines(old, new)
            .unified_diff()
            .header(
                &format!("a/{}", change.display),
                &format!("b/{}", change.display),
            )
            .to_string();
        receipt.push_str(&format!("diff {}\n{}\n", change.display, diff));
        changes.push(json!({"path": change.display, "kind": change.kind, "diff": diff}));
        locations.push(change.display.clone());
    }
    let mut outcome = ToolOutcome::ok(format!(
        "Applied {} file change{}.\n\n{}",
        changes.len(),
        if changes.len() == 1 { "" } else { "s" },
        receipt
    ))
    .with_details(json!({"changes": changes}));
    for path in locations {
        outcome = outcome.with_location(path, None);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::{fs::ReadFile, ReadTracker};
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn ctx(root: &Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(root).unwrap()),
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

    #[tokio::test]
    async fn applies_add_update_delete_with_structured_receipt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("edit.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.path().join("delete.txt"), "gone\n").unwrap();
        let ctx = ctx(dir.path());
        for path in ["edit.txt", "delete.txt"] {
            assert!(!ReadFile.invoke(json!({"path": path}), &ctx).await.is_error);
        }
        let patch = "*** Begin Patch\n*** Update File: edit.txt\n@@\n one\n-two\n+changed\n*** Add File: nested/new.txt\n+new\n*** Delete File: delete.txt\n*** End Patch";
        let outcome = ApplyPatch.invoke(json!({"patch": patch}), &ctx).await;
        assert!(!outcome.is_error, "{}", outcome.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("edit.txt")).unwrap(),
            "one\nchanged\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/new.txt")).unwrap(),
            "new\n"
        );
        assert!(!dir.path().join("delete.txt").exists());
        assert_eq!(outcome.locations.len(), 3);
        assert_eq!(outcome.details["changes"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn large_patch_returns_the_complete_change_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!("{}END_OF_PATCH_RECEIPT", "x".repeat(30_000));
        let patch = format!("*** Begin Patch\n*** Add File: large.txt\n+{content}\n*** End Patch");
        let outcome = ApplyPatch
            .invoke(json!({"patch": patch}), &ctx(dir.path()))
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        assert!(outcome.content.contains("END_OF_PATCH_RECEIPT"));
        assert!(!outcome.content.contains("receipt truncated"));
    }

    #[tokio::test]
    async fn preflight_rejects_unread_existing_file_without_partial_add() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("edit.txt"), "one\n").unwrap();
        let patch = "*** Begin Patch\n*** Add File: added.txt\n+new\n*** Update File: edit.txt\n@@\n-one\n+changed\n*** End Patch";
        let outcome = ApplyPatch
            .invoke(json!({"patch": patch}), &ctx(dir.path()))
            .await;
        assert!(outcome.is_error);
        assert!(!dir.path().join("added.txt").exists());
    }
}
