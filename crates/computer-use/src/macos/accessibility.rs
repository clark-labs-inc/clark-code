mod text_input;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;

use accessibility_sys::{
    kAXChildrenAttribute, kAXDescriptionAttribute, kAXEnabledAttribute, kAXErrorAPIDisabled,
    kAXErrorActionUnsupported, kAXErrorAttributeUnsupported, kAXErrorNoValue, kAXFocusedAttribute,
    kAXFrontmostAttribute, kAXHelpAttribute, kAXMaxValueAttribute, kAXMinValueAttribute,
    kAXPositionAttribute, kAXPressAction, kAXRaiseAction, kAXRoleAttribute,
    kAXSecureTextFieldSubrole, kAXSizeAttribute, kAXSubroleAttribute, kAXTitleAttribute,
    kAXValueAttribute, kAXValueIncrementAttribute, kAXValueTypeCGPoint, kAXValueTypeCGSize,
    kAXWindowsAttribute, AXUIElementCopyActionNames, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementIsAttributeSettable, AXUIElementPerformAction,
    AXUIElementRef, AXUIElementSetAttributeValue, AXUIElementSetMessagingTimeout, AXValueGetValue,
    AXValueRef,
};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFRetain, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGSize};

use crate::{ComputerUseError, ElementInfo, Rect, WindowInfo};

pub use text_input::{
    perform_secondary_action, press, select_text, set_numeric_value, set_text,
    verify_observed_element,
};

const NODE_CAP: usize = 1_500;
const DEPTH_CAP: usize = 20;
const TEXT_CAP: usize = 500;
const AX_CONTAINS_PROTECTED_CONTENT_ATTRIBUTE: &str = "AXContainsProtectedContent";

pub struct WalkElement {
    pub info: ElementInfo,
    pub global_bounds: Rect,
}

pub struct WalkResult {
    pub elements: Vec<WalkElement>,
    pub truncated: bool,
}

pub fn walk_window(
    window: &WindowInfo,
    screenshot_width: u32,
    screenshot_height: u32,
) -> Result<WalkResult, ComputerUseError> {
    let root = matching_window_element(window)?;
    let mut queue = VecDeque::from([(root, 0_usize)]);
    let mut elements = Vec::new();
    let mut truncated = false;
    while let Some((element, depth)) = queue.pop_front() {
        if elements.len() >= NODE_CAP || depth > DEPTH_CAP {
            truncated = true;
            continue;
        }
        let global_bounds = element_bounds(&element).unwrap_or_default();
        let role = string_attr(&element, kAXRoleAttribute)?.unwrap_or_default();
        let subrole = string_attr(&element, kAXSubroleAttribute)?;
        let contains_protected_content =
            bool_attr(&element, AX_CONTAINS_PROTECTED_CONTENT_ATTRIBUTE)?.unwrap_or(false);
        let sensitive_text =
            is_sensitive_text(&role, subrole.as_deref(), contains_protected_content);
        let name = string_attr(&element, kAXTitleAttribute)?;
        // Determine protection from role/subrole/AXContainsProtectedContent
        // before asking for AXValue. AppKit exposes NSSecureTextField as
        // AXTextField + AXSecureTextField subrole on current macOS, and merely
        // redacting after the read would still copy a credential into helper
        // memory.
        let value = if sensitive_text {
            None
        } else {
            text_attr(&element, kAXValueAttribute)?
        };
        let description = string_attr(&element, kAXDescriptionAttribute)?
            .or(string_attr(&element, kAXHelpAttribute)?);
        let enabled = bool_attr(&element, kAXEnabledAttribute)?.unwrap_or(true);
        let focused = bool_attr(&element, kAXFocusedAttribute)?.unwrap_or(false);
        let actions = action_names(&element);
        let value_settable = attribute_settable(&element, kAXValueAttribute)?;
        let value_constraints = if matches!(role.as_str(), "AXSlider" | "AXIncrementor") {
            match (
                number_attr(&element, kAXMinValueAttribute)?,
                number_attr(&element, kAXMaxValueAttribute)?,
            ) {
                (Some(minimum), Some(maximum))
                    if minimum.is_finite() && maximum.is_finite() && minimum <= maximum =>
                {
                    Some(crate::ValueConstraints {
                        minimum,
                        maximum,
                        step: number_attr(&element, kAXValueIncrementAttribute)?
                            .filter(|step| step.is_finite() && *step > 0.0),
                    })
                }
                _ => None,
            }
        } else {
            None
        };
        let actionable = actions.iter().any(|action| action == kAXPressAction)
            || matches!(
                role.as_str(),
                "AXButton"
                    | "AXCheckBox"
                    | "AXComboBox"
                    | "AXLink"
                    | "AXMenuItem"
                    | "AXPopUpButton"
                    | "AXRadioButton"
                    | "AXSearchField"
                    | "AXSecureTextField"
                    | "AXTextArea"
                    | "AXTextField"
            );
        let id = format!("ax-{}", elements.len());
        elements.push(WalkElement {
            info: ElementInfo {
                id,
                role,
                name,
                value,
                description,
                bounds: screenshot_bounds(
                    global_bounds,
                    window.frame,
                    screenshot_width,
                    screenshot_height,
                ),
                enabled,
                focused,
                actionable,
                actions,
                sensitive_text,
                value_settable,
                value_constraints,
            },
            global_bounds,
        });
        if depth == DEPTH_CAP {
            if !children(&element)?.is_empty() {
                truncated = true;
            }
            continue;
        }
        for child in children(&element)? {
            if queue.len() + elements.len() >= NODE_CAP {
                truncated = true;
                break;
            }
            queue.push_back((child, depth + 1));
        }
    }
    Ok(WalkResult {
        elements,
        truncated,
    })
}

