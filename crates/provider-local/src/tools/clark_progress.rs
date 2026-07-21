use std::time::Duration;

use agent_core::domain::{
    ToolCallProgress, ToolProgressAgent, ToolProgressPhase, ToolProgressStep, ToolStatus,
};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::config::AgenticClarkConfig;

#[path = "clark_response_result.rs"]
mod response_result;

use response_result::terminal_response;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const RUN_DEADLINE: Duration = Duration::from_secs(11 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const RESEARCH_INSTRUCTION: &str = "Investigate this research task thoroughly using web search, browsing, and reasoning. Return a concise, well-organized findings report and cite sources where relevant.";

#[derive(Clone)]
pub(super) struct ClarkResearchClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    poll_interval: Duration,
    status_poll_interval: Duration,
    deadline: Duration,
}

#[derive(Debug)]
enum RequestError {
    Fatal(String),
    Transient(String),
}

impl RequestError {
    fn message(self) -> String {
        match self {
            Self::Fatal(message) | Self::Transient(message) => message,
        }
    }
}

impl ClarkResearchClient {
    pub(super) fn new(config: AgenticClarkConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| format!("Clark research client build failed: {error}"))?;
        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            model: config.model,
            api_key: config.api_key,
            poll_interval: POLL_INTERVAL,
            status_poll_interval: STATUS_POLL_INTERVAL,
            deadline: RUN_DEADLINE,
        })
    }

    pub(super) async fn research(
        &self,
        task: &str,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(ToolCallProgress),
    ) -> Result<String, String> {
        let mut progress = starting_progress();
        on_progress(progress.clone());

        let request = json!({
            "model": self.model,
            "input": format!("{RESEARCH_INSTRUCTION}\n\nTask:\n{task}"),
            "background": true,
        });
        let initial = self
            .request_json(Method::POST, "/responses", Some(&request), cancel)
            .await
            .map_err(RequestError::message)?;
        let response_id = initial
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Clark research response did not include an id".to_string())?;
        if let Some(result) = terminal_response(&initial)? {
            return Ok(result);
        }

        let started = Instant::now();
        let mut after_seq = 0_u64;
        let mut last_status_poll = Instant::now();

        loop {
            if started.elapsed() >= self.deadline {
                return Err("Clark research timed out waiting for a final response".to_string());
            }

            let events_path =
                format!("/responses/{response_id}/events?after_seq={after_seq}&limit=200");
            let mut terminal_event = false;
            let events_failed = match self
                .request_json(Method::GET, &events_path, None, cancel)
                .await
            {
                Ok(payload) => {
                    if let Some(events) = payload.get("data").and_then(Value::as_array) {
                        for event in events {
                            let sequence = event
                                .get("sequence")
                                .and_then(Value::as_u64)
                                .unwrap_or(after_seq);
                            if sequence <= after_seq {
                                continue;
                            }
                            after_seq = after_seq.max(sequence);
                            terminal_event |= is_terminal_event(event);
                            if apply_public_event(&mut progress, event) {
                                on_progress(progress.clone());
                            }
                        }
                    }
                    if let Some(next) = payload.get("next_after_seq").and_then(Value::as_u64) {
                        after_seq = after_seq.max(next);
                    }
                    false
                }
                Err(RequestError::Fatal(message)) => return Err(message),
                Err(RequestError::Transient(_)) => true,
            };

            let status_due = terminal_event
                || events_failed
                || last_status_poll.elapsed() >= self.status_poll_interval;
            if status_due {
                let response_path = format!("/responses/{response_id}");
                match self
                    .request_json(Method::GET, &response_path, None, cancel)
                    .await
                {
                    Ok(response) => {
                        last_status_poll = Instant::now();
                        if let Some(result) = terminal_response(&response)? {
                            return Ok(result);
                        }
                    }
                    Err(RequestError::Fatal(message)) => return Err(message),
                    Err(RequestError::Transient(_)) => {}
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => return Err("Clark research cancelled".to_string()),
                _ = tokio::time::sleep(self.poll_interval) => {}
            }
        }
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        cancel: &CancellationToken,
    ) -> Result<Value, RequestError> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(RequestError::Fatal("Clark research cancelled".to_string())),
            response = request.send() => response.map_err(|error| {
                RequestError::Transient(format!("Clark research request failed: {error}"))
            })?,
        };
        let status = response.status();
        let text = tokio::select! {
            _ = cancel.cancelled() => return Err(RequestError::Fatal("Clark research cancelled".to_string())),
            text = response.text() => text.map_err(|error| {
                RequestError::Transient(format!("Clark research response failed: {error}"))
            })?,
        };
        if !status.is_success() {
            return Err(classify_http_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|error| {
            RequestError::Transient(format!("Clark research returned invalid JSON: {error}"))
        })
    }

    #[cfg(test)]
    fn with_test_timing(mut self, poll: Duration, status: Duration, deadline: Duration) -> Self {
        self.poll_interval = poll;
        self.status_poll_interval = status;
        self.deadline = deadline;
        self
    }
}

