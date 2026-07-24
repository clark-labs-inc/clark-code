use agent_core::domain::AgentEvent;

pub(super) fn event_name(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::RunStarted { .. } => "RunStarted",
        AgentEvent::Checkpoint { .. } => "Checkpoint",
        AgentEvent::MessageChunk { .. } => "MessageChunk",
        AgentEvent::MessagePhase { .. } => "MessagePhase",
        AgentEvent::ToolCall { .. } => "ToolCall",
        AgentEvent::ToolCallUpdate { .. } => "ToolCallUpdate",
        AgentEvent::ExecutionChecklistUpdated { .. } => "ExecutionChecklistUpdated",
        AgentEvent::ProposedPlanUpdated { .. } => "ProposedPlanUpdated",
        AgentEvent::GoalUpdated { .. } => "GoalUpdated",
        AgentEvent::RunUsageUpdated { .. } => "RunUsageUpdated",
        AgentEvent::PermissionRequest { .. } => "PermissionRequest",
        AgentEvent::Artifact { .. } => "Artifact",
        AgentEvent::Surface { .. } => "Surface",
        AgentEvent::FanOut { .. } => "FanOut",
        AgentEvent::ProviderIncidentUpdated { .. } => "ProviderIncidentUpdated",
        AgentEvent::ModeChanged { .. } => "ModeChanged",
        AgentEvent::ContextCompacted { .. } => "ContextCompacted",
        AgentEvent::Trace { .. } => "Trace",
        AgentEvent::RunFinished { .. } => "RunFinished",
        AgentEvent::Error { .. } => "Error",
    }
}

pub(super) fn tool_name_from_title(title: &str) -> String {
    title.split(':').next().unwrap_or(title).trim().to_string()
}

pub(super) fn trim_terminal_line_endings(value: &str) -> &str {
    value.trim_end_matches(&['\r', '\n'][..])
}

#[cfg(test)]
mod tests {
    use super::trim_terminal_line_endings;

    #[test]
    fn text_receipts_accept_terminal_line_endings_only() {
        assert_eq!(trim_terminal_line_endings("beta"), "beta");
        assert_eq!(trim_terminal_line_endings("beta\n"), "beta");
        assert_eq!(trim_terminal_line_endings("beta\r\n"), "beta");
        assert_eq!(trim_terminal_line_endings("beta \n"), "beta ");
    }
}
