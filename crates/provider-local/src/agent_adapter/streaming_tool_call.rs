//! Accumulates enough provider delta metadata to announce a tool before its
//! arguments finish streaming. Execution later reconciles the same call id
//! with the parsed arguments and definitive title.

use std::collections::BTreeMap;

use agent_core::domain as desktop;
use agent_core::ids::{RunId, ToolCallId};
use async_channel::Sender;
use serde_json::Value;

use crate::tools::final_answer::FINAL_ANSWER_TOOL;
use crate::tools::ToolRegistry;

use super::{redaction, tool_result_to_content, tool_title};

#[derive(Default)]
struct Candidate {
    id: String,
    name: String,
    arguments: String,
    arguments_started: bool,
    announced: bool,
}

pub(super) fn streamed_tool_event(
    run: &RunId,
    registry: &ToolRegistry,
    tool_call_id: String,
    tool_name: String,
    parsed_args: Option<Value>,
) -> desktop::AgentEvent {
    let kind = registry
        .get(&tool_name)
        .as_ref()
        .map(|tool| tool.kind())
        .unwrap_or_default();
    let raw_input = parsed_args
        .as_ref()
        .map(|args| redaction::persisted_tool_args(&tool_name, args));
    let title_args = raw_input.as_ref().unwrap_or(&Value::Null);
    desktop::AgentEvent::ToolCall {
        run: run.clone(),
        call: desktop::ToolCall {
            id: ToolCallId::new(tool_call_id),
            tool_name: Some(tool_name.clone()),
            title: tool_title(&tool_name, title_args),
            kind,
            status: desktop::ToolStatus::Pending,
            locations: Vec::new(),
            content: Vec::new(),
            raw_input,
            streamed_input: String::new(),
            progress: None,
        },
    }
}

pub(super) fn execution_tool_event(
    run: &RunId,
    registry: &ToolRegistry,
    announced: bool,
    tool_call_id: String,
    tool_name: String,
    args: Value,
) -> desktop::AgentEvent {
    let kind = registry
        .get(&tool_name)
        .as_ref()
        .map(|tool| tool.kind())
        .unwrap_or_default();
    let id = ToolCallId::new(tool_call_id);
    if announced {
        desktop::AgentEvent::ToolCallUpdate {
            run: run.clone(),
            id,
            patch: desktop::ToolCallPatch {
                title: Some(tool_title(&tool_name, &args)),
                kind: Some(kind),
                raw_input: Some(args),
                status: Some(desktop::ToolStatus::Pending),
                ..Default::default()
            },
        }
    } else {
        desktop::AgentEvent::ToolCall {
            run: run.clone(),
            call: desktop::ToolCall {
                id,
                tool_name: Some(tool_name.clone()),
                title: tool_title(&tool_name, &args),
                kind,
                status: desktop::ToolStatus::Pending,
                locations: Vec::new(),
                content: Vec::new(),
                raw_input: Some(args),
                streamed_input: String::new(),
                progress: None,
            },
        }
    }
}

pub(super) fn execution_update_event(
    run: &RunId,
    tool_call_id: String,
    partial: agent_loop::ToolResult,
) -> desktop::AgentEvent {
    desktop::AgentEvent::ToolCallUpdate {
        run: run.clone(),
        id: ToolCallId::new(tool_call_id),
        patch: desktop::ToolCallPatch {
            status: Some(desktop::ToolStatus::InProgress),
            append_content: tool_result_to_content(&partial),
            ..Default::default()
        },
    }
}

pub(super) struct ExecutionStart {
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) args: Value,
}

pub(super) async fn emit_execution_start(
    events: &Sender<desktop::AgentEvent>,
    run: &RunId,
    registry: &ToolRegistry,
    execution: Option<&crate::root_execution::RootExecutionTrace>,
    streaming_calls: &std::sync::Mutex<StreamingToolCalls>,
    start: ExecutionStart,
) {
    let ExecutionStart {
        tool_call_id,
        tool_name,
        args,
    } = start;
    let executor = registry.get(&tool_name);
    if let Some(execution) = execution {
        execution.tool_started(
            &tool_call_id,
            &tool_name,
            executor.as_ref().is_some_and(|tool| tool.mutating()),
        );
    }
    let announced = streaming_calls
        .lock()
        .expect("streaming tool calls lock")
        .was_announced(&tool_call_id);
    let event = execution_tool_event(run, registry, announced, tool_call_id, tool_name, args);
    let _ = events.send(event).await;
}

#[derive(Default)]
pub(super) struct StreamingToolCalls {
    candidates: BTreeMap<usize, Candidate>,
}

impl StreamingToolCalls {
    pub(super) fn reset_message(&mut self) {
        self.candidates.clear();
    }

    pub(super) fn observe_delta(
        &mut self,
        index: usize,
        id_delta: Option<&str>,
        name_delta: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Option<(String, String, Option<serde_json::Value>)> {
        let candidate = self.candidates.entry(index).or_default();
        if let Some(id) = id_delta {
            candidate.id.push_str(id);
        }
        if let Some(name) = name_delta {
            candidate.name.push_str(name);
        }
        if let Some(arguments) = arguments_delta {
            candidate.arguments.push_str(arguments);
        }
        candidate.arguments_started |= arguments_delta.is_some();

        if candidate.announced
            || !candidate.arguments_started
            || candidate.id.is_empty()
            || candidate.name.is_empty()
            || candidate.name == FINAL_ANSWER_TOOL
        {
            return None;
        }
        candidate.announced = true;
        let parsed_arguments = serde_json::from_str(&candidate.arguments).ok();
        Some((
            candidate.id.clone(),
            candidate.name.clone(),
            parsed_arguments,
        ))
    }

    /// The announced call id at a provider tool-call index, once the row
    /// exists. Lets a later delta for the same index patch that row.
    pub(super) fn announced_id(&self, index: usize) -> Option<String> {
        self.candidates
            .get(&index)
            .filter(|candidate| candidate.announced && !candidate.id.is_empty())
            .map(|candidate| candidate.id.clone())
    }

    pub(super) fn was_announced(&self, tool_call_id: &str) -> bool {
        self.candidates
            .values()
            .any(|candidate| candidate.announced && candidate.id == tool_call_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announces_an_ordinary_tool_once_arguments_begin() {
        let mut calls = StreamingToolCalls::default();
        assert_eq!(
            calls.observe_delta(0, Some("call-1"), Some("read_"), None),
            None
        );
        assert_eq!(
            calls.observe_delta(0, None, Some("file"), Some("")),
            Some(("call-1".into(), "read_file".into(), None))
        );
        assert_eq!(calls.observe_delta(0, None, None, Some("{}")), None);
    }

    #[test]
    fn includes_arguments_when_the_first_payload_is_complete() {
        let mut calls = StreamingToolCalls::default();
        assert_eq!(
            calls.observe_delta(
                0,
                Some("call-1"),
                Some("read_file"),
                Some("{\"path\":\"README.md\"}"),
            ),
            Some((
                "call-1".into(),
                "read_file".into(),
                Some(serde_json::json!({"path": "README.md"})),
            ))
        );
    }

    #[test]
    fn never_announces_final_answer_as_a_tool_row() {
        let mut calls = StreamingToolCalls::default();
        assert_eq!(
            calls.observe_delta(
                0,
                Some("answer-1"),
                Some(FINAL_ANSWER_TOOL),
                Some("{\"content\":"),
            ),
            None
        );
    }
}