fn classify_http_error(status: StatusCode, body: &str) -> RequestError {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    let message = match status.as_u16() {
        401 => format!("platform key rejected: {detail}"),
        402 => "insufficient_credits".to_string(),
        403 => format!("Clark research permission denied: {detail}"),
        _ => format!("Clark research endpoint returned {status}: {detail}"),
    };
    if status == StatusCode::NOT_FOUND
        || status.as_u16() == 408
        || status.as_u16() == 425
        || status.as_u16() == 429
        || status.as_u16() == 524
        || status.is_server_error()
    {
        RequestError::Transient(message)
    } else {
        RequestError::Fatal(message)
    }
}

fn starting_progress() -> ToolCallProgress {
    ToolCallProgress {
        revision: 1,
        status: ToolStatus::InProgress,
        latest_activity: Some("Starting Clark Cloud Agent".to_string()),
        phases: Vec::new(),
        agents: Vec::new(),
    }
}

fn apply_public_event(progress: &mut ToolCallProgress, event: &Value) -> bool {
    let before = progress.clone();
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data = event.get("data").unwrap_or(&Value::Null);
    match event_type {
        "run_queued" => {
            progress.status = ToolStatus::Pending;
            progress.latest_activity = Some("Queued in Clark Cloud".to_string());
        }
        "run_claimed" | "run_started" => {
            progress.status = ToolStatus::InProgress;
            progress.latest_activity = first_text(data, &["summary", "task"])
                .or_else(|| Some("Starting Clark Cloud Agent".to_string()));
        }
        "run_note" => {
            progress.status = ToolStatus::InProgress;
            if let Some(activity) = first_text(data, &["summary", "task"]) {
                progress.latest_activity = Some(activity);
            }
        }
        "execution_started" => {
            progress.status = ToolStatus::InProgress;
            if let Some(activity) = first_text(data, &["title", "goal", "task"]) {
                progress.latest_activity = Some(activity);
            }
        }
        "execution_plan_provisional" | "execution_plan_committed" | "execution_plan_finalized" => {
            apply_plan(progress, data)
        }
        "execution_node_updated" => apply_execution_node(progress, data),
        "subagent_event" => apply_subagent(progress, data),
        "run_completed" => {
            progress.status = ToolStatus::Completed;
            progress.latest_activity =
                first_text(data, &["summary"]).or_else(|| Some("Research complete".to_string()));
        }
        "run_failed" => {
            progress.status = status_field(data, "status", ToolStatus::Failed);
            progress.latest_activity =
                first_text(data, &["error", "summary"]).or_else(|| match progress.status {
                    ToolStatus::Cancelled => Some("Research cancelled".to_string()),
                    _ => Some("Research failed".to_string()),
                });
        }
        "run_cancelled" => {
            progress.status = ToolStatus::Cancelled;
            progress.latest_activity = Some("Research cancelled".to_string());
        }
        _ => {}
    }
    if *progress == before {
        false
    } else {
        progress.revision = before.revision.saturating_add(1);
        true
    }
}

fn apply_plan(progress: &mut ToolCallProgress, data: &Value) {
    let Some(phases) = data.get("phases").and_then(Value::as_array) else {
        return;
    };
    progress.phases = phases
        .iter()
        .enumerate()
        .map(|(phase_index, phase)| {
            let id =
                identifier_field(phase, "id").unwrap_or_else(|| format!("phase-{phase_index}"));
            let status = status_field(phase, "status", ToolStatus::Pending);
            let steps = phase
                .get("planned_steps")
                .and_then(Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .enumerate()
                        .map(|(step_index, step)| ToolProgressStep {
                            id: identifier_field(step, "id")
                                .unwrap_or_else(|| format!("{id}-step-{step_index}")),
                            title: first_text(step, &["title", "summary"])
                                .unwrap_or_else(|| "Planned step".to_string()),
                            status: status_field(step, "status", ToolStatus::Pending),
                            summary: string_field(step, "summary"),
                        })
                        .collect()
                })
                .unwrap_or_default();
            ToolProgressPhase {
                id,
                title: string_field(phase, "title")
                    .unwrap_or_else(|| format!("Phase {}", phase_index + 1)),
                status,
                summary: string_field(phase, "public_narration"),
                steps,
            }
        })
        .collect();
    if let Some(current) = progress
        .phases
        .iter()
        .find(|phase| phase.status == ToolStatus::InProgress)
    {
        progress.latest_activity = current
            .summary
            .clone()
            .or_else(|| Some(current.title.clone()));
    }
}

