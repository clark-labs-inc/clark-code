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

/// Read config for the user's Clark-hosted personal memory (recall only —
/// Clark extracts it server-side; there is no client write path).
#[derive(Clone)]
pub struct PersonalRecall {
    pub base_url: String,
    pub api_key: String,
}

/// What the registry needs to expose the `memory` tool + inject memory at
/// session start. `Some` ⇒ memories are enabled.
#[derive(Clone, Default)]
pub struct MemoryConfig {
    /// `~/.clark/memory` for the local, agent-writable global scope.
    pub global_dir: Option<PathBuf>,
    /// Clark Platform recall of the user's extracted memory, when signed in.
    pub personal: Option<PersonalRecall>,
}

/// Recall/save durable memory across the project, global (local), and personal
/// (Clark-extracted) scopes.
pub struct MemoryTool {
    /// `~/.clark/memory` on the local machine, or `None` if home is unresolved.
    global_dir: Option<PathBuf>,
    /// Clark personal-memory recall, when signed in.
    personal: Option<PersonalRecall>,
}

impl MemoryTool {
    pub fn new(global_dir: Option<PathBuf>, personal: Option<PersonalRecall>) -> Self {
        Self {
            global_dir,
            personal,
        }
    }
}

#[async_trait]
impl ToolExecutor for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Recall, save, or retire durable memories. action \"recall\" returns your saved facts \
(project + global) plus personal memory Clark has learned about the user across their work; \
action \"remember\" saves one; action \"forget\" removes a note whose fact was superseded or \
turned out wrong (when the user reverses a decision, save the new fact AND forget the old \
note in the same turn). scope \"project\" = facts about this codebase; scope \"global\" = \
facts about the user across all their projects. Save durable, reusable facts only."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["recall", "remember", "forget"],
                    "description": "\"recall\" to read saved memories, \"remember\" to save one, \"forget\" to remove a superseded or wrong note."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "For remember/forget: \"project\" (this codebase) or \"global\" (across all your projects). Defaults to project."
                },
                "source": {
                    "type": "string",
                    "enum": ["user-stated", "inferred"],
                    "description": "For remember: did the user actually say this (\"user-stated\"), or did you conclude it yourself (\"inferred\")? Required for remember."
                },
                "title": {
                    "type": "string",
                    "description": "Short title for the fact (becomes its filename). Required for remember and forget."
                },
                "content": {
                    "type": "string",
                    "description": "The fact to remember, in markdown. Quote the user's own words for decisions and preferences. Required for remember."
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
                if let Some(s) = memory::recall_scope(
                    ctx.executor.as_ref(),
                    &proj_dir,
                    "Project",
                    Some(ctx.sandbox.root()),
                )
                .await
                {
                    out.push_str(&s);
                    out.push_str("\n\n");
                }
                if let Some(gdir) = &self.global_dir {
                    if let Some(s) =
                        memory::recall_scope(&LocalExecutor, gdir, "Global", None).await
                    {
                        out.push_str(&s);
                        out.push_str("\n\n");
                    }
                }
                // Personal memory Clark extracted from the user's conversations.
                if let Some(p) = &self.personal {
                    if let Ok(mems) =
                        crate::platform::recall_personal_memories(&p.base_url, &p.api_key).await
                    {
                        if let Some(s) = crate::platform::personal_memory_section(&mems) {
                            out.push_str(&s);
                        }
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
                let source = arg_str_opt(&args, "source");
                let (result, label) = if scope == "global" {
                    let Some(gdir) = &self.global_dir else {
                        return ToolOutcome::error(
                            "global memory is unavailable (no home directory)",
                        );
                    };
                    (
                        memory::save_memory(
                            &LocalExecutor,
                            gdir,
                            &title,
                            &content,
                            kind,
                            source.as_deref(),
                        )
                        .await,
                        "global",
                    )
                } else {
                    let dir = memory::memory_dir(ctx.sandbox.root());
                    (
                        memory::save_memory(
                            ctx.executor.as_ref(),
                            &dir,
                            &title,
                            &content,
                            kind,
                            source.as_deref(),
                        )
                        .await,
                        "project",
                    )
                };
                match result {
                    Ok(file) => ToolOutcome::ok(format!("Saved to {label} memory: {file}")),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            "forget" => {
                let title = match arg_str(&args, "title") {
                    Ok(t) => t,
                    Err(e) => return ToolOutcome::error(e),
                };
                let scope = arg_str_opt(&args, "scope").unwrap_or_else(|| "project".into());
                let result = if scope == "global" {
                    let Some(gdir) = &self.global_dir else {
                        return ToolOutcome::error(
                            "global memory is unavailable (no home directory)",
                        );
                    };
                    memory::delete_memory(&LocalExecutor, gdir, &title).await
                } else {
                    let dir = memory::memory_dir(ctx.sandbox.root());
                    memory::delete_memory(ctx.executor.as_ref(), &dir, &title).await
                };
                match result {
                    Ok(Some(file)) => {
                        ToolOutcome::ok(format!("Forgot {scope} note: {file} (removed)"))
                    }
                    Ok(None) => ToolOutcome::error(format!(
                        "no {scope} note matches {title:?} — recall first to see exact titles"
                    )),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            other => ToolOutcome::error(format!(
                "unknown action `{other}` — use \"recall\", \"remember\", or \"forget\""
            )),
        }
    }
}