fn is_sensitive_text(role: &str, subrole: Option<&str>, contains_protected_content: bool) -> bool {
    role == "AXSecureTextField"
        || subrole == Some(kAXSecureTextFieldSubrole)
        || contains_protected_content
}

pub fn raise_window(window: &WindowInfo) -> Result<(), ComputerUseError> {
    let element = matching_window_element(window)?;
    let action = CFString::new(kAXRaiseAction);
    let result = unsafe { AXUIElementPerformAction(element.0, action.as_concrete_TypeRef()) };
    if result == 0 || result == kAXErrorActionUnsupported || result == kAXErrorAttributeUnsupported
    {
        Ok(())
    } else {
        Err(ComputerUseError::Os(format!(
            "AXRaise failed with error {result}"
        )))
    }
}

pub fn focus_application(window: &WindowInfo) -> Result<(), ComputerUseError> {
    let app = unsafe { AXUIElementCreateApplication(window.target.pid) };
    if app.is_null() {
        return Err(ComputerUseError::Os(format!(
            "AXUIElementCreateApplication returned null for {}",
            window.target.pid
        )));
    }
    let app = OwnedElement(app);
    let timeout = unsafe { AXUIElementSetMessagingTimeout(app.0, 1.0) };
    if timeout == kAXErrorAPIDisabled {
        return Err(ComputerUseError::PermissionMissing("Accessibility"));
    }
    let attribute = CFString::new(kAXFrontmostAttribute);
    let value = CFBoolean::true_value();
    let result = unsafe {
        AXUIElementSetAttributeValue(app.0, attribute.as_concrete_TypeRef(), value.as_CFTypeRef())
    };
    if result == 0 {
        Ok(())
    } else {
        Err(ComputerUseError::Os(format!(
            "AXFrontmost failed with error {result}"
        )))
    }
}

