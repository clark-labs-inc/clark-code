use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use base64::Engine as _;
use computer_use::{PermissionRequest, WindowFilter};
use serde_json::{json, Value};

mod presentation;

use presentation::{format_diff, format_element, format_settlement, granted};

use super::{
    app_scope, backend_call, bool_arg, bundle_preflight, optional_string, required_string, target,
    target_preflight, ComputerBackend,
};
use crate::tools::{PermissionScope, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass};

pub struct Permissions {
    backend: Arc<dyn ComputerBackend>,
}

impl Permissions {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ToolExecutor for Permissions {
    fn name(&self) -> &str {
        "computer_permissions"
    }

    fn description(&self) -> &str {
        "Check whether local agent currently has the macOS Accessibility and Screen Recording permissions required for computer use. This does not show system prompts."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let backend = self.backend.clone();
        match backend_call(ctx, move || backend.permissions()).await {
            Ok(status) => ToolOutcome::ok(format!(
                "Accessibility: {}\nScreen Recording: {}\nRelaunch needed after Screen Recording grant: {}",
                granted(status.accessibility),
                granted(status.screen_recording),
                status.screen_recording_restart_required,
            ))
            .with_details(json!(status)),
            Err(error) => ToolOutcome::error(error),
        }
    }
}

pub struct RequestPermissions {
    backend: Arc<dyn ComputerBackend>,
}

impl RequestPermissions {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ToolExecutor for RequestPermissions {
    fn name(&self) -> &str {
        "computer_request_permissions"
    }

