//! Opt-in tools for observing and operating ordinary macOS application
//! windows. The provider layer owns schemas, permissions, and model-facing
//! output; `computer-use` owns native APIs and the observe-before-act state
//! machine.

mod actions;
mod observe;
mod schemas;

use std::sync::Arc;

use serde_json::Value;

pub use computer_use::ComputerBackend;
use computer_use::{ActionIntent, ActionRisk, Key, Modifier, WindowTarget};

use super::{PermissionScope, ToolCtx, ToolExecutor};

const MAX_TEXT_INPUT_CHARS: usize = 2_000;
const MAX_INTENT_REASON_CHARS: usize = 500;

pub fn executors(backend: Arc<dyn ComputerBackend>) -> Vec<Arc<dyn ToolExecutor>> {
    let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
        Arc::new(observe::Permissions::new(backend.clone())),
        Arc::new(observe::RequestPermissions::new(backend.clone())),
        Arc::new(observe::ListWindows::new(backend.clone())),
        Arc::new(observe::OpenApplication::new(backend.clone())),
        Arc::new(observe::GetState::new(backend.clone())),
    ];
    tools.extend(actions::executors(backend));
    tools
}

pub(super) async fn backend_call<T, F>(ctx: &ToolCtx, action: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, computer_use::ComputerUseError> + Send + 'static,
{
    if ctx.cancel.is_cancelled() {
        return Err("cancelled".to_string());
    }
    let value = tokio::task::spawn_blocking(action)
        .await
        .map_err(|error| format!("computer-use worker failed: {error}"))?
        .map_err(|error| error.to_string())?;
    if ctx.cancel.is_cancelled() {
        return Err("cancelled".to_string());
    }
    Ok(value)
}

pub(super) fn target(args: &Value) -> Result<WindowTarget, String> {
    let bundle_id = required_string(args, "app_bundle_id")?;
    if bundle_id.len() > 255 {
        return Err("`app_bundle_id` is too long".to_string());
    }
    let pid = required_i64(args, "pid")?;
    let window_id = required_i64(args, "window_id")?;
    let pid = i32::try_from(pid).map_err(|_| "`pid` is outside the i32 range".to_string())?;
    let window_id = u32::try_from(window_id)
        .map_err(|_| "`window_id` must be between 1 and 4294967295".to_string())?;
    if pid <= 0 {
        return Err("`pid` must be positive".to_string());
    }
    if window_id == 0 {
        return Err("`window_id` must be positive".to_string());
    }
    Ok(WindowTarget {
        pid,
        window_id,
        bundle_id,
    })
}

pub(super) fn required_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("`{key}` must be a non-empty string"))
}

pub(super) fn required_observation_id(args: &Value) -> Result<String, String> {
    let value = required_string(args, "observation_id")?;
    if value.len() > 128 {
        return Err("`observation_id` is too long".to_string());
    }
    Ok(value)
}

pub(super) fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn required_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("`{key}` must be an integer"))
}

pub(super) fn optional_f64(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a finite number")),
    }
}

pub(super) fn bool_arg(args: &Value, key: &str, default: bool) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("`{key}` must be a boolean")),
    }
}

pub(super) fn parse_key(value: &str) -> Result<Key, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let key = match normalized.as_str() {
        "return" | "enter" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "arrow_up" | "up" => Key::ArrowUp,
        "arrow_down" | "down" => Key::ArrowDown,
        "arrow_left" | "left" => Key::ArrowLeft,
        "arrow_right" | "right" => Key::ArrowRight,
        "home" => Key::Home,
        "end" => Key::End,
        "page_up" => Key::PageUp,
        "page_down" => Key::PageDown,
        _ => {
            let mut characters = value.chars();
            let character = characters
                .next()
                .filter(|_| characters.next().is_none())
                .ok_or_else(|| format!("unknown key `{value}`"))?;
            Key::Character(character)
        }
    };
    Ok(key)
}

pub(super) fn parse_modifiers(args: &Value) -> Result<Vec<Modifier>, String> {
    let Some(value) = args.get("modifiers") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "`modifiers` must be an array".to_string())?;
    if values.len() > 4 {
        return Err("`modifiers` may contain at most four values".to_string());
    }
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let name = value
            .as_str()
            .ok_or_else(|| "every modifier must be a string".to_string())?;
        let modifier = match name {
            "command" => Modifier::Command,
            "control" => Modifier::Control,
            "option" => Modifier::Option,
            "shift" => Modifier::Shift,
            other => return Err(format!("unknown modifier `{other}`")),
        };
        if parsed.contains(&modifier) {
            return Err(format!("duplicate modifier `{name}`"));
        }
        parsed.push(modifier);
    }
    Ok(parsed)
}

