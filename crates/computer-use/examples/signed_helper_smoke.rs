#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
#[path = "signed_helper_smoke/takeover.rs"]
mod takeover;

#[cfg(target_os = "macos")]
use computer_use::{
    native_backend, ActionAuthorization, ActionDisposition, ActionIntent, ActionLocation,
    ActionRisk, ClickRequest, ComputerAction, ComputerBackend, ComputerUseError, MouseButton,
    PermissionRequest, PermissionStatus, Point, PrepareActionRequest, ReceiptOutcome,
    SimulatedComputerBackend, WindowFilter, WindowInfo,
};

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let simulator = SimulatedComputerBackend::new();
    let simulated_target = simulator
        .list_windows(WindowFilter::default())?
        .into_iter()
        .next()
        .ok_or("simulator window is missing")?
        .target;
    let observation = simulator.observe(&simulated_target)?;
    let underclassified = simulator
        .click(ClickRequest {
            intent: ActionIntent {
                risk: ActionRisk::Routine,
                reason: "exercise the safety classifier".to_string(),
            },
            window: simulated_target.clone(),
            observation_id: observation.observation_id.clone(),
            element_id: Some("ax-4".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: true,
        })
        .expect_err("delete must not accept routine risk");
    if !underclassified
        .to_string()
        .contains("must be `destructive`")
    {
        return Err(format!("unexpected risk rejection: {underclassified}").into());
    }
    let observation = simulator.observe(&simulated_target)?;
    simulator.click(ClickRequest {
        intent: ActionIntent {
            risk: ActionRisk::Destructive,
            reason: "validate the delete control without executing it".to_string(),
        },
        window: simulated_target,
        observation_id: observation.observation_id,
        element_id: Some("ax-4".to_string()),
        point: None,
        button: MouseButton::Left,
        dry_run: true,
    })?;
    println!("consequential_dry_run=passed");

    let approval_root = tempfile::tempdir()?;
    std::env::set_var(
        "DESKTOP_COMPUTER_USE_DATA_DIR",
        approval_root.path().join("computer-use"),
    );
    let backend = native_backend()?;
    let mut status = backend.permissions()?;
    println!(
        "helper_permissions=accessibility:{} screen_recording:{}",
        status.accessibility, status.screen_recording
    );
    if std::env::var_os("DESKTOP_COMPUTER_USE_REQUEST_PERMISSIONS").is_some() {
        status = backend.request_permissions(PermissionRequest {
            accessibility: true,
            screen_recording: true,
        })?;
        println!(
            "helper_permission_request=issued accessibility:{} screen_recording:{}",
            status.accessibility, status.screen_recording
        );
    }

    let forbidden = backend
        .launch_application("com.apple.Terminal")
        .expect_err("Terminal must be forbidden");
    if !forbidden.to_string().contains("forbids target") {
        return Err(format!("unexpected forbidden-target rejection: {forbidden}").into());
    }
    println!("forbidden_terminal=passed");

    backend.launch_application("com.google.Chrome")?;
    std::thread::sleep(Duration::from_millis(500));
    let browser_windows = backend.list_windows(WindowFilter {
        bundle_id: Some("com.google.Chrome".to_string()),
        title_contains: None,
    })?;
    if !browser_windows.is_empty() {
        return Err("generic Accessibility unexpectedly exposed a Chrome window".into());
    }
    println!("forbidden_browser=passed");

    if let Ok(bundle_id) = std::env::var("DESKTOP_COMPUTER_USE_FIXTURE_BUNDLE_ID") {
        run_native_fixture_smoke(backend, status, &bundle_id)?;
    } else {
        run_calculator_smoke(backend, status)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_calculator_smoke(
    backend: std::sync::Arc<dyn ComputerBackend>,
    status: PermissionStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    backend.launch_application("com.apple.calculator")?;
    std::thread::sleep(Duration::from_millis(500));
    let calculator = backend
        .list_windows(WindowFilter {
            bundle_id: Some("com.apple.calculator".to_string()),
            title_contains: None,
        })?
        .into_iter()
        .next()
        .ok_or("Calculator launched but no visible window was discovered")?;
    println!(
        "benign_launch=passed pid:{} window:{}",
        calculator.target.pid, calculator.target.window_id
    );

    if status.accessibility && status.screen_recording {
        let observation = backend.observe(&calculator.target)?;
        let base_observation_id = observation.observation_id.clone();
        let digit = observation
            .elements
            .iter()
            .find(|element| {
                element.actionable
                    && element
                        .semantic_label()
                        .as_deref()
                        .is_some_and(|label| label == "1")
            })
            .ok_or("Calculator observation did not expose the 1 button")?;
        let prepared = backend.prepare_action(PrepareActionRequest {
            intent: ActionIntent {
                risk: ActionRisk::Routine,
                reason: "enter one digit in Calculator".to_string(),
            },
            window: calculator.target.clone(),
            observation_id: observation.observation_id,
            action: ComputerAction::Click {
                element_id: Some(digit.id.clone()),
                point: None,
                button: MouseButton::Left,
            },
            dry_run: false,
        })?;
        if prepared.assessment.disposition != ActionDisposition::PreapprovalEligible {
            return Err(format!(
                "expected a signer-bound routine approval, got {}",
                prepared.assessment.disposition
            )
            .into());
        }
        backend.authorize_action(&prepared.id, ActionAuthorization::Once)?;
        let receipt = backend.commit_action(&prepared.id)?;
        if receipt.outcome != ReceiptOutcome::Succeeded {
            return Err(format!("unexpected Calculator action receipt: {receipt:?}").into());
        }
        let verified = backend.observe(&calculator.target)?;
        let diff = verified
            .accessibility_diff
            .as_ref()
            .ok_or("Calculator post-action observation did not include a diff")?;
        if diff.base_observation_id != base_observation_id {
            return Err(format!(
                "Calculator diff used {}, expected {}",
                diff.base_observation_id, base_observation_id
            )
            .into());
        }
        if diff.added_ids.is_empty() && diff.removed_ids.is_empty() && diff.changed.is_empty() {
            return Err("Calculator Accessibility diff was empty after clicking 1".into());
        }
        println!(
            "benign_native_click=passed receipt={} diff_base={} changed={}",
            receipt.receipt_id,
            diff.base_observation_id,
            diff.changed.len()
        );
    } else {
        println!("benign_native_click=skipped_missing_helper_tcc");
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_native_fixture_smoke(
    backend: std::sync::Arc<dyn ComputerBackend>,
    status: PermissionStatus,
    bundle_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if bundle_id != "com.agent-desktop.computer-use-fixture" {
        return Err(format!("unexpected native fixture bundle id: {bundle_id}").into());
    }
    let expected_pid = std::env::var("DESKTOP_COMPUTER_USE_FIXTURE_PID")
        .ok()
        .map(|value| value.parse::<i32>())
        .transpose()?;
    if expected_pid.is_none() {
        backend.launch_application(bundle_id)?;
    }
    let fixture = wait_for_window(backend.as_ref(), bundle_id, expected_pid)?;
    println!(
        "debug_fixture_launch=passed pid:{} window:{}",
        fixture.target.pid, fixture.target.window_id
    );

    if !status.accessibility || !status.screen_recording {
        println!("debug_fixture_actions=skipped_missing_helper_tcc");
        return Ok(());
    }

    let payload = "native fixture payload 8f31";
    let observation = backend.observe(&fixture.target)?;
    let base_observation_id = observation.observation_id.clone();
    let input_id = element_named(&observation.elements, "Fixture input")
        .ok_or("native fixture did not expose its text field")?
        .id
        .clone();
    let prepared = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Routine,
            reason: "enter disposable text in the native fixture".to_string(),
        },
        window: fixture.target.clone(),
        observation_id: observation.observation_id.clone(),
        action: ComputerAction::TypeText {
            element_id: input_id.clone(),
            text: payload.to_string(),
            replace: true,
        },
        dry_run: false,
    })?;
    if prepared.assessment.disposition != ActionDisposition::PreapprovalEligible {
        return Err(format!(
            "expected signer-bound fixture text approval, got {}",
            prepared.assessment.disposition
        )
        .into());
    }
    if prepared
        .preview
        .payload_summary
        .as_deref()
        .is_some_and(|summary| summary.contains(payload))
    {
        return Err("prepared fixture preview retained sensitive text".into());
    }
    backend.authorize_action(&prepared.id, ActionAuthorization::Once)?;
    let receipt = backend.commit_action(&prepared.id)?;
    if receipt.outcome != ReceiptOutcome::Succeeded || receipt.payload_summary.contains(payload) {
        return Err(format!("unexpected fixture text receipt: {receipt:?}").into());
    }
    if !matches!(
        backend.prepare_action(PrepareActionRequest {
            intent: ActionIntent {
                risk: ActionRisk::Routine,
                reason: "prove the observation is one use".to_string(),
            },
            window: fixture.target.clone(),
            observation_id: observation.observation_id,
            action: ComputerAction::Click {
                element_id: Some(input_id),
                point: None,
                button: MouseButton::Left,
            },
            dry_run: true,
        }),
        Err(ComputerUseError::ObservationStale)
    ) {
        return Err("the native fixture observation was reusable after preparation".into());
    }

    let after_text = backend.observe(&fixture.target)?;
    let text_diff = after_text
        .accessibility_diff
        .as_ref()
        .ok_or("fixture text action did not produce an Accessibility diff")?;
    if text_diff.base_observation_id != base_observation_id {
        return Err("fixture text diff used the wrong base observation".into());
    }
    let apply_id = element_named(&after_text.elements, "Apply text")
        .ok_or("native fixture did not expose its apply button")?
        .id
        .clone();
    let click = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Routine,
            reason: "apply the disposable fixture text".to_string(),
        },
        window: fixture.target.clone(),
        observation_id: after_text.observation_id,
        action: ComputerAction::Click {
            element_id: Some(apply_id),
            point: None,
            button: MouseButton::Left,
        },
        dry_run: false,
    })?;
    backend.authorize_action(&click.id, ActionAuthorization::Once)?;
    backend.commit_action(&click.id)?;

    let after_apply = backend.observe(&fixture.target)?;
    if !after_apply.elements.iter().any(|element| {
        element
            .value
            .as_deref()
            .or(element.name.as_deref())
            .is_some_and(|value| value.contains("Applied: native fixture payload"))
    }) {
        return Err("fixture apply action was not visible in fresh Accessibility state".into());
    }
    let secure_id = element_named(&after_apply.elements, "Fixture credential")
        .ok_or("native fixture did not expose its secure field")?
        .id
        .clone();
    let credential = "credential-must-never-persist";
    let handoff = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Credential,
            reason: "verify secure fields require direct user entry".to_string(),
        },
        window: fixture.target.clone(),
        observation_id: after_apply.observation_id,
        action: ComputerAction::TypeText {
            element_id: secure_id,
            text: credential.to_string(),
            replace: true,
        },
        dry_run: false,
    })?;
    if handoff.assessment.disposition != ActionDisposition::MandatoryHandoff
        || handoff
            .preview
            .payload_summary
            .as_deref()
            .is_some_and(|summary| summary.contains(credential))
    {
        return Err(format!("secure fixture action did not fail closed: {handoff:?}").into());
    }
    backend.authorize_action(&handoff.id, ActionAuthorization::Denied)?;

    let before_cancel = backend.observe(&fixture.target)?;
    let slider_bounds = element_named(&before_cancel.elements, "Fixture slider")
        .ok_or("native fixture did not expose its slider")?
        .bounds;
    let y = slider_bounds.y + slider_bounds.height / 2.0;
    let drag = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Ambiguous,
            reason: "exercise cancellation during a bounded fixture drag".to_string(),
        },
        window: fixture.target.clone(),
        observation_id: before_cancel.observation_id,
        action: ComputerAction::Drag {
            start: ActionLocation {
                element_id: None,
                point: Some(Point {
                    x: slider_bounds.x + 4.0,
                    y,
                }),
            },
            end: ActionLocation {
                element_id: None,
                point: Some(Point {
                    x: slider_bounds.x + slider_bounds.width - 4.0,
                    y,
                }),
            },
            button: MouseButton::Left,
            duration_ms: 2_000,
        },
        dry_run: false,
    })?;
    if drag.assessment.disposition != ActionDisposition::ActionTimeConfirmation {
        return Err(format!(
            "expected action-time confirmation for fixture drag, got {}",
            drag.assessment.disposition
        )
        .into());
    }
    backend.authorize_action(&drag.id, ActionAuthorization::Once)?;
    let commit_backend = backend.clone();
    let drag_id = drag.id.clone();
    let action = std::thread::spawn(move || commit_backend.commit_action(&drag_id));
    std::thread::sleep(Duration::from_millis(100));
    let ack = backend.cancel_active()?;
    if !ack.quiesced {
        return Err(format!("fixture cancellation was not quiesced: {ack:?}").into());
    }
    if action
        .join()
        .map_err(|_| "fixture action thread panicked")?
        .is_ok()
    {
        return Err("fixture drag completed after cancellation".into());
    }

    let first_stable = backend.observe(&fixture.target)?;
    let first_value = element_named(&first_stable.elements, "Fixture slider")
        .and_then(|element| element.value.clone());
    std::thread::sleep(Duration::from_millis(150));
    let second_stable = backend.observe(&fixture.target)?;
    let second_value = element_named(&second_stable.elements, "Fixture slider")
        .and_then(|element| element.value.clone());
    if first_value != second_value {
        return Err(format!(
            "fixture slider changed after quiesced cancellation: {first_value:?} -> {second_value:?}"
        )
        .into());
    }

    println!(
        "debug_fixture_actions=passed receipt={} cancellation_helper_terminated={}",
        receipt.receipt_id, ack.helper_terminated
    );
    if std::env::var_os("DESKTOP_COMPUTER_USE_REQUIRE_PHYSICAL_TAKEOVER").is_some() {
        takeover::run(backend.clone(), &fixture)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn wait_for_window(
    backend: &dyn ComputerBackend,
    bundle_id: &str,
    expected_pid: Option<i32>,
) -> Result<WindowInfo, Box<dyn std::error::Error>> {
    for _ in 0..30 {
        if let Some(window) = backend
            .list_windows(WindowFilter {
                bundle_id: Some(bundle_id.to_string()),
                title_contains: Some("Agent Computer Use Fixture".to_string()),
            })?
            .into_iter()
            .find(|window| expected_pid.is_none_or(|pid| window.target.pid == pid))
        {
            return Ok(window);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("native fixture launched but no visible window was discovered".into())
}

#[cfg(target_os = "macos")]
fn element_named<'a>(
    elements: &'a [computer_use::ElementInfo],
    expected: &str,
) -> Option<&'a computer_use::ElementInfo> {
    elements.iter().find(|element| {
        element
            .semantic_label()
            .as_deref()
            .is_some_and(|label| label == expected)
    })
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("signed_helper_smoke is only supported on macOS");
    std::process::exit(1);
}
