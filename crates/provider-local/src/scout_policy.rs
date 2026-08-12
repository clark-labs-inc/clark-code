use std::collections::HashSet;

use agent_loop::plugin::{ToolGate, ToolGateContext};
use agent_loop::{Plugin, PluginCapabilities};

pub(crate) const EVIDENCE_TOOLS: &[&str] = &[
    "scout_capabilities",
    "scout_repository_census",
    "scout_adapter",
    "scout_enterprise",
    "scout_enterprise_query",
    "scout_capsule",
];

/// Host-enforced capability boundary for an explicit Scout turn.
///
/// Scout's skill instructions describe the workflow, but they are not an
/// authorization boundary. This gate keeps generic coding, shell, discovery,
/// and memory tools out of every model call in the turn, including Full access
/// sessions where the ordinary shell executor runs directly on the host.
pub(crate) struct ScoutToolGate;

impl ScoutToolGate {
    fn allowed(tool_name: &str) -> bool {
        EVIDENCE_TOOLS.contains(&tool_name)
            || matches!(
                tool_name,
                "update_plan" | crate::tools::final_answer::FINAL_ANSWER_TOOL
            )
    }
}

impl Plugin for ScoutToolGate {
    fn name(&self) -> &'static str {
        "scout_tool_boundary"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::tool_gate()
    }
}

#[async_trait::async_trait]
impl ToolGate for ScoutToolGate {
    async fn next_turn_tool_allowlist(&self, ctx: ToolGateContext<'_>) -> Option<HashSet<String>> {
        Some(
            ctx.available_tool_names
                .iter()
                .copied()
                .filter(|name| Self::allowed(name))
                .map(str::to_string)
                .collect(),
        )
    }

    async fn denial_reason(&self, tool_name: &str, _ctx: ToolGateContext<'_>) -> Option<String> {
        (!Self::allowed(tool_name)).then(|| {
            format!(
                "Tool `{tool_name}` is outside Scout's declared evidence boundary. Use the typed Scout tools; if no authorized source or charter is available, stop and ask the user to declare one."
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scout_boundary_allows_typed_cartography_but_not_host_or_memory_tools() {
        let gate = ScoutToolGate;
        let available = [
            "read_file",
            "list_dir",
            "grep",
            "bash",
            "tool_search",
            "memory",
            "memory_recall",
            "scout_capabilities",
            "scout_repository_census",
            "scout_adapter",
            "scout_enterprise",
            "scout_enterprise_query",
            "scout_capsule",
            "update_plan",
            crate::tools::final_answer::FINAL_ANSWER_TOOL,
        ];
        let context = ToolGateContext {
            iteration: 0,
            messages: &[],
            conversation_id: Some("scout-session"),
            available_tool_names: &available,
        };

        let allowed = gate.next_turn_tool_allowlist(context).await.unwrap();

        for name in [
            "scout_capabilities",
            "scout_repository_census",
            "scout_adapter",
            "scout_enterprise",
            "scout_enterprise_query",
            "scout_capsule",
            "update_plan",
            crate::tools::final_answer::FINAL_ANSWER_TOOL,
        ] {
            assert!(allowed.contains(name), "Scout should retain {name}");
        }
        for name in [
            "read_file",
            "list_dir",
            "grep",
            "bash",
            "tool_search",
            "memory",
            "memory_recall",
        ] {
            assert!(!allowed.contains(name), "Scout must not expose {name}");
            assert!(gate
                .denial_reason(
                    name,
                    ToolGateContext {
                        iteration: 0,
                        messages: &[],
                        conversation_id: Some("scout-session"),
                        available_tool_names: &available,
                    },
                )
                .await
                .is_some());
        }
    }
}
