use crate::{
    ActionDisposition, ActionIntent, ActionRisk, ApplicationIdentity, ComputerAction,
    ComputerUseError, ElementInfo, MouseButton, TrustedActionAssessment, WindowInfo,
};

const MAX_SCROLL_DELTA: i32 = 1_200;
const MIN_DRAG_DURATION_MS: u32 = 50;
const MAX_DRAG_DURATION_MS: u32 = 2_000;
const MAX_SELECTION_INDEX: u32 = 20_000;
const SECONDARY_ACTION_ALLOWLIST: &[&str] = &[
    "AXPress",
    "AXShowMenu",
    "AXConfirm",
    "AXCancel",
    "AXIncrement",
    "AXDecrement",
];

pub fn assess_proposed_action(
    window: &WindowInfo,
    application: &ApplicationIdentity,
    elements: &[ElementInfo],
    intent: &ActionIntent,
    action: &ComputerAction,
    dry_run: bool,
    durable_grant: bool,
) -> Result<TrustedActionAssessment, ComputerUseError> {
    if let Err(error) = super::ensure_window_allowed(window) {
        return Ok(denied("target_forbidden", error.to_string(), intent));
    }
    if application.bundle_id != window.target.bundle_id {
        return Err(ComputerUseError::TargetChanged(
            "running application signing identity does not match the observed bundle".to_string(),
        ));
    }

    let (risk, reason_code, reason) = match action {
        ComputerAction::Click {
            element_id,
            point,
            button,
        } => {
            if element_id.is_some() == point.is_some() {
                return Ok(denied(
                    "invalid_click_target",
                    "click must identify exactly one observed element or screenshot point",
                    intent,
                ));
            }
            let element = element_id
                .as_deref()
                .map(|id| actionable_element(elements, id))
                .transpose()?;
            let assessed = super::assess_click(window, element, point.is_some(), *button);
            (assessed.risk, "click_semantics", assessed.reason)
        }
        ComputerAction::TypeText { element_id, .. } => {
            let element = enabled_element(elements, element_id)?;
            if !matches!(
                element.role.as_str(),
                "AXTextField" | "AXTextArea" | "AXSearchField" | "AXSecureTextField" | "AXComboBox"
            ) {
                return Ok(denied(
                    "not_a_text_control",
                    format!(
                        "{element_id} is {}, not a supported text control",
                        element.role
                    ),
                    intent,
                ));
            }
            let assessed = super::assess_type_text(window, element);
            (assessed.risk, "text_destination_semantics", assessed.reason)
        }
        ComputerAction::Keypress { key, modifiers } => {
            if modifiers.len() > 4 || has_duplicates(modifiers) {
                return Ok(denied(
                    "invalid_keypress",
                    "keypress modifiers must be unique and contain at most four values",
                    intent,
                ));
            }
            let assessed =
                super::assess_keypress(window, elements.iter().cloned(), *key, modifiers);
            (assessed.risk, "keypress_semantics", assessed.reason)
        }
        ComputerAction::Scroll {
            element_id,
            delta_x,
            delta_y,
        } => {
            if (*delta_x == 0 && *delta_y == 0)
                || delta_x.unsigned_abs() > MAX_SCROLL_DELTA as u32
                || delta_y.unsigned_abs() > MAX_SCROLL_DELTA as u32
            {
                return Ok(denied(
                    "scroll_out_of_bounds",
                    format!(
                        "scroll deltas must be non-zero and each within ±{MAX_SCROLL_DELTA} pixels"
                    ),
                    intent,
                ));
            }
            if let Some(id) = element_id {
                enabled_element(elements, id)?;
            }
            (
                ActionRisk::Routine,
                "bounded_scroll",
                "the scroll is bounded to the observed window".to_string(),
            )
        }
        ComputerAction::Drag {
            start,
            end,
            button,
            duration_ms,
        } => {
            if *button != MouseButton::Left
                || !(MIN_DRAG_DURATION_MS..=MAX_DRAG_DURATION_MS).contains(duration_ms)
                || !valid_location(start)
                || !valid_location(end)
            {
                return Ok(denied(
                    "invalid_drag",
                    format!(
                        "drag requires one start and end target, the left button, and a {MIN_DRAG_DURATION_MS}..={MAX_DRAG_DURATION_MS} ms duration"
                    ),
                    intent,
                ));
            }
            for id in [start.element_id.as_deref(), end.element_id.as_deref()]
                .into_iter()
                .flatten()
            {
                enabled_element(elements, id)?;
            }
            (
                ActionRisk::Ambiguous,
                "drag_semantics",
                "drag-and-drop has application-defined effects and needs action-time review"
                    .to_string(),
            )
        }
        ComputerAction::SecondaryAction { element_id, action } => {
            let element = enabled_element(elements, element_id)?;
            if !element
                .actions
                .iter()
                .any(|advertised| advertised == action)
            {
                return Ok(denied(
                    "action_not_advertised",
                    format!("{element_id} did not advertise Accessibility action {action}"),
                    intent,
                ));
            }
            if !SECONDARY_ACTION_ALLOWLIST.contains(&action.as_str()) {
                return Ok(denied(
                    "secondary_action_not_allowed",
                    format!("Accessibility action {action} is outside the bounded allowlist"),
                    intent,
                ));
            }
            let assessed = super::assess_click(window, Some(element), false, MouseButton::Left);
            if matches!(action.as_str(), "AXShowMenu" | "AXCancel")
                && assessed.risk == ActionRisk::Routine
            {
                (
                    ActionRisk::Ambiguous,
                    "contextual_secondary_action",
                    format!("{action} has context-dependent application effects"),
                )
            } else {
                (
                    assessed.risk,
                    "advertised_secondary_action",
                    assessed.reason,
                )
            }
        }
        ComputerAction::SelectText {
            element_id,
            start,
            end,
        } => {
            let element = enabled_element(elements, element_id)?;
            if !matches!(
                element.role.as_str(),
                "AXTextField" | "AXTextArea" | "AXSearchField" | "AXSecureTextField"
            ) || start > end
                || *end > MAX_SELECTION_INDEX
            {
                return Ok(denied(
                    "invalid_text_selection",
                    "selection requires a supported text control and a bounded start <= end range",
                    intent,
                ));
            }
            if element.sensitive_text || element.role == "AXSecureTextField" {
                (
                    ActionRisk::Credential,
                    "secure_text_selection",
                    "the selection target is a secure or protected text field".to_string(),
                )
            } else {
                (
                    ActionRisk::Routine,
                    "bounded_text_selection",
                    "the selection is bounded to an observed text control".to_string(),
                )
            }
        }
        ComputerAction::SetValue { element_id, value } => {
            let element = enabled_element(elements, element_id)?;
            let Some(constraints) = element.value_constraints else {
                return Ok(denied(
                    "value_constraints_missing",
                    "direct value setting requires observed numeric constraints",
                    intent,
                ));
            };
            if !element.value_settable
                || !matches!(element.role.as_str(), "AXSlider" | "AXIncrementor")
                || !value.is_finite()
                || *value < constraints.minimum
                || *value > constraints.maximum
                || !step_aligned(*value, constraints.minimum, constraints.step)
            {
                return Ok(denied(
                    "value_out_of_constraints",
                    "the numeric value is not settable within the observed range and step",
                    intent,
                ));
            }
            let assessed = super::assess_click(window, Some(element), false, MouseButton::Left);
            (assessed.risk, "constrained_numeric_value", assessed.reason)
        }
    };

    let disposition = if dry_run {
        ActionDisposition::Allow
    } else {
        match risk {
            ActionRisk::Credential | ActionRisk::SecuritySensitive => {
                ActionDisposition::MandatoryHandoff
            }
            ActionRisk::Routine if durable_grant => ActionDisposition::Allow,
            ActionRisk::Routine if application.durable_approval_eligible => {
                ActionDisposition::PreapprovalEligible
            }
            ActionRisk::Routine => ActionDisposition::ActionTimeConfirmation,
            ActionRisk::Destructive
            | ActionRisk::Financial
            | ActionRisk::ExternalCommunication
            | ActionRisk::Ambiguous => ActionDisposition::ActionTimeConfirmation,
        }
    };
    Ok(TrustedActionAssessment {
        risk,
        disposition,
        reason_code: reason_code.to_string(),
        reason,
        model_underclassified: risk_rank(intent.risk) < risk_rank(risk),
    })
}

