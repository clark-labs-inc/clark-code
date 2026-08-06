use super::*;
use crate::{
    ActionAuthorization, ActionDisposition, ActionIntent, ActionLocation, ActionRisk,
    ComputerAction, Key, Modifier, MouseButton, PrepareActionRequest, ReceiptOutcome,
};

fn backend() -> SimulatedComputerBackend {
    SimulatedComputerBackend::new()
}

fn intent(risk: ActionRisk, reason: &str) -> ActionIntent {
    ActionIntent {
        risk,
        reason: reason.to_string(),
    }
}

#[test]
fn simulator_requires_observe_between_actions() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let request = ClickRequest {
        intent: intent(ActionRisk::Routine, "open the benign example"),
        window: target.clone(),
        observation_id: "not-observed".to_string(),
        element_id: Some("ax-2".to_string()),
        point: None,
        button: MouseButton::Left,
        dry_run: false,
    };
    assert!(matches!(
        backend.click(request.clone()),
        Err(ComputerUseError::ObservationRequired)
    ));
    let observed = backend.observe(&target).unwrap();
    let request = ClickRequest {
        observation_id: observed.observation_id,
        ..request
    };
    backend.click(request.clone()).unwrap();
    assert!(matches!(
        backend.click(request),
        Err(ComputerUseError::ObservationRequired)
    ));
    assert_eq!(backend.snapshot().1, "Opened example");
}

#[test]
fn simulator_types_and_returns_a_real_image() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let observed = backend.observe(&target).unwrap();
    assert_eq!(&observed.screenshot.png[..8], b"\x89PNG\r\n\x1a\n");
    backend
        .type_text(TypeTextRequest {
            intent: intent(ActionRisk::Routine, "enter text into the simulator"),
            window: target,
            observation_id: observed.observation_id,
            element_id: "ax-1".to_string(),
            text: "hello Clark".to_string(),
            replace: true,
            dry_run: false,
        })
        .unwrap();
    assert_eq!(backend.snapshot().0, "hello Clark");
}

#[test]
fn simulator_gates_important_clicks_and_shortcuts() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let observed = backend.observe(&target).unwrap();
    let error = backend
        .click(ClickRequest {
            intent: intent(ActionRisk::Routine, "delete the record"),
            window: target.clone(),
            observation_id: observed.observation_id,
            element_id: Some("ax-4".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: false,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        ComputerUseError::RiskDeclarationMismatch {
            required: ActionRisk::Destructive,
            ..
        }
    ));

    let observed = backend.observe(&target).unwrap();
    let error = backend
        .keypress(KeyPressRequest {
            intent: intent(ActionRisk::Routine, "quit the app"),
            window: target,
            observation_id: observed.observation_id,
            key: Key::Character('q'),
            modifiers: vec![Modifier::Command],
            dry_run: false,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        ComputerUseError::RiskDeclarationMismatch {
            required: ActionRisk::Destructive,
            ..
        }
    ));
}

