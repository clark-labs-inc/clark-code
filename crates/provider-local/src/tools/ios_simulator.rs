//! iOS Simulator control tools, backed by `xcrun simctl` for lifecycle/
//! install/launch/screenshot, and Facebook's `idb` for synthetic UI input
//! (`simctl` has no tap/swipe/text/button API at all). macOS-only — there is
//! no iOS Simulator anywhere else.
//!
//! Split into read-only tools (list, screenshot — never gate the user) and
//! mutating tools (everything that changes device/app state — one "always
//! allow" confirm each), per `ToolExecutor::mutating()` being a static
//! per-tool property with no per-argument granularity.
//!
//! `idb`'s exact CLI surface below (`idb ui tap/swipe/text/button`) is
//! implemented against its well-documented, long-stable command shape but
//! was not build-verified against a live `idb` install (not present on the
//! machine this was written on) — verify against `idb ui --help` before
//! relying on it, per the plan's own risk callout.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    arg_i64, arg_i64_opt, arg_str, arg_str_opt, mobile, ToolCtx, ToolExecutor, ToolOutcome,
};

const XCRUN_HINT: &str =
    "`xcrun`/`simctl` not found — install Xcode Command Line Tools (`xcode-select --install`).";
const IDB_HINT: &str =
    "idb is not installed — UI automation (tap/swipe/text/button) on the iOS Simulator needs Facebook's idb: `brew install idb-companion && pip3 install fb-idb`. Device listing, boot/shutdown, install/launch/uninstall, and screenshots don't need it.";
const CMD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct SimctlList {
    devices: BTreeMap<String, Vec<SimDevice>>,
}

#[derive(Deserialize, Clone)]
struct SimDevice {
    udid: String,
    name: String,
    state: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
}

/// Turn a runtime identifier like `com.apple.CoreSimulator.SimRuntime.iOS-26-5`
/// into a friendlier `iOS 26.5` for display.
fn friendly_runtime(runtime_id: &str) -> String {
    let Some(short) = runtime_id.strip_prefix("com.apple.CoreSimulator.SimRuntime.") else {
        return runtime_id.to_string();
    };
    let mut parts = short.splitn(2, '-');
    let platform = parts.next().unwrap_or(short);
    let version = parts.next().unwrap_or("").replace('-', ".");
    if version.is_empty() {
        platform.to_string()
    } else {
        format!("{platform} {version}")
    }
}

async fn list_simulators(ctx: &ToolCtx) -> Result<Vec<(String, SimDevice)>, String> {
    let out = mobile::run_cmd(
        "xcrun",
        &["simctl", "list", "devices", "--json"],
        CMD_TIMEOUT,
        &ctx.cancel,
        XCRUN_HINT,
    )
    .await?;
    let parsed: SimctlList = serde_json::from_str(&out.stdout)
        .map_err(|e| format!("couldn't parse `simctl list` output: {e}"))?;
    Ok(parsed
        .devices
        .into_iter()
        .flat_map(|(runtime, devices)| devices.into_iter().map(move |d| (runtime.clone(), d)))
        .collect())
}

/// Resolve the `udid` argument: use it verbatim if given, otherwise pick the
/// single booted simulator, or error listing candidates if there are zero or
/// several.
async fn resolve_udid(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    if let Some(udid) = arg_str_opt(args, "udid") {
        return Ok(udid);
    }
    let all = list_simulators(ctx).await?;
    let booted: Vec<_> = all.iter().filter(|(_, d)| d.state == "Booted").collect();
    match booted.len() {
        0 => Err(
            "No simulator is booted. Call ios_list_simulators and ios_boot_simulator first."
                .to_string(),
        ),
        1 => Ok(booted[0].1.udid.clone()),
        _ => {
            let names: Vec<_> = booted
                .iter()
                .map(|(_, d)| format!("{} ({})", d.name, d.udid))
                .collect();
            Err(format!(
                "Multiple simulators are booted ({}); pass `udid` to pick one.",
                names.join(", ")
            ))
        }
    }
}

/// `simctl boot`/`shutdown` return a non-zero exit with a specific "current
/// state" message when the device is already in the target state — treat
/// that as a successful no-op rather than an error (an agent calling these
/// iteratively shouldn't be penalized for redundancy).
fn already_in_state(stderr: &str) -> bool {
    stderr.contains("Unable to boot device in current state: Booted")
        || stderr.contains("Unable to shutdown device in current state: Shutdown")
}

