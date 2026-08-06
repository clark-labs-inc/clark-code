use super::goals::{parse, PlanGoalCommand};
use super::render::TranscriptKind;
use super::session::App;
use crate::runtime::ConnectedRuntime;

pub(crate) async fn apply_command(
    app: &mut App,
    runtime: &mut ConnectedRuntime,
    line: &str,
) -> bool {
    let Some(command) = parse(line) else {
        return false;
    };
    let result = match command {
        Err(error) => Err(error),
        Ok(PlanGoalCommand::Inspect) => Ok(app.plan_goal.goal_report()),
        Ok(PlanGoalCommand::Resume(token_budget)) => resume_goal(app, runtime, token_budget).await,
    };
    match result {
        Ok(report) => push_report(app, TranscriptKind::System, report, "goal updated"),
        Err(error) => push_report(app, TranscriptKind::Error, error, "goal unchanged"),
    }
    true
}

async fn resume_goal(
    app: &mut App,
    runtime: &mut ConnectedRuntime,
    token_budget: Option<u64>,
) -> Result<String, String> {
    let goal = runtime
        .provider
        .resume_goal(&runtime.session.id, token_budget)
        .await
        .map_err(|error| format!("Clark could not resume the durable goal: {error}"))?;
    let goal_id = goal.id.clone();
    let budget = goal.token_budget;
    app.plan_goal.set_goal(goal);
    Ok(format!(
        "Resumed exact goal {goal_id}.{} Send the next instruction to continue it.",
        budget
            .map(|budget| format!(" Total token budget: {budget}."))
            .unwrap_or_default()
    ))
}

fn push_report(app: &mut App, kind: TranscriptKind, text: String, status: &str) {
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript.push(kind, text);
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = status.into();
}