pub(super) fn app_scope(args: &Value, remember: bool) -> Option<PermissionScope> {
    let bundle_id = args
        .get("app_bundle_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 255)?;
    Some(PermissionScope {
        key: format!("computer:{bundle_id}"),
        title: Some(format!("Allow Clark Code to see and control {bundle_id}?")),
        always_label: Some(format!("Always allow {bundle_id}")),
        reason: Some(
            "observes or controls this app through macOS Accessibility and Screen Recording"
                .to_string(),
        ),
        // Computer use is materially broader than project access. Require one
        // human decision for each app even under Full access; an explicit
        // "Always allow" answer then covers routine actions for this app.
        risk: Some("confirm".to_string()),
        remember,
        preapproved: false,
    })
}

pub(super) fn required_intent(args: &Value) -> Result<ActionIntent, String> {
    let risk = required_string(args, "risk")?.parse::<ActionRisk>()?;
    let reason = required_string(args, "reason")?;
    if reason.chars().count() > MAX_INTENT_REASON_CHARS {
        return Err(format!(
            "`reason` exceeds the {MAX_INTENT_REASON_CHARS}-character limit"
        ));
    }
    Ok(ActionIntent { risk, reason })
}

pub(super) fn action_preflight(args: &Value) -> Result<(), String> {
    let target = target(args)?;
    computer_use::ensure_bundle_allowed(&target.bundle_id).map_err(|error| error.to_string())?;
    required_intent(args)?;
    Ok(())
}

pub(super) fn target_preflight(args: &Value) -> Result<(), String> {
    let target = target(args)?;
    computer_use::ensure_bundle_allowed(&target.bundle_id).map_err(|error| error.to_string())
}

pub(super) fn bundle_preflight(args: &Value) -> Result<(), String> {
    let bundle_id = required_string(args, "app_bundle_id")?;
    computer_use::ensure_bundle_allowed(&bundle_id).map_err(|error| error.to_string())
}

pub(super) fn validate_text_length(text: &str) -> Result<(), String> {
    if text.chars().count() > MAX_TEXT_INPUT_CHARS {
        return Err(format!(
            "`text` exceeds the {MAX_TEXT_INPUT_CHARS}-character computer input limit"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn target_requires_all_three_identity_fields() {
        let parsed = target(&json!({
            "app_bundle_id": "com.apple.Safari",
            "pid": 123,
            "window_id": 456
        }))
        .unwrap();
        assert_eq!(parsed.bundle_id, "com.apple.Safari");
        assert_eq!(parsed.pid, 123);
        assert_eq!(parsed.window_id, 456);
        assert!(target(&json!({"pid": 123, "window_id": 456})).is_err());
        assert!(target(&json!({
            "app_bundle_id": "com.apple.Safari",
            "pid": 0,
            "window_id": 456
        }))
        .is_err());
        assert!(target(&json!({
            "app_bundle_id": "com.apple.Safari",
            "pid": 123,
            "window_id": 0
        }))
        .is_err());
    }

    #[test]
    fn key_parser_accepts_named_and_single_character_keys() {
        assert_eq!(parse_key("return").unwrap(), Key::Return);
        assert_eq!(parse_key("k").unwrap(), Key::Character('k'));
        assert!(parse_key("not-a-key").is_err());
        assert!(parse_modifiers(&json!({
            "modifiers": ["command", "command"]
        }))
        .is_err());
    }

    #[test]
    fn app_scope_is_bound_to_one_bundle() {
        let scope = app_scope(&json!({"app_bundle_id": "com.apple.Safari"}), true).unwrap();
        assert_eq!(scope.key, "computer:com.apple.Safari");
        assert!(scope.remember);
        assert_eq!(scope.risk.as_deref(), Some("confirm"));
        assert!(app_scope(&json!({"app_bundle_id": "x".repeat(256)}), true).is_none());
    }

    #[test]
    fn forbidden_bundle_fails_permission_preflight() {
        let error = bundle_preflight(&json!({"app_bundle_id": "com.apple.Terminal"})).unwrap_err();
        assert!(error.contains("forbids target"));
    }
}