fn matching_window_element(window: &WindowInfo) -> Result<OwnedElement, ComputerUseError> {
    let app = unsafe { AXUIElementCreateApplication(window.target.pid) };
    if app.is_null() {
        return Err(ComputerUseError::Os(format!(
            "AXUIElementCreateApplication returned null for {}",
            window.target.pid
        )));
    }
    let app = OwnedElement(app);
    let timeout = unsafe { AXUIElementSetMessagingTimeout(app.0, 1.0) };
    if timeout == kAXErrorAPIDisabled {
        return Err(ComputerUseError::PermissionMissing("Accessibility"));
    }
    let candidates = children_for_attribute(&app, kAXWindowsAttribute)?;
    let mut best: Option<(f64, OwnedElement)> = None;
    for candidate in candidates {
        let Some(bounds) = element_bounds(&candidate) else {
            continue;
        };
        let candidate_title = string_attr(&candidate, kAXTitleAttribute)?.unwrap_or_default();
        let score = (bounds.x - window.frame.x).abs()
            + (bounds.y - window.frame.y).abs()
            + (bounds.width - window.frame.width).abs()
            + (bounds.height - window.frame.height).abs();
        let title_matches = window_titles_match(&window.title, &candidate_title);
        if !title_matches {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score < *best_score)
        {
            best = Some((score, candidate));
        }
    }
    match best {
        Some((score, element)) if score <= 80.0 => Ok(element),
        _ => Err(ComputerUseError::WindowNotFound(format!(
            "Accessibility did not expose a window for {}",
            window.app_name
        ))),
    }
}

fn window_titles_match(cg_title: &str, ax_title: &str) -> bool {
    let cg_title = cg_title.trim();
    let ax_title = ax_title.trim();
    if cg_title.is_empty() || cg_title == ax_title {
        return true;
    }

    // Chromium exposes the page title through CGWindow but decorates the AX
    // window title with browser/profile text, for example:
    //   "Example Domain"
    //   "Example Domain - Google Chrome - Personal"
    // Keep the frame check below as the identity anchor, and only accept the
    // extra AX title when it is separated from the CG title as decoration.
    has_title_decoration(ax_title, cg_title)
        || has_title_decoration(cg_title, ax_title)
        || truncated_title_matches(cg_title, ax_title)
        || truncated_title_matches(ax_title, cg_title)
}

fn has_title_decoration(longer: &str, title: &str) -> bool {
    longer.strip_prefix(title).is_some_and(is_title_decoration)
}

fn truncated_title_matches(truncated: &str, full: &str) -> bool {
    let Some((prefix, suffix)) = truncated.split_once('…') else {
        return false;
    };
    if prefix.chars().count() + suffix.chars().count() < 8 {
        return false;
    }
    let Some(remainder) = full.strip_prefix(prefix) else {
        return false;
    };
    let Some(suffix_start) = remainder.find(suffix) else {
        return false;
    };
    let after_suffix = &remainder[suffix_start + suffix.len()..];
    after_suffix.is_empty() || is_title_decoration(after_suffix)
}

fn is_title_decoration(suffix: &str) -> bool {
    suffix.starts_with(" - ") || suffix.starts_with(" – ") || suffix.starts_with(" — ")
}

fn screenshot_bounds(global: Rect, frame: Rect, width: u32, height: u32) -> Rect {
    if frame.width <= 0.0 || frame.height <= 0.0 {
        return Rect::default();
    }
    let scale_x = width as f64 / frame.width;
    let scale_y = height as f64 / frame.height;
    Rect {
        x: (global.x - frame.x) * scale_x,
        y: (global.y - frame.y) * scale_y,
        width: global.width * scale_x,
        height: global.height * scale_y,
    }
}

