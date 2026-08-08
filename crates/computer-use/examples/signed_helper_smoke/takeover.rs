use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use computer_use::{
    ActionAuthorization, ActionDisposition, ActionIntent, ActionLocation, ActionRisk,
    ComputerAction, ComputerBackend, ComputerUseError, MouseButton, Point, PrepareActionRequest,
    WindowInfo,
};

pub(super) fn run(
    backend: Arc<dyn ComputerBackend>,
    fixture: &WindowInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut takeover_observed = false;
    for attempt in 1..=6 {
        let before = backend.observe(&fixture.target)?;
        let slider = before
            .elements
            .iter()
            .find(|element| {
                element
                    .semantic_label()
                    .as_deref()
                    .is_some_and(|label| label == "Fixture slider")
            })
            .ok_or("native fixture did not expose its slider for takeover")?;
        let y = slider.bounds.y + slider.bounds.height / 2.0;
        let left = slider.bounds.x + 4.0;
        let right = slider.bounds.x + slider.bounds.width - 4.0;
        let (start_x, end_x) = if attempt % 2 == 0 {
            (right, left)
        } else {
            (left, right)
        };
        let drag = backend.prepare_action(PrepareActionRequest {
            intent: ActionIntent {
                risk: ActionRisk::Ambiguous,
                reason: "prove physical input immediately takes over from the agent".to_string(),
            },
            window: fixture.target.clone(),
            observation_id: before.observation_id,
            action: ComputerAction::Drag {
                start: ActionLocation {
                    element_id: None,
                    point: Some(Point { x: start_x, y }),
                },
                end: ActionLocation {
                    element_id: None,
                    point: Some(Point { x: end_x, y }),
                },
                button: MouseButton::Left,
                duration_ms: 2_000,
            },
            dry_run: false,
        })?;
        if drag.assessment.disposition != ActionDisposition::ActionTimeConfirmation {
            return Err(format!(
                "expected action-time confirmation for takeover drag, got {} ({})",
                drag.assessment.disposition, drag.assessment.reason
            )
            .into());
        }
        backend.authorize_action(&drag.id, ActionAuthorization::Once)?;
        println!("physical_takeover_armed=attempt_{attempt}_of_6");
        std::io::stdout().flush()?;

        let action_backend = backend.clone();
        let drag_id = drag.id.clone();
        let action = std::thread::spawn(move || action_backend.commit_action(&drag_id));
        let outcome = action
            .join()
            .map_err(|_| "physical-takeover action thread panicked")?;
        match outcome {
            Err(ComputerUseError::UserTakeover) => {
                takeover_observed = true;
                break;
            }
            Ok(_) => {}
            Err(error) => {
                return Err(format!("native takeover action failed unexpectedly: {error}").into())
            }
        }
    }
    if !takeover_observed {
        return Err("physical takeover did not stop any of six native actions".into());
    }

    let first = backend.observe(&fixture.target)?;
    let first_value = first
        .elements
        .iter()
        .find(|element| {
            element
                .semantic_label()
                .as_deref()
                .is_some_and(|label| label == "Fixture slider")
        })
        .and_then(|element| element.value.clone());
    std::thread::sleep(Duration::from_millis(150));
    let second = backend.observe(&fixture.target)?;
    let second_value = second
        .elements
        .iter()
        .find(|element| {
            element
                .semantic_label()
                .as_deref()
                .is_some_and(|label| label == "Fixture slider")
        })
        .and_then(|element| element.value.clone());
    if first_value != second_value {
        return Err(format!(
            "fixture slider changed after physical takeover: {first_value:?} -> {second_value:?}"
        )
        .into());
    }
    println!("physical_takeover=passed");
    Ok(())
}