fn idb_button(button: &str) -> Result<&'static str, String> {
    Ok(match button {
        "home" => "HOME",
        "lock" => "LOCK",
        "side_button" => "SIDE_BUTTON",
        "siri" => "SIRI",
        "apple_pay" => "APPLE_PAY",
        other => return Err(format!(
            "unknown button `{other}` (expected one of: home, lock, side_button, siri, apple_pay)"
        )),
    })
}

pub struct ListSimulators;

#[async_trait]
impl ToolExecutor for ListSimulators {
    fn name(&self) -> &str {
        "ios_list_simulators"
    }
    fn description(&self) -> &str {
        "List available iOS Simulator devices and their state (Booted/Shutdown)."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let devices = match list_simulators(ctx).await {
            Ok(d) => d,
            Err(e) => return ToolOutcome::error(e),
        };
        if devices.is_empty() {
            return ToolOutcome::ok("No simulators found.".to_string());
        }
        let mut body = String::new();
        for (runtime, d) in &devices {
            if !d.is_available {
                continue;
            }
            body.push_str(&format!(
                "{} — {} — {} — {}\n",
                d.name,
                friendly_runtime(runtime),
                d.state,
                d.udid
            ));
        }
        ToolOutcome::ok(body.trim_end().to_string())
    }
}

pub struct BootSimulator;

