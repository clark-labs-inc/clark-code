use crate::domain::ChecklistStatus;
use crate::ids::RunId;

use super::{Snapshot, TimelineItem};

/// A completed standing goal is the typed declaration that its work is done.
/// Close only the checklist tied to that same run: a terminal turn by itself
/// can still be a deliberate pause for user input.
pub(super) fn complete_run_checklist(snapshot: &mut Snapshot, run: &RunId) {
    let current_checklist_is_for_run = snapshot
        .timeline
        .iter()
        .rev()
        .find_map(|item| match item {
            TimelineItem::ExecutionChecklist {
                run: Some(checklist_run),
                ..
            } => Some(checklist_run == run),
            _ => None,
        })
        .unwrap_or(false);

    let completed = {
        let Some(TimelineItem::ExecutionChecklist { checklist, .. }) =
            snapshot.timeline.iter_mut().rev().find(|item| {
                matches!(
                    item,
                    TimelineItem::ExecutionChecklist {
                        run: Some(checklist_run),
                        ..
                    } if checklist_run == run
                )
            })
        else {
            return;
        };

        if checklist
            .steps
            .iter()
            .any(|step| step.status != ChecklistStatus::Completed)
        {
            for step in &mut checklist.steps {
                step.status = ChecklistStatus::Completed;
            }
            checklist.revision = checklist.revision.saturating_add(1);
        }
        checklist.clone()
    };

    // `Snapshot::execution_checklist` mirrors the newest checklist card. Do
    // not overwrite a newer parallel run's card while closing an older goal.
    if current_checklist_is_for_run {
        snapshot.execution_checklist = Some(completed);
    }
}
