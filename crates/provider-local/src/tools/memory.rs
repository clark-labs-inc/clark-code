//! The `memory` tool — how the agent recalls and saves durable facts.
//!
//! Two scopes: **project** (this codebase, `<root>/.clark/memory`, reached
//! through the session executor so it works local or remote) and **global** (the
//! user, `~/.clark/memory` on the desktop machine, always local). This is the
//! only way the agent can reach the global scope — its ordinary file tools are
//! sandboxed to the project root.

use std::path::PathBuf;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};
use crate::exec::LocalExecutor;
use crate::memory::{self, MemoryType};

/// Recall/save durable memory across the project and global scopes.
pub struct MemoryTool {
    /// `~/.clark/memory` on the local machine, or `None` if home is unresolved.
    global_dir: Option<PathBuf>,
}

impl MemoryTool {
    pub fn new(global_dir: Option<PathBuf>) -> Self {
        Self { global_dir }
    }
}

#[async_trait]
impl ToolExecutor for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Recall or save durable memories. action \"recall\" returns saved facts from both \
scopes; action \"remember\" saves one. scope \"project\" = facts about this codebase; scope \
\"global\" = facts about the user across all their projects. Save durable, reusable facts only."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["recall", "remember"],
                    "description": "\"recall\" to read saved memories, \"remember\" to save one."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "For remember: \"project\" (this codebase) or \"global\" (across all your projects). Defaults to project."
                },
                "title": {
                    "type": "string",
                    "description": "Short title for the fact (becomes its filename). Required for remember."
                },
                "content": {
                    "type": "string",
                    "description": "The fact to remember, in markdown. Required for remember."
                },
                "type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Optional category for the fact."
                }
            },
            "required": ["action"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let action = match arg_str(&args, "action") {
            Ok(a) => a,
            Err(e) => return ToolOutcome::error(e),
        };
        match action.as_str() {
            "recall" => {
                let mut out = String::new();
                let proj_dir = memory::memory_dir(ctx.sandbox.root());
                if let Some(s) =
                    memory::recall_scope(ctx.executor.as_ref(), &proj_dir, "Project").await
                {
                    out.push_str(&s);
                    out.push_str("\n\n");
                }
                if let Some(gdir) = &self.global_dir {
                    if let Some(s) = memory::recall_scope(&LocalExecutor, gdir, "Global").await {
                        out.push_str(&s);
                    }
                }
                if out.trim().is_empty() {
                    ToolOutcome::ok("No memories saved yet.")
                } else {
                    ToolOutcome::ok(out.trim().to_string())
                }
            }
            "remember" => {
                let title = match arg_str(&args, "title") {
                    Ok(t) => t,
                    Err(e) => return ToolOutcome::error(e),
                };
                let content = match arg_str(&args, "content") {
                    Ok(c) => c,
                    Err(e) => return ToolOutcome::error(e),
                };
                let kind = arg_str_opt(&args, "type").and_then(|s| MemoryType::parse(&s));
                let scope = arg_str_opt(&args, "scope").unwrap_or_else(|| "project".into());
                let (result, label) = if scope == "global" {
                    let Some(gdir) = &self.global_dir else {
                        return ToolOutcome::error(
                            "global memory is unavailable (no home directory)",
                        );
                    };
                    (
                        memory::save_memory(&LocalExecutor, gdir, &title, &content, kind).await,
                        "global",
                    )
                } else {
                    let dir = memory::memory_dir(ctx.sandbox.root());
                    (
                        memory::save_memory(ctx.executor.as_ref(), &dir, &title, &content, kind)
                            .await,
                        "project",
                    )
                };
                match result {
                    Ok(file) => ToolOutcome::ok(format!("Saved to {label} memory: {file}")),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            other => ToolOutcome::error(format!(
                "unknown action `{other}` — use \"recall\" or \"remember\""
            )),
        }
    }
}