fn element_bounds(element: &OwnedElement) -> Option<Rect> {
    let position = point_attr(element, kAXPositionAttribute).ok().flatten()?;
    let size = size_attr(element, kAXSizeAttribute).ok().flatten()?;
    Some(Rect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

fn string_attr(
    element: &OwnedElement,
    attribute: &str,
) -> Result<Option<String>, ComputerUseError> {
    let Some(value) = copy_attr(element, attribute)? else {
        return Ok(None);
    };
    Ok(value
        .downcast::<CFString>()
        .map(|value| bounded(value.to_string())))
}

fn text_attr(element: &OwnedElement, attribute: &str) -> Result<Option<String>, ComputerUseError> {
    let Some(value) = copy_attr(element, attribute)? else {
        return Ok(None);
    };
    if let Some(string) = value.downcast::<CFString>() {
        return Ok(Some(bounded(string.to_string())));
    }
    if let Some(number) = value.downcast::<CFNumber>() {
        return Ok(number
            .to_f64()
            .map(|value| value.to_string())
            .or_else(|| number.to_i64().map(|value| value.to_string())));
    }
    if let Some(boolean) = value.downcast::<CFBoolean>() {
        let value: bool = boolean.into();
        return Ok(Some(value.to_string()));
    }
    Ok(None)
}

fn bool_attr(element: &OwnedElement, attribute: &str) -> Result<Option<bool>, ComputerUseError> {
    let Some(value) = copy_attr(element, attribute)? else {
        return Ok(None);
    };
    Ok(value.downcast::<CFBoolean>().map(Into::into))
}

fn number_attr(element: &OwnedElement, attribute: &str) -> Result<Option<f64>, ComputerUseError> {
    let Some(value) = copy_attr(element, attribute)? else {
        return Ok(None);
    };
    Ok(value.downcast::<CFNumber>().and_then(|number| {
        number
            .to_f64()
            .or_else(|| number.to_i64().map(|value| value as f64))
    }))
}

fn attribute_settable(element: &OwnedElement, attribute: &str) -> Result<bool, ComputerUseError> {
    let attribute = CFString::new(attribute);
    let mut settable = 0_u8;
    let result = unsafe {
        AXUIElementIsAttributeSettable(element.0, attribute.as_concrete_TypeRef(), &mut settable)
    };
    if result == 0 {
        Ok(settable != 0)
    } else if result == kAXErrorAttributeUnsupported || result == kAXErrorNoValue {
        Ok(false)
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else {
        Err(ComputerUseError::Os(format!(
            "AX settable check for {attribute} failed with error {result}"
        )))
    }
}

fn point_attr(
    element: &OwnedElement,
    attribute: &str,
) -> Result<Option<CGPoint>, ComputerUseError> {
    let Some(raw) = copy_attr_raw(element, attribute)? else {
        return Ok(None);
    };
    let mut point = CGPoint::new(0.0, 0.0);
    let ok = unsafe {
        AXValueGetValue(
            raw as AXValueRef,
            kAXValueTypeCGPoint,
            &mut point as *mut _ as *mut c_void,
        )
    };
    unsafe {
        CFRelease(raw);
    }
    Ok(ok.then_some(point))
}

fn size_attr(element: &OwnedElement, attribute: &str) -> Result<Option<CGSize>, ComputerUseError> {
    let Some(raw) = copy_attr_raw(element, attribute)? else {
        return Ok(None);
    };
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        AXValueGetValue(
            raw as AXValueRef,
            kAXValueTypeCGSize,
            &mut size as *mut _ as *mut c_void,
        )
    };
    unsafe {
        CFRelease(raw);
    }
    Ok(ok.then_some(size))
}

fn children(element: &OwnedElement) -> Result<Vec<OwnedElement>, ComputerUseError> {
    children_for_attribute(element, kAXChildrenAttribute)
}

fn children_for_attribute(
    element: &OwnedElement,
    attribute: &str,
) -> Result<Vec<OwnedElement>, ComputerUseError> {
    let Some(raw) = copy_attr_raw(element, attribute)? else {
        return Ok(Vec::new());
    };
    let array: CFArray = unsafe { CFArray::wrap_under_create_rule(raw as CFArrayRef) };
    let mut children = Vec::with_capacity(array.len() as usize);
    for index in 0..array.len() {
        let Some(child) = array.get(index) else {
            continue;
        };
        let raw = *child as CFTypeRef;
        unsafe {
            CFRetain(raw);
        }
        children.push(OwnedElement(raw as AXUIElementRef));
    }
    Ok(children)
}

fn action_names(element: &OwnedElement) -> Vec<String> {
    let mut raw: CFArrayRef = ptr::null();
    let result = unsafe { AXUIElementCopyActionNames(element.0, &mut raw) };
    if result != 0 || raw.is_null() {
        return Vec::new();
    }
    let array: CFArray<CFString> = unsafe { CFArray::wrap_under_create_rule(raw) };
    array.iter().map(|action| action.to_string()).collect()
}

fn copy_attr(element: &OwnedElement, attribute: &str) -> Result<Option<CFType>, ComputerUseError> {
    let Some(raw) = copy_attr_raw(element, attribute)? else {
        return Ok(None);
    };
    Ok(Some(unsafe { CFType::wrap_under_create_rule(raw) }))
}

fn copy_attr_raw(
    element: &OwnedElement,
    attribute: &str,
) -> Result<Option<CFTypeRef>, ComputerUseError> {
    let attribute = CFString::new(attribute);
    let mut value: CFTypeRef = ptr::null();
    let result = unsafe {
        AXUIElementCopyAttributeValue(
            element.0,
            attribute.as_concrete_TypeRef() as CFStringRef,
            &mut value,
        )
    };
    if result == 0 {
        Ok((!value.is_null()).then_some(value))
    } else if result == kAXErrorNoValue || result == kAXErrorAttributeUnsupported {
        Ok(None)
    } else if result == kAXErrorAPIDisabled {
        Err(ComputerUseError::PermissionMissing("Accessibility"))
    } else {
        Err(ComputerUseError::Os(format!(
            "AX attribute {attribute} failed with error {result}"
        )))
    }
}

fn bounded(mut value: String) -> String {
    if value.chars().count() <= TEXT_CAP {
        return value;
    }
    value = value.chars().take(TEXT_CAP).collect();
    value.push('…');
    value
}

struct OwnedElement(AXUIElementRef);

impl Clone for OwnedElement {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.0 as CFTypeRef);
        }
        Self(self.0)
    }
}