#[test]
fn simulator_rejects_stale_capabilities_and_non_actionable_elements() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let first = backend.observe(&target).unwrap();
    let second = backend.observe(&target).unwrap();
    assert!(matches!(
        backend.click(ClickRequest {
            intent: intent(ActionRisk::Routine, "open the benign example"),
            window: target.clone(),
            observation_id: first.observation_id,
            element_id: Some("ax-2".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: false,
        }),
        Err(ComputerUseError::ObservationStale)
    ));
    assert!(matches!(
        backend.click(ClickRequest {
            intent: intent(ActionRisk::Routine, "click the window"),
            window: target.clone(),
            observation_id: second.observation_id.clone(),
            element_id: Some("ax-0".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: false,
        }),
        Err(ComputerUseError::ElementNotActionable(_))
    ));
    backend
        .click(ClickRequest {
            intent: intent(ActionRisk::Routine, "open the benign example"),
            window: target,
            observation_id: second.observation_id,
            element_id: Some("ax-2".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: false,
        })
        .unwrap();
}

#[test]
fn prepared_actions_use_trusted_dispositions_and_single_use_commit() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let observed = backend.observe(&target).unwrap();
    let prepared = backend
        .prepare_action(PrepareActionRequest {
            intent: intent(ActionRisk::Routine, "open the example"),
            window: target.clone(),
            observation_id: observed.observation_id,
            action: ComputerAction::Click {
                element_id: Some("ax-2".to_string()),
                point: None,
                button: MouseButton::Left,
            },
            dry_run: false,
        })
        .unwrap();
    assert_eq!(
        prepared.assessment.disposition,
        ActionDisposition::PreapprovalEligible
    );
    backend
        .authorize_action(&prepared.id, ActionAuthorization::Durable)
        .unwrap();
    let receipt = backend.commit_action(&prepared.id).unwrap();
    assert_eq!(receipt.outcome, ReceiptOutcome::Succeeded);
    assert!(matches!(
        backend.commit_action(&prepared.id),
        Err(ComputerUseError::PreparedActionNotFound(_))
    ));

    let observed = backend.observe(&target).unwrap();
    let preapproved = backend
        .prepare_action(PrepareActionRequest {
            intent: intent(ActionRisk::Routine, "open the example again"),
            window: target,
            observation_id: observed.observation_id,
            action: ComputerAction::Click {
                element_id: Some("ax-2".to_string()),
                point: None,
                button: MouseButton::Left,
            },
            dry_run: false,
        })
        .unwrap();
    assert_eq!(preapproved.assessment.disposition, ActionDisposition::Allow);
    backend.commit_action(&preapproved.id).unwrap();
}

#[test]
fn prepared_text_and_receipts_are_redacted() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;
    let observed = backend.observe(&target).unwrap();
    let secret_marker = "sensitive-marker-that-must-not-persist";
    let prepared = backend
        .prepare_action(PrepareActionRequest {
            intent: intent(ActionRisk::Routine, "enter reviewed text"),
            window: target,
            observation_id: observed.observation_id,
            action: ComputerAction::TypeText {
                element_id: "ax-1".to_string(),
                text: secret_marker.to_string(),
                replace: true,
            },
            dry_run: false,
        })
        .unwrap();
    let serialized = serde_json::to_string(&prepared).unwrap();
    assert!(!serialized.contains(secret_marker));
    backend
        .authorize_action(&prepared.id, ActionAuthorization::Once)
        .unwrap();
    let receipt = backend.commit_action(&prepared.id).unwrap();
    let serialized = serde_json::to_string(&receipt).unwrap();
    assert!(!serialized.contains(secret_marker));
    assert!(receipt.payload_summary.contains("redacted"));
}

