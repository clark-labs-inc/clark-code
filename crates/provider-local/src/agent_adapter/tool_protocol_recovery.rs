use serde_json::{json, Value};

use crate::llm::ToolSchema;

pub(crate) const TOOL_PROTOCOL_EXHAUSTED_PREFIX: &str = "tool_protocol_exhausted:";

pub(super) struct RecoveryRequest {
    pub(super) tools: Vec<ToolSchema>,
    pub(super) forced_tool_name: Option<&'static str>,
    pub(super) tool_choice: Value,
}

impl RecoveryRequest {
    pub(super) fn advertised_tool_names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect()
    }
}

pub(super) fn request(tools: &[ToolSchema], repair_attempts: u8) -> RecoveryRequest {
    let forced_tool_name =
        (repair_attempts >= 2).then_some(crate::tools::final_answer::FINAL_ANSWER_TOOL);
    // The last repair is a true singleton contract, not merely a named
    // choice over the broad catalog. Some OpenAI-compatible providers accept
    // the named choice but still reason over (or reject) unrelated tools.
    let tools = if let Some(name) = forced_tool_name {
        tools
            .iter()
            .filter(|tool| tool.function.name == name)
            .cloned()
            .collect()
    } else {
        tools.to_vec()
    };
    let tool_choice = forced_tool_name.map_or_else(
        || json!("auto"),
        |name| {
            json!({
                "type": "function",
                "function": { "name": name },
            })
        },
    );
    RecoveryRequest {
        tools,
        forced_tool_name,
        tool_choice,
    }
}
