use std::sync::{Arc, Mutex};

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
    let (path, label, status) = match event {
        CoordinatorEvent::Queued { path, label } => (path, label.clone(), FanOutStatus::Queued),
        CoordinatorEvent::Running { path, .. } | CoordinatorEvent::ReworkRequested { path, .. } => {
            (path, String::new(), FanOutStatus::Running)
        }
        CoordinatorEvent::Reported { path, .. } | CoordinatorEvent::Accepted { path, .. } => {
            (path, String::new(), FanOutStatus::Done)
        }
        CoordinatorEvent::Interrupted { path } | CoordinatorEvent::Failed { path, .. } => {
            (path, String::new(), FanOutStatus::Failed)
        }
        CoordinatorEvent::Harness { .. } => return None,
    };
    Some(FanOutAgent {
        id: path.as_str().to_string(),
        label,
        status,
    })
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
            Err("external research must use clark_research".to_string())
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

        let running = fan_out_agent(&CoordinatorEvent::Running { path, attempt: 1 }).unwrap();
        assert!(running.label.is_empty());
        assert_eq!(running.status, FanOutStatus::Running);
    }
}