fn denied(
    reason_code: impl Into<String>,
    reason: impl Into<String>,
    intent: &ActionIntent,
) -> TrustedActionAssessment {
    TrustedActionAssessment {
        risk: ActionRisk::SecuritySensitive,
        disposition: ActionDisposition::Deny,
        reason_code: reason_code.into(),
        reason: reason.into(),
        model_underclassified: intent.risk != ActionRisk::SecuritySensitive,
    }
}

fn enabled_element<'a>(
    elements: &'a [ElementInfo],
    id: &str,
) -> Result<&'a ElementInfo, ComputerUseError> {
    let element = elements
        .iter()
        .find(|element| element.id == id)
        .ok_or_else(|| ComputerUseError::ElementNotFound(id.to_string()))?;
    if !element.enabled {
        return Err(ComputerUseError::ElementDisabled(id.to_string()));
    }
    Ok(element)
}

fn actionable_element<'a>(
    elements: &'a [ElementInfo],
    id: &str,
) -> Result<&'a ElementInfo, ComputerUseError> {
    let element = enabled_element(elements, id)?;
    if !element.actionable {
        return Err(ComputerUseError::ElementNotActionable(id.to_string()));
    }
    Ok(element)
}

fn valid_location(location: &crate::ActionLocation) -> bool {
    location.element_id.is_some() != location.point.is_some()
        && location
            .point
            .is_none_or(|point| point.x.is_finite() && point.y.is_finite())
}

fn step_aligned(value: f64, minimum: f64, step: Option<f64>) -> bool {
    let Some(step) = step.filter(|step| step.is_finite() && *step > 0.0) else {
        return true;
    };
    let quotient = (value - minimum) / step;
    (quotient - quotient.round()).abs() <= 1e-7
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

fn risk_rank(risk: ActionRisk) -> u8 {
    match risk {
        ActionRisk::Routine => 0,
        ActionRisk::Ambiguous => 1,
        ActionRisk::ExternalCommunication | ActionRisk::Destructive => 2,
        ActionRisk::Financial => 3,
        ActionRisk::Credential | ActionRisk::SecuritySensitive => 4,
    }
}
