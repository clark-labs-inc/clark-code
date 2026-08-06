use std::collections::VecDeque;

use accessibility_sys::{
    kAXDescriptionAttribute, kAXErrorAPIDisabled, kAXErrorActionUnsupported,
    kAXErrorAttributeUnsupported, kAXErrorNotImplemented, kAXFocusedAttribute, kAXPressAction,
    kAXRoleAttribute, kAXSelectedTextAttribute, kAXSelectedTextRangeAttribute, kAXTitleAttribute,
    kAXValueAttribute, kAXValueTypeCFRange, AXUIElementPerformAction, AXUIElementSetAttributeValue,
    AXValueCreate,
};
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::CFRange;

use crate::{ComputerUseError, ElementInfo, Rect, WindowInfo};

use super::{
    action_names, children, element_bounds, matching_window_element, number_attr, string_attr,
    text_attr, OwnedElement,
};

const NODE_CAP: usize = 1_500;
const DEPTH_CAP: usize = 20;
const ELEMENT_MATCH_TOLERANCE: f64 = 3.0;

/// Re-resolve an observed control immediately before activation and prefer its
/// native Accessibility action over a global coordinate event. Returning false
/// means the app exposes no AXPress contract; the caller may use the already
/// revalidated bounds as a fallback.
pub fn press(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
) -> Result<bool, ComputerUseError> {
    let element = matching_observed_element(window, expected, expected_bounds)?;
    let action = CFString::new(kAXPressAction);
    let result = unsafe { AXUIElementPerformAction(element.0, action.as_concrete_TypeRef()) };
    if result == 0 {
        Ok(true)
    } else if result == kAXErrorActionUnsupported
        || result == kAXErrorAttributeUnsupported
        || result == kAXErrorNotImplemented
    {
        Ok(false)
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else {
        Err(ComputerUseError::Os(format!(
            "AX element activation failed with error {result}"
        )))
    }
}

pub fn verify_observed_element(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
) -> Result<(), ComputerUseError> {
    matching_observed_element(window, expected, expected_bounds).map(|_| ())
}

pub fn perform_secondary_action(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
    action: &str,
) -> Result<(), ComputerUseError> {
    let element = matching_observed_element(window, expected, expected_bounds)?;
    if !action_names(&element)
        .iter()
        .any(|advertised| advertised == action)
    {
        return Err(ComputerUseError::ObservationStale);
    }
    let action = CFString::new(action);
    let result = unsafe { AXUIElementPerformAction(element.0, action.as_concrete_TypeRef()) };
    if result == 0 {
        Ok(())
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else if result == kAXErrorActionUnsupported
        || result == kAXErrorAttributeUnsupported
        || result == kAXErrorNotImplemented
    {
        Err(ComputerUseError::ObservationStale)
    } else {
        Err(ComputerUseError::Os(format!(
            "AX secondary action failed with error {result}"
        )))
    }
}

pub fn select_text(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
    start: u32,
    end: u32,
) -> Result<(), ComputerUseError> {
    let element = matching_observed_element(window, expected, expected_bounds)?;
    if expected.sensitive_text || expected.role == "AXSecureTextField" {
        return Err(ComputerUseError::HumanHandoffRequired(
            "text selection in a secure field is credential-sensitive".to_string(),
        ));
    }
    let current = text_attr(&element, kAXValueAttribute)?.unwrap_or_default();
    let range =
        utf16_selection_range(&current, start, end).ok_or(ComputerUseError::ObservationStale)?;
    set_focus_if_supported(&element)?;
    let value = unsafe {
        AXValueCreate(
            kAXValueTypeCFRange,
            &range as *const CFRange as *const std::ffi::c_void,
        )
    };
    if value.is_null() {
        return Err(ComputerUseError::Os(
            "could not create Accessibility text range".to_string(),
        ));
    }
    let attribute = CFString::new(kAXSelectedTextRangeAttribute);
    let result = unsafe {
        AXUIElementSetAttributeValue(element.0, attribute.as_concrete_TypeRef(), value as _)
    };
    unsafe {
        CFRelease(value as _);
    }
    if result == 0 {
        Ok(())
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else if result == kAXErrorAttributeUnsupported || result == kAXErrorNotImplemented {
        Err(ComputerUseError::ObservationStale)
    } else {
        Err(ComputerUseError::Os(format!(
            "AX text selection failed with error {result}"
        )))
    }
}

pub fn set_numeric_value(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
    value: f64,
) -> Result<(), ComputerUseError> {
    let constraints = expected.value_constraints.ok_or_else(|| {
        ComputerUseError::InvalidAction("numeric constraints missing".to_string())
    })?;
    if !expected.value_settable
        || !value.is_finite()
        || value < constraints.minimum
        || value > constraints.maximum
    {
        return Err(ComputerUseError::InvalidAction(
            "numeric value is outside the observed constraints".to_string(),
        ));
    }
    let element = matching_observed_element(window, expected, expected_bounds)?;
    let attribute = CFString::new(kAXValueAttribute);
    let number = CFNumber::from(value);
    let result = unsafe {
        AXUIElementSetAttributeValue(
            element.0,
            attribute.as_concrete_TypeRef(),
            number.as_CFTypeRef(),
        )
    };
    if result == kAXErrorAPIDisabled {
        return Err(ComputerUseError::PermissionMissing("Accessibility"));
    }
    if result == kAXErrorAttributeUnsupported || result == kAXErrorNotImplemented {
        return Err(ComputerUseError::ObservationStale);
    }
    if result != 0 {
        return Err(ComputerUseError::Os(format!(
            "AX numeric value assignment failed with error {result}"
        )));
    }
    let after = number_attr(&element, kAXValueAttribute)?;
    if after.is_some_and(|after| (after - value).abs() <= 1e-7) {
        Ok(())
    } else {
        Err(ComputerUseError::ObservationStale)
    }
}

/// Prefer the target app's accessibility text contract over global synthetic
/// key events. Chromium's omnibox accepts AXValue updates but can silently
/// discard a CGEvent Unicode payload even while reporting keyboard focus.
///
/// Returns `Ok(false)` when the element does not expose a settable text
/// attribute, allowing the caller to fall back to keyboard input.
pub fn set_text(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
    text: &str,
    replace: bool,
) -> Result<bool, ComputerUseError> {
    let element = matching_observed_element(window, expected, expected_bounds)?;
    set_focus_if_supported(&element)?;

    let sensitive = expected.sensitive_text || expected.role == "AXSecureTextField";
    // Never copy a password field's current or resulting value into helper
    // memory merely to verify assignment.
    let before = if sensitive {
        None
    } else {
        text_attr(&element, kAXValueAttribute)?
    };
    let attribute = if replace || (!sensitive && before.as_deref().unwrap_or_default().is_empty()) {
        kAXValueAttribute
    } else {
        // AXSelectedText preserves insertion/replacement semantics at the
        // app's current selection instead of guessing a caret position.
        kAXSelectedTextAttribute
    };
    let attribute_name = CFString::new(attribute);
    let value = CFString::new(text);
    let result = unsafe {
        AXUIElementSetAttributeValue(
            element.0,
            attribute_name.as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    };
    if result == kAXErrorAttributeUnsupported || result == kAXErrorNotImplemented {
        return Ok(false);
    }
    if result == kAXErrorAPIDisabled {
        return Err(ComputerUseError::PermissionMissing("Accessibility"));
    }
    if result != 0 {
        return Err(ComputerUseError::Os(format!(
            "AX text assignment failed with error {result}"
        )));
    }

    if sensitive {
        return Ok(true);
    }
    let after = text_attr(&element, kAXValueAttribute)?;
    if text.is_empty()
        || after.is_none()
        || (attribute == kAXValueAttribute && after.as_deref() == Some(text))
        || (attribute == kAXSelectedTextAttribute && after != before)
    {
        Ok(true)
    } else {
        Err(ComputerUseError::Os(
            "the target app accepted AX text assignment but its value did not change".to_string(),
        ))
    }
}

fn matching_observed_element(
    window: &WindowInfo,
    expected: &ElementInfo,
    expected_bounds: Rect,
) -> Result<OwnedElement, ComputerUseError> {
    let root = matching_window_element(window)?;
    let mut queue = VecDeque::from([(root, 0_usize)]);
    let mut visited = 0_usize;
    while let Some((element, depth)) = queue.pop_front() {
        if visited >= NODE_CAP || depth > DEPTH_CAP {
            continue;
        }
        visited += 1;
        let role = string_attr(&element, kAXRoleAttribute)?.unwrap_or_default();
        let bounds = element_bounds(&element).unwrap_or_default();
        let description = string_attr(&element, kAXDescriptionAttribute)?;
        let name = string_attr(&element, kAXTitleAttribute)?;
        if role == expected.role
            && rect_matches(bounds, expected_bounds)
            && optional_identity_matches(expected.description.as_deref(), description.as_deref())
            && optional_identity_matches(expected.name.as_deref(), name.as_deref())
        {
            return Ok(element);
        }
        if depth == DEPTH_CAP {
            continue;
        }
        for child in children(&element)? {
            if queue.len() + visited >= NODE_CAP {
                break;
            }
            queue.push_back((child, depth + 1));
        }
    }
    Err(ComputerUseError::ObservationStale)
}

fn set_focus_if_supported(element: &OwnedElement) -> Result<(), ComputerUseError> {
    let attribute = CFString::new(kAXFocusedAttribute);
    let result = unsafe {
        AXUIElementSetAttributeValue(
            element.0,
            attribute.as_concrete_TypeRef(),
            CFBoolean::true_value().as_CFTypeRef(),
        )
    };
    if result == 0 || result == kAXErrorAttributeUnsupported || result == kAXErrorNotImplemented {
        Ok(())
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else {
        Err(ComputerUseError::Os(format!(
            "AX element focus failed with error {result}"
        )))
    }
}

fn rect_matches(left: Rect, right: Rect) -> bool {
    (left.x - right.x).abs() <= ELEMENT_MATCH_TOLERANCE
        && (left.y - right.y).abs() <= ELEMENT_MATCH_TOLERANCE
        && (left.width - right.width).abs() <= ELEMENT_MATCH_TOLERANCE
        && (left.height - right.height).abs() <= ELEMENT_MATCH_TOLERANCE
}

fn optional_identity_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected
        .filter(|value| !value.is_empty())
        .is_none_or(|expected| actual == Some(expected))
}

fn utf16_selection_range(text: &str, start: u32, end: u32) -> Option<CFRange> {
    if start > end {
        return None;
    }
    let character_count = text.chars().count();
    if end as usize > character_count {
        return None;
    }
    let utf16_offset = |character_index: usize| {
        text.chars()
            .take(character_index)
            .map(char::len_utf16)
            .sum::<usize>()
    };
    let location = utf16_offset(start as usize);
    let limit = utf16_offset(end as usize);
    Some(CFRange {
        location: isize::try_from(location).ok()?,
        length: isize::try_from(limit.checked_sub(location)?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::utf16_selection_range;

    #[test]
    fn selection_indices_are_bounded_and_translated_to_utf16() {
        let ascii = utf16_selection_range("abcdef", 1, 4).unwrap();
        assert_eq!(ascii.location, 1);
        assert_eq!(ascii.length, 3);

        let unicode = utf16_selection_range("a🙂éb", 1, 3).unwrap();
        assert_eq!(unicode.location, 1);
        assert_eq!(unicode.length, 3);

        assert!(utf16_selection_range("abc", 3, 2).is_none());
        assert!(utf16_selection_range("abc", 0, 4).is_none());
    }
}