#[async_trait]
impl ToolExecutor for BootSimulator {
    fn name(&self) -> &str {
        "ios_boot_simulator"
    }
    fn description(&self) -> &str {
        "Boot an iOS Simulator device by udid. Use ios_list_simulators first to find a udid."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "The simulator's udid, as reported by ios_list_simulators."}
            },
            "required": ["udid"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let udid = match arg_str(&args, "udid") {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "xcrun",
            &["simctl", "boot", &udid],
            CMD_TIMEOUT,
            &ctx.cancel,
            XCRUN_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => ToolOutcome::ok(format!("Booted {udid}.")),
            Ok(out) if already_in_state(&out.stderr) => {
                ToolOutcome::ok(format!("{udid} was already booted."))
            }
            Ok(out) => ToolOutcome::error(format!("boot failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct ShutdownSimulator;

#[async_trait]
impl ToolExecutor for ShutdownSimulator {
    fn name(&self) -> &str {
        "ios_shutdown_simulator"
    }
    fn description(&self) -> &str {
        "Shut down an iOS Simulator device by udid."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."}
            }
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "xcrun",
            &["simctl", "shutdown", &udid],
            CMD_TIMEOUT,
            &ctx.cancel,
            XCRUN_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => ToolOutcome::ok(format!("Shut down {udid}.")),
            Ok(out) if already_in_state(&out.stderr) => {
                ToolOutcome::ok(format!("{udid} was already shut down."))
            }
            Ok(out) => ToolOutcome::error(format!("shutdown failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct InstallApp;

#[async_trait]
impl ToolExecutor for InstallApp {
    fn name(&self) -> &str {
        "ios_install_app"
    }
    fn description(&self) -> &str {
        "Install a built .app bundle onto a booted iOS Simulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "app_path": {"type": "string", "description": "Path to the built .app bundle."}
            },
            "required": ["app_path"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let app_path = match arg_str(&args, "app_path") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "xcrun",
            &["simctl", "install", &udid, &app_path],
            Duration::from_secs(60),
            &ctx.cancel,
            XCRUN_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Installed {app_path} on {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("install failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct UninstallApp;

#[async_trait]
impl ToolExecutor for UninstallApp {
    fn name(&self) -> &str {
        "ios_uninstall_app"
    }
    fn description(&self) -> &str {
        "Uninstall an app by bundle id from a booted iOS Simulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "bundle_id": {"type": "string", "description": "Bundle identifier, e.g. `com.example.app`."}
            },
            "required": ["bundle_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let bundle_id = match arg_str(&args, "bundle_id") {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(e),
        };
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "xcrun",
            &["simctl", "uninstall", &udid, &bundle_id],
            CMD_TIMEOUT,
            &ctx.cancel,
            XCRUN_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Uninstalled {bundle_id} from {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("uninstall failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct LaunchApp;

#[async_trait]
impl ToolExecutor for LaunchApp {
    fn name(&self) -> &str {
        "ios_launch_app"
    }
    fn description(&self) -> &str {
        "Launch an installed app by bundle id on a booted iOS Simulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "bundle_id": {"type": "string", "description": "Bundle identifier, e.g. `com.example.app`."},
                "args": {"type": "array", "items": {"type": "string"}, "description": "Optional launch arguments passed through to the app."}
            },
            "required": ["bundle_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let bundle_id = match arg_str(&args, "bundle_id") {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(e),
        };
        let extra_args: Vec<String> = args
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        let mut cmd_args: Vec<&str> = vec!["simctl", "launch", &udid, &bundle_id];
        cmd_args.extend(extra_args.iter().map(String::as_str));
        match mobile::run_cmd("xcrun", &cmd_args, CMD_TIMEOUT, &ctx.cancel, XCRUN_HINT).await {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Launched {bundle_id} on {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("launch failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct Screenshot;

#[async_trait]
impl ToolExecutor for Screenshot {
    fn name(&self) -> &str {
        "ios_screenshot"
    }
    fn description(&self) -> &str {
        "Capture a screenshot of a booted iOS Simulator. Returns the image so you can visually inspect the current screen."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."}
            }
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        let dir: PathBuf = mobile::screenshot_dir(ctx);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return ToolOutcome::error(format!("couldn't create {}: {e}", dir.display()));
        }
        let path = dir.join(format!("ios-{udid}-{}.png", mobile::timestamp_slug()));
        let path_str = path.display().to_string();
        match mobile::run_cmd(
            "xcrun",
            &["simctl", "io", &udid, "screenshot", &path_str],
            CMD_TIMEOUT,
            &ctx.cancel,
            XCRUN_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => {}
            Ok(out) => {
                return ToolOutcome::error(format!("screenshot failed: {}", out.stderr.trim()))
            }
            Err(e) => return ToolOutcome::error(e),
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(format!("couldn't read {}: {e}", path.display())),
        };
        mobile::prune_screenshots(&dir);
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        ToolOutcome::ok(format!("Captured a screenshot of {udid}."))
            .with_image(
                "image/png",
                data_base64,
                Some(format!("iOS Simulator screenshot ({udid})")),
            )
            .with_location(path_str, None)
    }
}

pub struct Tap;

#[async_trait]
impl ToolExecutor for Tap {
    fn name(&self) -> &str {
        "ios_tap"
    }
    fn description(&self) -> &str {
        "Simulate a tap at (x, y) point coordinates on a booted iOS Simulator. Requires `idb` (Facebook's iOS automation tool) to be installed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "x": {"type": "integer"},
                "y": {"type": "integer"}
            },
            "required": ["x", "y"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let (x, y) = match (arg_i64(&args, "x"), arg_i64(&args, "y")) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(e), _) | (_, Err(e)) => return ToolOutcome::error(e),
        };
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        let (xs, ys) = (x.to_string(), y.to_string());
        match mobile::run_cmd(
            "idb",
            &["ui", "tap", &xs, &ys, "--udid", &udid],
            CMD_TIMEOUT,
            &ctx.cancel,
            IDB_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Tapped ({x}, {y}) on {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("tap failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct Swipe;

#[async_trait]
impl ToolExecutor for Swipe {
    fn name(&self) -> &str {
        "ios_swipe"
    }
    fn description(&self) -> &str {
        "Simulate a swipe from (x1, y1) to (x2, y2) point coordinates on a booted iOS Simulator. Requires `idb` to be installed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "x1": {"type": "integer"},
                "y1": {"type": "integer"},
                "x2": {"type": "integer"},
                "y2": {"type": "integer"},
                "duration_ms": {"type": "integer", "description": "Swipe duration in milliseconds. Omit for idb's default."}
            },
            "required": ["x1", "y1", "x2", "y2"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let (x1, y1, x2, y2) = match (
            arg_i64(&args, "x1"),
            arg_i64(&args, "y1"),
            arg_i64(&args, "x2"),
            arg_i64(&args, "y2"),
        ) {
            (Ok(x1), Ok(y1), Ok(x2), Ok(y2)) => (x1, y1, x2, y2),
            (r1, r2, r3, r4) => {
                return ToolOutcome::error(
                    [r1.err(), r2.err(), r3.err(), r4.err()]
                        .into_iter()
                        .flatten()
                        .next()
                        .unwrap_or_default(),
                )
            }
        };
        let duration_ms = arg_i64_opt(&args, "duration_ms");
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        let (x1s, y1s, x2s, y2s) = (
            x1.to_string(),
            y1.to_string(),
            x2.to_string(),
            y2.to_string(),
        );
        let mut cmd_args = vec![
            "ui",
            "swipe",
            x1s.as_str(),
            y1s.as_str(),
            x2s.as_str(),
            y2s.as_str(),
        ];
        // idb's --duration takes seconds, not milliseconds.
        let duration_secs_str = duration_ms.map(|ms| format!("{:.3}", ms as f64 / 1000.0));
        if let Some(d) = &duration_secs_str {
            cmd_args.push("--duration");
            cmd_args.push(d);
        }
        cmd_args.push("--udid");
        cmd_args.push(&udid);
        match mobile::run_cmd("idb", &cmd_args, CMD_TIMEOUT, &ctx.cancel, IDB_HINT).await {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Swiped ({x1},{y1}) -> ({x2},{y2}) on {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("swipe failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct TypeText;

#[async_trait]
impl ToolExecutor for TypeText {
    fn name(&self) -> &str {
        "ios_type_text"
    }
    fn description(&self) -> &str {
        "Type text into the currently focused field on a booted iOS Simulator. Requires `idb` to be installed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "text": {"type": "string"}
            },
            "required": ["text"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let text = match arg_str(&args, "text") {
            Ok(t) => t,
            Err(e) => return ToolOutcome::error(e),
        };
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "idb",
            &["ui", "text", &text, "--udid", &udid],
            CMD_TIMEOUT,
            &ctx.cancel,
            IDB_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => ToolOutcome::ok(format!("Typed text on {udid}.")),
            Ok(out) => ToolOutcome::error(format!("type failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct PressButton;

#[async_trait]
impl ToolExecutor for PressButton {
    fn name(&self) -> &str {
        "ios_press_button"
    }
    fn description(&self) -> &str {
        "Press a hardware button on a booted iOS Simulator: home, lock, side_button, siri, apple_pay. Requires `idb` to be installed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "udid": {"type": "string", "description": "Simulator udid. Omit if only one simulator is booted."},
                "button": {"type": "string", "enum": ["home", "lock", "side_button", "siri", "apple_pay"]}
            },
            "required": ["button"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let button = match arg_str(&args, "button") {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(e),
        };
        let idb_button = match idb_button(&button) {
            Ok(b) => b,
            Err(e) => return ToolOutcome::error(e),
        };
        let udid = match resolve_udid(&args, ctx).await {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "idb",
            &["ui", "button", idb_button, "--udid", &udid],
            CMD_TIMEOUT,
            &ctx.cancel,
            IDB_HINT,
        )
        .await
        {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Pressed {button} on {udid}."))
            }
            Ok(out) => ToolOutcome::error(format!("button press failed: {}", out.stderr.trim())),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_runtime_formats_known_shape() {
        assert_eq!(
            friendly_runtime("com.apple.CoreSimulator.SimRuntime.iOS-26-5"),
            "iOS 26.5"
        );
        assert_eq!(
            friendly_runtime("unrecognized-format"),
            "unrecognized-format"
        );
    }

    #[test]
    fn parses_real_simctl_list_json_shape() {
        // Captured live from `xcrun simctl list devices --json` (trimmed).
        let json = r#"{
            "devices" : {
              "com.apple.CoreSimulator.SimRuntime.iOS-26-5" : [
                {
                  "dataPath" : "/tmp",
                  "dataPathSize" : 1,
                  "logPath" : "/tmp",
                  "udid" : "83EAE099-5C74-465D-9FAE-CC86D32D7A20",
                  "isAvailable" : true,
                  "deviceTypeIdentifier" : "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro",
                  "state" : "Shutdown",
                  "name" : "iPhone 17 Pro"
                }
              ]
            }
        }"#;
        let parsed: SimctlList = serde_json::from_str(json).unwrap();
        let devices = parsed
            .devices
            .get("com.apple.CoreSimulator.SimRuntime.iOS-26-5")
            .unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].udid, "83EAE099-5C74-465D-9FAE-CC86D32D7A20");
        assert_eq!(devices[0].name, "iPhone 17 Pro");
        assert_eq!(devices[0].state, "Shutdown");
        assert!(devices[0].is_available);
    }

    #[test]
    fn already_in_state_matches_real_simctl_error_text() {
        // Captured live from `xcrun simctl boot`/`shutdown` on an
        // already-booted/shutdown device.
        assert!(already_in_state(
            "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405):\nUnable to boot device in current state: Booted\n"
        ));
        assert!(already_in_state(
            "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405):\nUnable to shutdown device in current state: Shutdown\n"
        ));
        assert!(!already_in_state("some other simctl error"));
    }

    #[test]
    fn idb_button_maps_known_buttons() {
        assert_eq!(idb_button("home").unwrap(), "HOME");
        assert!(idb_button("bogus").is_err());
    }
}
