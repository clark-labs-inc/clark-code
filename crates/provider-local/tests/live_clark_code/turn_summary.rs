use std::collections::BTreeMap;

use agent_core::domain::{RunStatus, RunUsage, ToolStatus};
use serde_json::Value;

#[derive(Default, Debug)]
pub(super) struct TurnSummary {
    pub(super) finished: bool,
    pub(super) status: Option<RunStatus>,
    pub(super) run_error: Option<String>,
    pub(super) usage: Option<RunUsage>,
    pub(super) text: String,
    pub(super) tools: Vec<String>,
    pub(super) tool_inputs: Vec<(String, Value)>,
    pub(super) errors: Vec<String>,
    pub(super) tool_statuses: BTreeMap<String, Vec<ToolStatus>>,
    pub(super) permission_requests: usize,
    pub(super) event_counts: BTreeMap<&'static str, usize>,
    pub(super) canonical_text: Option<String>,
}

impl TurnSummary {
    pub(super) fn require_done(&self, label: &str) {
        assert!(self.finished, "{label}: run did not finish: {self:?}");
        assert_eq!(
            self.status,
            Some(RunStatus::Done),
            "{label}: run did not finish cleanly: {self:?}"
        );
        let canonical = self
            .canonical_text
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: canonical message_end was missing"));
        assert_eq!(
            &self.text, canonical,
            "{label}: streamed text differed from canonical message_end"
        );
        assert_eq!(
            self.tool_statuses.len(),
            self.tools.len(),
            "{label}: every tool call must have one terminal status: {self:?}"
        );
        assert!(
            self.tool_statuses
                .values()
                .all(|statuses| statuses.last() == Some(&ToolStatus::Completed)),
            "{label}: every tool call must complete: {self:?}"
        );
    }

    pub(super) fn require_tool(&self, label: &str, tool: &str) {
        assert!(
            self.tools.iter().any(|seen| seen == tool),
            "{label}: expected tool {tool}, got {:?}",
            self.tools
        );
    }

    pub(super) fn require_tool_input_contains(&self, label: &str, tool: &str, needle: &str) {
        assert!(
            self.tool_inputs.iter().any(|(seen, input)| {
                seen == tool
                    && serde_json::to_string(input)
                        .is_ok_and(|serialized| serialized.contains(needle))
            }),
            "{label}: expected {tool} input containing {needle:?}, got {:?}",
            self.tool_inputs
        );
    }

    pub(super) fn last_tool_input_str<'a>(
        &'a self,
        label: &str,
        tool: &str,
        field: &str,
    ) -> &'a str {
        self.tool_inputs
            .iter()
            .rev()
            .find_map(|(seen, input)| {
                (seen == tool)
                    .then(|| input.get(field))
                    .flatten()
                    .and_then(Value::as_str)
            })
            .unwrap_or_else(|| {
                panic!(
                    "{label}: expected string field {field:?} in {tool} input, got {:?}",
                    self.tool_inputs
                )
            })
    }

    pub(super) fn require_cloud_research_first(&self, label: &str) {
        let research = self
            .tools
            .iter()
            .position(|tool| tool == "clark_research")
            .unwrap_or_else(|| panic!("{label}: expected clark_research, got {:?}", self.tools));
        let discovery = self
            .tools
            .iter()
            .position(|tool| tool == "tool_search")
            .unwrap_or_else(|| panic!("{label}: expected tool_search, got {:?}", self.tools));
        assert!(
            discovery < research,
            "{label}: research must be activated before use: {:?}",
            self.tools
        );
        assert!(
            !self.tools[..research]
                .iter()
                .any(|tool| tool == "web_fetch" || tool == "bash"),
            "{label}: local retrieval ran before Clark Cloud Agent: {:?}",
            self.tools
        );
    }
}
