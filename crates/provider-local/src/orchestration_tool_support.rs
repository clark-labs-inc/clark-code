use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::domain::{FanOutAgent, FanOutStatus};
use agent_orchestration::{
    AgentRole, AgentStatus, CoordinatorEvent, CoordinatorEventSink, OrchestrationPurpose,
};

use crate::tools::ToolCtx;

pub(super) fn event_sink(
    ctx: &ToolCtx,
    execution: Option<crate::root_execution::RootExecutionTrace>,
) -> (CoordinatorEventSink, Arc<Mutex<Vec<CoordinatorEvent>>>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_events = captured.clone();
    let progress = ctx.progress.clone();
    let agent_progress = ctx.agent_progress.clone();
    let sink = Arc::new(move |event: CoordinatorEvent| {
        if let Some(execution) = &execution {
            match &event {
                CoordinatorEvent::Running { path, .. } => {
                    execution.update_child(path.clone(), AgentStatus::Running);
                }
                CoordinatorEvent::Reported { path, .. }
                | CoordinatorEvent::Accepted { path, .. } => {
                    execution.update_child(path.clone(), AgentStatus::Completed);
                }
                CoordinatorEvent::ReworkRequested { path, .. }
                | CoordinatorEvent::Interrupted { path } => {
                    execution.update_child(path.clone(), AgentStatus::Interrupted);
                }
                CoordinatorEvent::Failed { path, .. } => {
                    execution.update_child(path.clone(), AgentStatus::Errored);
                }
                CoordinatorEvent::Queued { .. } | CoordinatorEvent::Harness { .. } => {}
            }
        }
        if let Some(progress) = &progress {
            progress(render_event(&event));
        }
        if let (Some(agent_progress), Some(agent)) = (&agent_progress, fan_out_agent(&event)) {
            agent_progress(agent);
        }
        captured_events.lock().expect("event lock").push(event);
    });
    (sink, captured)
}

fn fan_out_agent(event: &CoordinatorEvent) -> Option<FanOutAgent> {
    let now = now_ms();
    let update = |path: &agent_orchestration::AgentPath,
                  label: String,
                  status: FanOutStatus,
                  objective: Option<String>,
                  activity: Option<String>,
                  result: Option<String>,
                  attempt: Option<u32>,
                  started_at_ms: Option<u64>| FanOutAgent {
        id: path.as_str().to_string(),
        label,
        status,
        objective,
        activity,
        result,
        attempt,
        started_at_ms,
        updated_at_ms: Some(now),
    };
    Some(match event {
        CoordinatorEvent::Queued { path, label } => update(
            path,
            label.clone(),
            FanOutStatus::Queued,
            Some(label.clone()),
            Some("Waiting to start".into()),
            None,
            None,
            None,
        ),
        CoordinatorEvent::Running { path, attempt } => update(
            path,
            String::new(),
            FanOutStatus::Running,
            None,
            Some("Working on the delegated task".into()),
            None,
            Some(*attempt),
            Some(now),
        ),
        CoordinatorEvent::Harness { path, detail } => {
            let activity = match detail {
                agent_orchestration::HarnessEvent::Progress { message }
                | agent_orchestration::HarnessEvent::Warning { message } => message.clone(),
                agent_orchestration::HarnessEvent::Tool { summary, .. } => summary.clone(),
            };
            update(
                path,
                String::new(),
                FanOutStatus::Running,
                None,
                Some(activity),
                None,
                None,
                None,
            )
        }
        CoordinatorEvent::Reported { path, report } => update(
            path,
            String::new(),
            FanOutStatus::Done,
            None,
            Some("Report ready".into()),
            Some(report.summary.clone()),
            Some(report.attempt),
            None,
        ),
        CoordinatorEvent::Accepted { path, attempt } => update(
            path,
            String::new(),
            FanOutStatus::Done,
            None,
            Some("Complete".into()),
            None,
            Some(*attempt),
            None,
        ),
        CoordinatorEvent::ReworkRequested { path, attempt, .. } => update(
            path,
            String::new(),
            FanOutStatus::Running,
            None,
            Some("Reworking the delegated task".into()),
            None,
            Some(*attempt),
            Some(now),
        ),
        CoordinatorEvent::Interrupted { path } => update(
            path,
            String::new(),
            FanOutStatus::Failed,
            None,
            Some("Interrupted".into()),
            Some("The subagent was interrupted".into()),
            None,
            None,
        ),
        CoordinatorEvent::Failed { path, error } => update(
            path,
            String::new(),
            FanOutStatus::Failed,
            None,
            Some("Needs attention".into()),
            Some(error.clone()),
            None,
            None,
        ),
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn render_event(event: &CoordinatorEvent) -> String {
    match event {
        CoordinatorEvent::Queued { path, label } => format!("Queued {path}: {label}"),
        CoordinatorEvent::Running { path, attempt } => {
            format!("Running {path} (attempt {attempt})")
        }
        CoordinatorEvent::Reported { path, .. } => format!("Reported {path}"),
        CoordinatorEvent::Accepted { path, .. } => format!("Accepted {path}"),
        CoordinatorEvent::ReworkRequested { path, .. } => format!("Reworking {path}"),
        CoordinatorEvent::Interrupted { path } => format!("Interrupted {path}"),
        CoordinatorEvent::Failed { path, error } => format!("Failed {path}: {error}"),
        CoordinatorEvent::Harness { path, detail } => format!("{path}: {detail:?}"),
    }
}

pub(super) fn role_for_purpose(purpose: OrchestrationPurpose) -> Result<AgentRole, String> {
    match purpose {
        OrchestrationPurpose::Explore => Ok(AgentRole::Explorer),
        OrchestrationPurpose::Review => Ok(AgentRole::Reviewer),
        OrchestrationPurpose::Verify => Ok(AgentRole::Verifier),
        OrchestrationPurpose::ExternalResearch => {
            Err("external research must use an installed research capability".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_orchestration::AgentPath;

    use super::*;

    #[test]
    fn coordinator_events_become_typed_agent_progress() {
        let path = AgentPath::parse("/root/api").unwrap();
        let queued = fan_out_agent(&CoordinatorEvent::Queued {
            path: path.clone(),
            label: "Inspect the API".into(),
        })
        .unwrap();
        assert_eq!(queued.id, "/root/api");
        assert_eq!(queued.label, "Inspect the API");
        assert_eq!(queued.status, FanOutStatus::Queued);
        assert_eq!(queued.objective.as_deref(), Some("Inspect the API"));
        assert_eq!(queued.activity.as_deref(), Some("Waiting to start"));

        let running = fan_out_agent(&CoordinatorEvent::Running { path, attempt: 1 }).unwrap();
        assert!(running.label.is_empty());
        assert_eq!(running.status, FanOutStatus::Running);
        assert_eq!(running.attempt, Some(1));
        assert!(running.started_at_ms.is_some());
    }
}