#[test]
fn cancellation_and_physical_takeover_quiesce_long_input() {
    fn prepared_drag(backend: &SimulatedComputerBackend) -> String {
        let target = SimulatedComputerBackend::window().target;
        let observed = backend.observe(&target).unwrap();
        let prepared = backend
            .prepare_action(PrepareActionRequest {
                intent: intent(ActionRisk::Ambiguous, "move the reviewed item"),
                window: target,
                observation_id: observed.observation_id,
                action: ComputerAction::Drag {
                    start: ActionLocation {
                        element_id: Some("ax-2".to_string()),
                        point: None,
                    },
                    end: ActionLocation {
                        element_id: Some("ax-4".to_string()),
                        point: None,
                    },
                    button: MouseButton::Left,
                    duration_ms: 500,
                },
                dry_run: false,
            })
            .unwrap();
        backend
            .authorize_action(&prepared.id, ActionAuthorization::Once)
            .unwrap();
        prepared.id
    }

    let backend = std::sync::Arc::new(backend());
    let prepared = prepared_drag(&backend);
    let worker_backend = backend.clone();
    let worker = std::thread::spawn(move || worker_backend.commit_action(&prepared));
    for _ in 0..100 {
        if backend.leases.has_active() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let ack = backend.cancel_active().unwrap();
    assert!(ack.quiesced);
    assert!(matches!(
        worker.join().unwrap(),
        Err(ComputerUseError::InputCancelled)
    ));

    let prepared = prepared_drag(&backend);
    let worker_backend = backend.clone();
    let worker = std::thread::spawn(move || worker_backend.commit_action(&prepared));
    for _ in 0..100 {
        if backend.leases.has_active() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    backend.simulate_user_takeover();
    assert!(matches!(
        worker.join().unwrap(),
        Err(ComputerUseError::UserTakeover)
    ));
}

#[test]
fn simulator_exercises_every_bounded_action_and_diffs_post_action_state() {
    let backend = backend();
    let target = SimulatedComputerBackend::window().target;

    fn run(
        backend: &SimulatedComputerBackend,
        target: &crate::WindowTarget,
        risk: ActionRisk,
        action: ComputerAction,
    ) -> crate::ActionReceipt {
        let observed = backend.observe(target).unwrap();
        let prepared = backend
            .prepare_action(PrepareActionRequest {
                intent: intent(risk, "exercise the reviewed simulator action"),
                window: target.clone(),
                observation_id: observed.observation_id,
                action,
                dry_run: false,
            })
            .unwrap();
        if prepared.assessment.disposition != ActionDisposition::Allow {
            backend
                .authorize_action(&prepared.id, ActionAuthorization::Once)
                .unwrap();
        }
        backend.commit_action(&prepared.id).unwrap()
    }

    let receipt = run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::TypeText {
            element_id: "ax-1".to_string(),
            text: "abcdef".to_string(),
            replace: true,
        },
    );
    assert_eq!(receipt.outcome, ReceiptOutcome::Succeeded);
    let after_text = backend.observe(&target).unwrap();
    let diff = after_text
        .accessibility_diff
        .expect("post-action observation should retain a diff baseline");
    assert_eq!(diff.base_observation_id, "sim-observation-0");
    assert!(diff
        .changed
        .iter()
        .any(|change| change.id == "ax-1" && change.fields.contains(&"value".to_string())));
    assert!(diff
        .changed
        .iter()
        .any(|change| change.id == "ax-3" && change.fields.contains(&"value".to_string())));

    run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::Click {
            element_id: Some("ax-2".to_string()),
            point: None,
            button: MouseButton::Left,
        },
    );
    assert_eq!(backend.snapshot().1, "Opened example");

    run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::Keypress {
            key: Key::ArrowDown,
            modifiers: Vec::new(),
        },
    );
    assert_eq!(backend.snapshot().1, "Pressed ArrowDown");

    run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::Scroll {
            element_id: Some("ax-5".to_string()),
            delta_x: 0,
            delta_y: 240,
        },
    );
    assert_eq!(backend.snapshot().1, "Scrolled 0,240");

    run(
        &backend,
        &target,
        ActionRisk::Ambiguous,
        ComputerAction::Drag {
            start: ActionLocation {
                element_id: Some("ax-2".to_string()),
                point: None,
            },
            end: ActionLocation {
                element_id: Some("ax-4".to_string()),
                point: None,
            },
            button: MouseButton::Left,
            duration_ms: 50,
        },
    );
    assert_eq!(backend.snapshot().1, "Dragged");

    run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::SecondaryAction {
            element_id: "ax-5".to_string(),
            action: "AXIncrement".to_string(),
        },
    );
    assert_eq!(backend.snapshot().1, "Performed AXIncrement on ax-5");

    run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::SelectText {
            element_id: "ax-1".to_string(),
            start: 1,
            end: 4,
        },
    );
    assert_eq!(backend.snapshot().1, "Selected 1..4");

    let value_receipt = run(
        &backend,
        &target,
        ActionRisk::Routine,
        ComputerAction::SetValue {
            element_id: "ax-5".to_string(),
            value: 73.0,
        },
    );
    assert_eq!(value_receipt.payload_summary, "numeric value redacted");
    assert!(!serde_json::to_string(&value_receipt)
        .unwrap()
        .contains("73.0"));
    assert_eq!(backend.snapshot().1, "Value set");
}
