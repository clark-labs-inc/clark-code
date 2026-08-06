//! `clark_research` — delegate research to Clark's agentic Platform API.
//!
//! This is the bridge that lets coding stay local while leaning on Clark's
//! power: web search, planning, parallel research agents, browsing, and
//! latest-doc lookups. A call starts a background Platform response, polls its
//! public progress projection, then fetches the final findings. Clark runs the
//! tools server-side. Uses the same `ck_live_` key as the coding model; nothing
//! here touches the local filesystem.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass};

use super::clark_progress::ClarkResearchClient;
use crate::config::AgenticClarkConfig;

pub struct ClarkResearchTool {
    client: Option<ClarkResearchClient>,
}

impl ClarkResearchTool {
    pub fn new(config: AgenticClarkConfig) -> Self {
        let client = ClarkResearchClient::new(config).ok();
        Self { client }
    }
}

#[async_trait]
impl ToolExecutor for ClarkResearchTool {
    fn name(&self) -> &str {
        "clark_research"
    }
    fn description(&self) -> &str {
        "Delegate a research task to Clark's agent: web search, browsing, reading the latest API/library documentation, and multi-step investigation. Use this whenever you need up-to-date external information you can't get from the local codebase. Returns Clark's findings as text. Runs remotely; does NOT touch local files."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "The research question or task, stated in full (Clark plans and executes it autonomously)."},
                "context": {"type": "string", "description": "Optional extra context from the local task to focus the research."}
            },
            "required": ["query"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Research
    }
    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::BrokeredClarkCloud
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(client) = &self.client else {
            return ToolOutcome::error("clark research is not configured");
        };
        let query = match arg_str(&args, "query") {
            Ok(q) => q,
            Err(e) => return ToolOutcome::error(e),
        };
        let text = match arg_str_opt(&args, "context") {
            Some(c) if !c.is_empty() => format!("{query}\n\nContext:\n{c}"),
            _ => query,
        };
        match client
            .research(&text, &ctx.cancel, |progress| {
                ctx.report_call_progress(progress)
            })
            .await
        {
            Ok(answer) if !answer.is_empty() => ToolOutcome::ok(answer),
            Ok(_) => ToolOutcome::error("clark research returned no findings"),
            Err(e) => ToolOutcome::error(format!("clark research: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_advertises_research_schema() {
        let t = ClarkResearchTool::new(AgenticClarkConfig {
            base_url: "https://api.clarkslabs.com/v1".into(),
            api_key: Some("ck_live_x".into()),
            model: "clark".into(),
        });
        assert_eq!(t.name(), "clark_research");
        assert!(!t.mutating());
        assert_eq!(
            t.permission_class(),
            ToolPermissionClass::BrokeredClarkCloud
        );
        let params = t.parameters();
        assert_eq!(params["required"][0], "query");
    }
}