    fn description(&self) -> &str {
        "Ask macOS for Accessibility and/or Screen Recording access. Use only after computer_permissions reports a missing permission. The user must approve the macOS system prompt; Screen Recording may require relaunching local agent."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "accessibility": {
                    "type": "boolean",
                    "description": "Request Accessibility access (default true)."
                },
                "screen_recording": {
                    "type": "boolean",
                    "description": "Request Screen Recording access (default true)."
                }
            }
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn permission_scope(&self, _args: &Value) -> Option<PermissionScope> {
        Some(PermissionScope {
            key: "computer:macos-permission-request:one-off".to_string(),
            title: Some("Open macOS privacy permission prompts?".to_string()),
            always_label: None,
            reason: Some(
                "changes which apps macOS permits local agent to observe and control".to_string(),
            ),
            risk: Some("confirm".to_string()),
            remember: false,
            preapproved: false,
        })
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        Some(format!(
            "Request Accessibility: {}\nRequest Screen Recording: {}",
            args.get("accessibility")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            args.get("screen_recording")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let accessibility = match bool_arg(&args, "accessibility", true) {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let screen_recording = match bool_arg(&args, "screen_recording", true) {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        if !accessibility && !screen_recording {
            return ToolOutcome::error(
                "at least one of `accessibility` or `screen_recording` must be true",
            );
        }
        let backend = self.backend.clone();
        match backend_call(ctx, move || {
            backend.request_permissions(PermissionRequest {
                accessibility,
                screen_recording,
            })
        })
        .await
        {
            Ok(status) => ToolOutcome::ok(format!(
                "Accessibility: {}\nScreen Recording: {}\nRelaunch needed after Screen Recording grant: {}",
                granted(status.accessibility),
                granted(status.screen_recording),
                status.screen_recording_restart_required,
            ))
            .with_details(json!(status)),
            Err(error) => ToolOutcome::error(error),
        }
    }
}

pub struct ListWindows {
    backend: Arc<dyn ComputerBackend>,
}

impl ListWindows {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ToolExecutor for ListWindows {
    fn name(&self) -> &str {
        "computer_list_windows"
    }

    fn description(&self) -> &str {
        "List visible application windows on this Mac and return stable pid + window_id + app_bundle_id targets. Filter by app bundle id or title when possible. Window titles are private desktop data, so this requires user approval."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "app_bundle_id": {
                    "type": "string",
                    "description": "Optional exact bundle id, such as com.apple.Safari."
                },
                "title_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive window-title substring."
                }
            }
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        app_scope(args, true).or_else(|| {
            Some(PermissionScope {
                key: "computer:window-discovery".to_string(),
                title: Some("Allow Clark Code to see open app windows?".to_string()),
                always_label: Some("Always allow window discovery".to_string()),
                reason: Some("reveals app names and window titles on this Mac".to_string()),
                // Full access must not silently reveal which private apps and
                // documents are open. The user may remember this grant after
                // explicitly reviewing it once.
                risk: Some("confirm".to_string()),
                remember: true,
                preapproved: false,
            })
        })
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        if args.get("app_bundle_id").is_some() {
            bundle_preflight(args)?;
        }
        Ok(())
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let filter = WindowFilter {
            bundle_id: optional_string(&args, "app_bundle_id"),
            title_contains: optional_string(&args, "title_contains"),
        };
        let backend = self.backend.clone();
        match backend_call(ctx, move || backend.list_windows(filter)).await {
            Ok(windows) if windows.is_empty() => ToolOutcome::ok(
                "No matching windows. If the app is closed, use computer_open_app with its bundle id.",
            )
            .with_details(json!([])),
            Ok(windows) => {
                let lines = windows
                    .iter()
                    .map(|window| {
                        format!(
                            "- {} — {:?}\n  app_bundle_id={} pid={} window_id={} frame=({:.0},{:.0} {:.0}x{:.0})",
                            window.app_name,
                            window.title,
                            window.target.bundle_id,
                            window.target.pid,
                            window.target.window_id,
                            window.frame.x,
                            window.frame.y,
                            window.frame.width,
                            window.frame.height,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolOutcome::ok(lines).with_details(json!(windows))
            }
            Err(error) => ToolOutcome::error(error),
        }
    }
}

pub struct OpenApplication {
    backend: Arc<dyn ComputerBackend>,
}

impl OpenApplication {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ToolExecutor for OpenApplication {
    fn name(&self) -> &str {
        "computer_open_app"
    }

    fn description(&self) -> &str {
        "Launch or activate a macOS application by exact bundle id. Then call computer_list_windows to obtain its concrete window identity."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "app_bundle_id": {
                    "type": "string",
                    "description": "Exact application bundle id, such as com.apple.Safari."
                }
            },
            "required": ["app_bundle_id"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        app_scope(args, true)
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        bundle_preflight(args)
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        args.get("app_bundle_id")
            .and_then(Value::as_str)
            .map(|bundle_id| format!("Open application: {bundle_id}"))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let bundle_id = match required_string(&args, "app_bundle_id") {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let backend = self.backend.clone();
        let launched = bundle_id.clone();
        match backend_call(ctx, move || backend.launch_application(&launched)).await {
            Ok(()) => ToolOutcome::ok(format!(
                "Opened {bundle_id}. Call computer_list_windows with this app_bundle_id next."
            )),
            Err(error) => ToolOutcome::error(error),
        }
    }
}

pub struct GetState {
    backend: Arc<dyn ComputerBackend>,
}

impl GetState {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ToolExecutor for GetState {
    fn name(&self) -> &str {
        "computer_get_state"
    }

    fn description(&self) -> &str {
        "Observe one exact macOS window. Returns a current screenshot, bounded settling result, and Accessibility tree plus a diff from the prior observation. Element ids and bounds are valid only for the next action; observe again after every computer action."
    }

    fn parameters(&self) -> Value {
        target_schema()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ViewImage
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        app_scope(args, true)
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        target_preflight(args)
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let target = match target(&args) {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let backend = self.backend.clone();
        match backend_call(ctx, move || backend.observe(&target)).await {
            Ok(observation) => {
                let elements = observation
                    .elements
                    .iter()
                    .map(format_element)
                    .collect::<Vec<_>>()
                    .join("\n");
                let settlement = format_settlement(&observation.settlement);
                let diff = format_diff(observation.accessibility_diff.as_ref());
                let content = format!(
                    "Window: {} — {:?}\nTarget: app_bundle_id={} pid={} window_id={}\nObservation ID: {}\nScreenshot: {}x{} pixels\n{}\n{}\nAccessibility tree{}:\n{}",
                    observation.window.app_name,
                    observation.window.title,
                    observation.window.target.bundle_id,
                    observation.window.target.pid,
                    observation.window.target.window_id,
                    observation.observation_id,
                    observation.screenshot.width,
                    observation.screenshot.height,
                    settlement,
                    diff,
                    if observation.accessibility_truncated {
                        " (truncated)"
                    } else {
                        ""
                    },
                    if elements.is_empty() {
                        "(no accessible elements)"
                    } else {
                        &elements
                    },
                );
                let image =
                    base64::engine::general_purpose::STANDARD.encode(&observation.screenshot.png);
                ToolOutcome::ok(content)
                    .with_image(
                        "image/png",
                        image,
                        Some(format!(
                            "{} — {}",
                            observation.window.app_name, observation.window.title
                        )),
                    )
                    .with_details(json!({
                        "window": observation.window,
                        "observation_id": observation.observation_id,
                        "screenshot": {
                            "width": observation.screenshot.width,
                            "height": observation.screenshot.height,
                        },
                        "elements": observation.elements,
                        "accessibility_truncated": observation.accessibility_truncated,
                        "observed_at_ms": observation.observed_at_ms,
                        "accessibility_diff": observation.accessibility_diff,
                        "settlement": observation.settlement,
                    }))
            }
            Err(error) => ToolOutcome::error(error),
        }
    }
}

fn target_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_bundle_id": {
                "type": "string",
                "description": "Exact bundle id returned by computer_list_windows."
            },
            "pid": {
                "type": "integer",
                "minimum": 1,
                "description": "Process id returned by computer_list_windows."
            },
            "window_id": {
                "type": "integer",
                "minimum": 1,
                "description": "Window id returned by computer_list_windows."
            }
        },
        "required": ["app_bundle_id", "pid", "window_id"]
    })
}

#[cfg(test)]
mod tests;
