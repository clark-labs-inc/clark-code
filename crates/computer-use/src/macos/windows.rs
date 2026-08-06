use std::process::Command;
use std::time::{Duration, Instant};

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer,
    kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName,
    kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

use crate::{ComputerUseError, Rect, WindowFilter, WindowInfo, WindowTarget};

const FRONTMOST_TIMEOUT: Duration = Duration::from_secs(2);
const FRONTMOST_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub fn list_windows(filter: &WindowFilter) -> Result<Vec<WindowInfo>, ComputerUseError> {
    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let list = copy_window_info(options, kCGNullWindowID).ok_or_else(|| {
        ComputerUseError::Os("CGWindowListCopyWindowInfo returned null".to_string())
    })?;
    let mut windows = Vec::new();
    for index in 0..list.len() {
        let Some(raw) = list.get(index) else {
            continue;
        };
        let dictionary: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*raw as _) };
        let Some(window) = window_from_dictionary(&dictionary) else {
            continue;
        };
        if window.layer != 0
            || window.frame.width < 80.0
            || window.frame.height < 60.0
            || !matches_filter(&window, filter)
        {
            continue;
        }
        windows.push(window);
    }
    windows.sort_by(|left, right| {
        left.app_name
            .to_ascii_lowercase()
            .cmp(&right.app_name.to_ascii_lowercase())
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(windows)
}

pub fn resolve_window(target: &WindowTarget) -> Result<WindowInfo, ComputerUseError> {
    list_windows(&WindowFilter {
        bundle_id: Some(target.bundle_id.clone()),
        title_contains: None,
    })?
    .into_iter()
    .find(|window| {
        window.target.pid == target.pid
            && window.target.window_id == target.window_id
            && window.target.bundle_id == target.bundle_id
    })
    .ok_or_else(|| {
        ComputerUseError::TargetChanged(format!(
            "{}:{} is no longer an on-screen window owned by {}",
            target.pid, target.window_id, target.bundle_id
        ))
    })
}

pub fn launch_application(bundle_id: &str) -> Result<(), ComputerUseError> {
    let output = Command::new("/usr/bin/open")
        .args(["-b", bundle_id])
        .output()
        .map_err(|error| ComputerUseError::Os(format!("could not run open: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ComputerUseError::Os(format!(
            "could not launch {bundle_id}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub fn focus_window(window: &WindowInfo) -> Result<(), ComputerUseError> {
    let app: Option<Retained<NSRunningApplication>> =
        NSRunningApplication::runningApplicationWithProcessIdentifier(window.target.pid);
    let app =
        app.ok_or_else(|| ComputerUseError::WindowNotFound(format!("pid {}", window.target.pid)))?;
    #[allow(deprecated)]
    let options = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
    let activation_requested = app.activateWithOptions(options);
    let ax_focus_error = super::accessibility::focus_application(window).err();
    super::accessibility::raise_window(window)?;

    let started = Instant::now();
    loop {
        let frontmost = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or_else(|| ComputerUseError::Os("macOS reported no frontmost app".to_string()))?;
        if frontmost.processIdentifier() == window.target.pid {
            return Ok(());
        }
        if started.elapsed() >= FRONTMOST_TIMEOUT {
            break;
        }
        std::thread::sleep(FRONTMOST_POLL_INTERVAL);
    }

    if !activation_requested {
        let accessibility = ax_focus_error
            .map(|error| format!("; Accessibility activation also failed: {error}"))
            .unwrap_or_default();
        Err(ComputerUseError::Os(format!(
            "macOS refused to activate {}{accessibility}",
            window.app_name
        )))
    } else {
        Err(ComputerUseError::TargetChanged(format!(
            "{} did not become the frontmost app within {} ms",
            window.app_name,
            FRONTMOST_TIMEOUT.as_millis()
        )))
    }
}

fn matches_filter(window: &WindowInfo, filter: &WindowFilter) -> bool {
    if filter
        .bundle_id
        .as_deref()
        .is_some_and(|bundle| bundle != window.target.bundle_id)
    {
        return false;
    }
    if let Some(needle) = filter.title_contains.as_deref() {
        let needle = needle.to_ascii_lowercase();
        if !window.title.to_ascii_lowercase().contains(&needle)
            && !window.app_name.to_ascii_lowercase().contains(&needle)
        {
            return false;
        }
    }
    true
}

fn window_from_dictionary(dictionary: &CFDictionary<CFString, CFType>) -> Option<WindowInfo> {
    let pid = number(dictionary, unsafe { kCGWindowOwnerPID })? as i32;
    let window_id = number(dictionary, unsafe { kCGWindowNumber })? as u32;
    let layer = number(dictionary, unsafe { kCGWindowLayer }).unwrap_or(0.0) as i32;
    let title = string(dictionary, unsafe { kCGWindowName }).unwrap_or_default();
    let owner_name = string(dictionary, unsafe { kCGWindowOwnerName }).unwrap_or_default();
    let frame = bounds(dictionary)?;
    let on_screen = boolean(dictionary, unsafe { kCGWindowIsOnscreen }).unwrap_or(true);
    let app: Option<Retained<NSRunningApplication>> =
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    let bundle_id = app
        .as_ref()
        .and_then(|application| application.bundleIdentifier())
        .map(|bundle| bundle.to_string())?;
    let app_name = app
        .and_then(|application| application.localizedName())
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or(owner_name);
    Some(WindowInfo {
        target: WindowTarget {
            pid,
            window_id,
            bundle_id,
        },
        app_name,
        title,
        frame,
        layer,
        on_screen,
    })
}

fn bounds(dictionary: &CFDictionary<CFString, CFType>) -> Option<Rect> {
    let key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let value = dictionary.find(&key)?;
    let untyped = value.downcast::<CFDictionary>()?;
    let values: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule(untyped.as_concrete_TypeRef()) };
    Some(Rect {
        x: number_by_name(&values, "X")?,
        y: number_by_name(&values, "Y")?,
        width: number_by_name(&values, "Width")?,
        height: number_by_name(&values, "Height")?,
    })
}

fn number(dictionary: &CFDictionary<CFString, CFType>, key_ref: CFStringRef) -> Option<f64> {
    let key = unsafe { CFString::wrap_under_get_rule(key_ref) };
    let number = dictionary.find(&key)?.downcast::<CFNumber>()?;
    number
        .to_f64()
        .or_else(|| number.to_i64().map(|value| value as f64))
}

fn number_by_name(dictionary: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
    let number = dictionary
        .find(CFString::new(key))?
        .downcast::<CFNumber>()?;
    number
        .to_f64()
        .or_else(|| number.to_i64().map(|value| value as f64))
}

fn string(dictionary: &CFDictionary<CFString, CFType>, key_ref: CFStringRef) -> Option<String> {
    let key = unsafe { CFString::wrap_under_get_rule(key_ref) };
    dictionary
        .find(&key)?
        .downcast::<CFString>()
        .map(|value| value.to_string())
}

fn boolean(dictionary: &CFDictionary<CFString, CFType>, key_ref: CFStringRef) -> Option<bool> {
    let key = unsafe { CFString::wrap_under_get_rule(key_ref) };
    dictionary
        .find(&key)?
        .downcast::<CFBoolean>()
        .map(Into::into)
}