fn apply_execution_node(progress: &mut ToolCallProgress, data: &Value) {
    let Some(label) = first_text(data, &["label", "summary"]) else {
        return;
    };
    let status = status_field(data, "status", ToolStatus::InProgress);
    let summary = string_field(data, "summary");
    let phase_id = identifier_field(data, "phase_id");
    let planned_step_id = identifier_field(data, "planned_step_id");
    let node_id = identifier_field(data, "node_id").unwrap_or_else(|| label.clone());

    let phase_index = phase_id
        .as_deref()
        .and_then(|id| progress.phases.iter().position(|phase| phase.id == id))
        .or_else(|| {
            progress
                .phases
                .iter()
                .position(|phase| phase.status == ToolStatus::InProgress)
        });
    if let Some(phase_index) = phase_index {
        let phase = &mut progress.phases[phase_index];
        let step_index = planned_step_id
            .as_deref()
            .and_then(|id| phase.steps.iter().position(|step| step.id == id))
            .or_else(|| phase.steps.iter().position(|step| step.id == node_id));
        if let Some(step_index) = step_index {
            let step = &mut phase.steps[step_index];
            step.status = status;
            step.title = label.clone();
            if summary.is_some() {
                step.summary = summary.clone();
            }
        } else {
            phase.steps.push(ToolProgressStep {
                id: planned_step_id.unwrap_or(node_id),
                title: label.clone(),
                status,
                summary: summary.clone(),
            });
        }
    }
    progress.latest_activity = Some(summary.unwrap_or(label));
}

fn apply_subagent(progress: &mut ToolCallProgress, data: &Value) {
    let group = string_field(data, "group_id").unwrap_or_else(|| "cloud".to_string());
    let row = data
        .get("row_index")
        .and_then(Value::as_u64)
        .map(|row| row.to_string());
    let label = string_field(data, "label")
        .or_else(|| string_field(data, "summary"))
        .unwrap_or_else(|| "Cloud agent".to_string());
    let id = row
        .map(|row| format!("{group}:{row}"))
        .unwrap_or_else(|| format!("{group}:{label}"));
    let status = status_field(data, "status", ToolStatus::InProgress);
    let activity = string_field(data, "activity");
    let summary = string_field(data, "summary");
    if let Some(agent) = progress.agents.iter_mut().find(|agent| agent.id == id) {
        agent.label = label.clone();
        agent.status = status;
        if activity.is_some() {
            agent.activity = activity.clone();
        }
        if summary.is_some() {
            agent.summary = summary.clone();
        }
    } else {
        progress.agents.push(ToolProgressAgent {
            id,
            label: label.clone(),
            status,
            activity: activity.clone(),
            summary: summary.clone(),
        });
    }
    progress.latest_activity = activity.or(summary).or(Some(label));
}

fn is_terminal_event(event: &Value) -> bool {
    matches!(
        event.get("type").and_then(Value::as_str),
        Some("run_completed") | Some("run_failed") | Some("run_cancelled")
    )
}

fn first_text(value: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| string_field(value, field))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn identifier_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|identifier| {
            identifier
                .as_str()
                .map(str::to_string)
                .or_else(|| identifier.as_u64().map(|value| value.to_string()))
        })
        .filter(|value| !value.is_empty())
}

fn status_field(value: &Value, field: &str, fallback: ToolStatus) -> ToolStatus {
    match value.get(field).and_then(Value::as_str).unwrap_or_default() {
        "pending" | "queued" | "waiting" => ToolStatus::Pending,
        "running" | "in_progress" | "started" | "claimed" | "tool_call" => ToolStatus::InProgress,
        "complete" | "completed" | "done" | "partial" | "success" | "succeeded" | "tool_result" => {
            ToolStatus::Completed
        }
        "cancelled" | "canceled" => ToolStatus::Cancelled,
        "failed" | "error" => ToolStatus::Failed,
        _ => fallback,
    }
}

#[cfg(test)]
#[path = "clark_progress_tests.rs"]
mod tests;
