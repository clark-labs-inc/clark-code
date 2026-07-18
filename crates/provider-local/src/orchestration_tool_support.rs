use std::sync::{Arc, Mutex};

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
        captured_events.lock().expect("event lock").push(event);
    });
    (sink, captured)
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