impl Drop for OwnedElement {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0 as CFTypeRef);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_sensitive_text, window_titles_match};

    #[test]
    fn chrome_ax_title_decoration_matches_cg_window_title() {
        assert!(window_titles_match(
            "Example Domain",
            "Example Domain - Google Chrome - Personal"
        ));
        assert!(window_titles_match(
            "Clark Labs | An AI agent",
            "Clark Labs | An AI agent — Google Chrome"
        ));
        assert!(window_titles_match(
            "Clark Labs | An AI agent in the …A coding agent on your machine.",
            "Clark Labs | An AI agent in the cloud. A coding agent on your machine. - Google Chrome - Stanislav"
        ));
    }

    #[test]
    fn unrelated_or_prefix_only_titles_do_not_match() {
        assert!(!window_titles_match("Example", "Example Domain"));
        assert!(!window_titles_match("Inbox", "Calendar - Google Chrome"));
        assert!(!window_titles_match("A…B", "Anything B"));
    }

    #[test]
    fn empty_cg_title_falls_back_to_frame_matching() {
        assert!(window_titles_match("", "Example Domain - Google Chrome"));
    }

    #[test]
    fn secure_text_detection_covers_role_subrole_and_protected_content() {
        assert!(is_sensitive_text("AXSecureTextField", None, false));
        assert!(is_sensitive_text(
            "AXTextField",
            Some("AXSecureTextField"),
            false
        ));
        assert!(is_sensitive_text("AXGroup", None, true));
        assert!(!is_sensitive_text("AXTextField", None, false));
    }
}
