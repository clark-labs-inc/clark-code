use crate::lease::InputLease;
use crate::{ActionLocation, ComputerAction, ComputerUseError, MouseButton, Point, Rect};

use super::super::{
    accessibility, input, windows, LatestElement, LatestObservation, MacServiceBackend,
    PreparedRecord,
};

const MAX_TEXT_INPUT_CHARS: usize = 2_000;

pub(super) fn validate_action_geometry(
    observation: &LatestObservation,
    action: &ComputerAction,
) -> Result<(), ComputerUseError> {
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: observation.screenshot_width as f64,
        height: observation.screenshot_height as f64,
    };
    let points = match action {
        ComputerAction::Click { point, .. } => point.iter().collect::<Vec<_>>(),
        ComputerAction::Drag { start, end, .. } => {
            start.point.iter().chain(end.point.iter()).collect()
        }
        _ => Vec::new(),
    };
    for point in points {
        if !bounds.contains(*point) {
            return Err(ComputerUseError::PointOutOfBounds {
                x: point.x,
                y: point.y,
            });
        }
    }
    if let ComputerAction::TypeText { text, .. } = action {
        if text.chars().count() > MAX_TEXT_INPUT_CHARS {
            return Err(ComputerUseError::InvalidAction(format!(
                "text input exceeds the {MAX_TEXT_INPUT_CHARS}-character lease limit"
            )));
        }
    }
    Ok(())
}

pub(super) fn execute_action(
    backend: &MacServiceBackend,
    record: &PreparedRecord,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    lease.check()?;
    windows::focus_window(&record.observation.window)?;
    lease.check()?;
    backend.ensure_window_unchanged(&record.observation.window)?;
    match &record.request.action {
        ComputerAction::Click {
            element_id,
            point,
            button,
        } => execute_click(record, element_id.as_deref(), *point, *button, lease)?,
        ComputerAction::TypeText {
            element_id,
            text,
            replace,
        } => execute_type_text(record, element_id, text, *replace, lease)?,
        ComputerAction::Keypress { key, modifiers } => {
            input::keypress(*key, modifiers, lease)?;
        }
        ComputerAction::Scroll {
            element_id,
            delta_x,
            delta_y,
        } => {
            let point = if let Some(id) = element_id {
                let element = element(record, id)?;
                accessibility::verify_observed_element(
                    &record.observation.window,
                    &element.info,
                    element.global_bounds,
                )?;
                element.global_bounds.center()
            } else {
                record.observation.window.frame.center()
            };
            input::scroll(point, *delta_x, *delta_y, lease)?;
        }
        ComputerAction::Drag {
            start,
            end,
            button,
            duration_ms,
        } => {
            let start = resolve_location(record, start)?;
            let end = resolve_location(record, end)?;
            input::drag(start, end, *button, *duration_ms, lease)?;
        }
        ComputerAction::SecondaryAction { element_id, action } => {
            let element = element(record, element_id)?;
            lease.check()?;
            accessibility::perform_secondary_action(
                &record.observation.window,
                &element.info,
                element.global_bounds,
                action,
            )?;
        }
        ComputerAction::SelectText {
            element_id,
            start,
            end,
        } => {
            let element = element(record, element_id)?;
            lease.check()?;
            accessibility::select_text(
                &record.observation.window,
                &element.info,
                element.global_bounds,
                *start,
                *end,
            )?;
        }
        ComputerAction::SetValue { element_id, value } => {
            let element = element(record, element_id)?;
            lease.check()?;
            accessibility::set_numeric_value(
                &record.observation.window,
                &element.info,
                element.global_bounds,
                *value,
            )?;
        }
    }
    lease.check()
}

fn execute_click(
    record: &PreparedRecord,
    element_id: Option<&str>,
    point: Option<Point>,
    button: MouseButton,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let point = if let Some(id) = element_id {
        element(record, id)?.global_bounds.center()
    } else {
        screenshot_point(
            &record.observation,
            point.ok_or_else(|| {
                ComputerUseError::InvalidAction("click point is missing".to_string())
            })?,
        )?
    };
    if let Some(id) = element_id {
        let element = element(record, id)?;
        lease.check()?;
        if button == MouseButton::Left
            && accessibility::press(
                &record.observation.window,
                &element.info,
                element.global_bounds,
            )?
        {
            return lease.check();
        }
        accessibility::verify_observed_element(
            &record.observation.window,
            &element.info,
            element.global_bounds,
        )?;
    }
    input::click(point, button, lease)
}

fn execute_type_text(
    record: &PreparedRecord,
    element_id: &str,
    text: &str,
    replace: bool,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let element = element(record, element_id)?;
    if element.info.sensitive_text || element.info.role == "AXSecureTextField" {
        return Err(ComputerUseError::HumanHandoffRequired(
            "credential text must be entered by the user".to_string(),
        ));
    }
    lease.check()?;
    if accessibility::set_text(
        &record.observation.window,
        &element.info,
        element.global_bounds,
        text,
        replace,
    )? {
        return lease.check();
    }
    input::click(element.global_bounds.center(), MouseButton::Left, lease)?;
    if replace {
        input::keypress(
            crate::Key::Character('a'),
            &[crate::Modifier::Command],
            lease,
        )?;
    }
    input::type_text(text, lease)
}

fn resolve_location(
    record: &PreparedRecord,
    location: &ActionLocation,
) -> Result<Point, ComputerUseError> {
    if let Some(id) = location.element_id.as_deref() {
        let element = element(record, id)?;
        accessibility::verify_observed_element(
            &record.observation.window,
            &element.info,
            element.global_bounds,
        )?;
        return Ok(element.global_bounds.center());
    }
    screenshot_point(
        &record.observation,
        location.point.ok_or_else(|| {
            ComputerUseError::InvalidAction("action location is missing".to_string())
        })?,
    )
}

fn screenshot_point(
    observation: &LatestObservation,
    point: Point,
) -> Result<Point, ComputerUseError> {
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: observation.screenshot_width as f64,
        height: observation.screenshot_height as f64,
    };
    if !bounds.contains(point) {
        return Err(ComputerUseError::PointOutOfBounds {
            x: point.x,
            y: point.y,
        });
    }
    let frame = observation.window.frame;
    let global = Point {
        x: frame.x + point.x / observation.screenshot_width as f64 * frame.width,
        y: frame.y + point.y / observation.screenshot_height as f64 * frame.height,
    };
    if frame.contains(global) {
        Ok(global)
    } else {
        Err(ComputerUseError::ObservationStale)
    }
}

fn element<'a>(
    record: &'a PreparedRecord,
    id: &str,
) -> Result<&'a LatestElement, ComputerUseError> {
    record
        .observation
        .elements
        .get(id)
        .ok_or_else(|| ComputerUseError::ElementNotFound(id.to_string()))
}
