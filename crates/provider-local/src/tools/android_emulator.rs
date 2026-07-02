//! Android Emulator control tools, backed by `adb` and (for booting an AVD)
//! the `emulator` binary from the Android SDK. Cross-platform — `adb` runs on
//! macOS, Linux, and Windows — unlike the iOS Simulator tools, which are
//! macOS-only.
//!
//! Split into read-only tools (list, screenshot — never gate the user) and
//! mutating tools (everything that changes device state — one "always
//! allow" confirm each), per `ToolExecutor::mutating()` being a static
//! per-tool property with no per-argument granularity.

use std::path::PathBuf;
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};

use super::{
    arg_i64, arg_i64_opt, arg_str, arg_str_opt, mobile, ToolCtx, ToolExecutor, ToolOutcome,
};

const ADB_HINT: &str =
    "adb not found — install Android SDK Platform Tools and ensure `adb` is on PATH (or set ANDROID_HOME).";
const EMULATOR_HINT: &str =
    "`emulator` not found — install the Android SDK Emulator package and ensure it's on PATH (or set ANDROID_HOME).";
const CMD_TIMEOUT: Duration = Duration::from_secs(15);
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);
const BOOT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// A running device/emulator as reported by `adb devices -l`.
struct Device {
    serial: String,
    state: String,
}

async fn list_adb_devices(
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<Device>, String> {
    let out = mobile::run_cmd("adb", &["devices", "-l"], CMD_TIMEOUT, cancel, ADB_HINT).await?;
    Ok(out
        .stdout
        .lines()
        .skip(1) // "List of devices attached"
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            Some(Device { serial, state })
        })
        .collect())
}

/// Resolve the `serial` argument: use it verbatim if given, otherwise pick
/// the single running device, or error listing candidates if there are zero
/// or several.
async fn resolve_serial(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    if let Some(serial) = arg_str_opt(args, "serial") {
        return Ok(serial);
    }
    let devices = list_adb_devices(&ctx.cancel).await?;
    match devices.len() {
        0 => Err("No Android device/emulator is running. Call android_list_devices, then android_boot_emulator.".to_string()),
        1 => Ok(devices.into_iter().next().unwrap().serial),
        _ => {
            let serials: Vec<_> = devices.iter().map(|d| d.serial.as_str()).collect();
            Err(format!(
                "Multiple devices are running ({}); pass `serial` to pick one.",
                serials.join(", ")
            ))
        }
    }
}

/// Encode a string for `adb shell input text`, which passes its argument
/// through an on-device shell: spaces use `input text`'s own `%s` escape,
/// and shell-metacharacters are backslash-escaped so the device shell treats
/// them literally. Not a complete shell-quoting implementation — documented
/// as a known limitation on `AndroidTypeText::description()`.
fn escape_adb_text(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' ' => "%s".to_string(),
            '\'' | '"' | '\\' | '$' | '`' | '(' | ')' | ';' | '&' | '|' | '<' | '>' | '*' | '?'
            | '~' | '#' => {
                format!("\\{c}")
            }
            _ => c.to_string(),
        })
        .collect()
}

fn keyevent_code(button: &str) -> Result<&'static str, String> {
    Ok(match button {
        "home" => "3",
        "back" => "4",
        "power" => "26",
        "volume_up" => "24",
        "volume_down" => "25",
        "app_switch" => "187",
        "enter" => "66",
        "menu" => "82",
        other => return Err(format!(
            "unknown button `{other}` (expected one of: home, back, power, volume_up, volume_down, app_switch, enter, menu)"
        )),
    })
}

pub struct ListDevices;

#[async_trait]
impl ToolExecutor for ListDevices {
    fn name(&self) -> &str {
        "android_list_devices"
    }
    fn description(&self) -> &str {
        "List running Android devices/emulators (from `adb devices`) and, best-effort, available AVDs to boot."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let devices = match list_adb_devices(&ctx.cancel).await {
            Ok(d) => d,
            Err(e) => return ToolOutcome::error(e),
        };
        let mut body = String::new();
        if devices.is_empty() {
            body.push_str("Running: (none)\n");
        } else {
            body.push_str("Running:\n");
            for d in &devices {
                body.push_str(&format!("  {} ({})\n", d.serial, d.state));
            }
        }
        // Best-effort: the `emulator` binary may not be installed even when
        // `adb` is (platform-tools vs. the full SDK emulator package).
        match mobile::run_cmd(
            "emulator",
            &["-list-avds"],
            CMD_TIMEOUT,
            &ctx.cancel,
            EMULATOR_HINT,
        )
        .await
        {
            Ok(out) => {
                body.push_str("Available AVDs:\n");
                let avds: Vec<_> = out
                    .stdout
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .collect();
                if avds.is_empty() {
                    body.push_str("  (none)\n");
                } else {
                    for avd in avds {
                        body.push_str(&format!("  {}\n", avd.trim()));
                    }
                }
            }
            Err(_) => body.push_str("Available AVDs: (couldn't list — `emulator` not on PATH)\n"),
        }
        ToolOutcome::ok(body.trim_end().to_string())
    }
}

pub struct BootEmulator;

#[async_trait]
impl ToolExecutor for BootEmulator {
    fn name(&self) -> &str {
        "android_boot_emulator"
    }
    fn description(&self) -> &str {
        "Boot an Android Virtual Device (AVD) by name and wait until it finishes booting. Use android_list_devices first to see available AVD names."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "avd_name": {"type": "string", "description": "The AVD name to boot, exactly as reported by `emulator -list-avds`."}
            },
            "required": ["avd_name"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let avd = match arg_str(&args, "avd_name") {
            Ok(a) => a,
            Err(e) => return ToolOutcome::error(e),
        };
        let before: Vec<String> = match list_adb_devices(&ctx.cancel).await {
            Ok(d) => d.into_iter().map(|d| d.serial).collect(),
            Err(e) => return ToolOutcome::error(e),
        };

        // Spawn detached: this is a long-running GUI process, not a one-shot
        // command. We don't wait on it or pipe its output — `adb` polling
        // below is the actual "is it ready" signal.
        let spawn = tokio::process::Command::new("emulator")
            .arg("-avd")
            .arg(&avd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(false)
            .spawn();
        if let Err(e) = spawn {
            return if e.kind() == std::io::ErrorKind::NotFound {
                ToolOutcome::error(EMULATOR_HINT)
            } else {
                ToolOutcome::error(format!("failed to start emulator: {e}"))
            };
        }

        let deadline = tokio::time::Instant::now() + BOOT_TIMEOUT;
        let mut serial: Option<String> = None;
        while tokio::time::Instant::now() < deadline {
            tokio::select! {
                _ = ctx.cancel.cancelled() => return ToolOutcome::error("cancelled"),
                _ = tokio::time::sleep(BOOT_POLL_INTERVAL) => {}
            }
            let Ok(current) = list_adb_devices(&ctx.cancel).await else {
                continue;
            };
            if let Some(new_serial) = current
                .iter()
                .map(|d| &d.serial)
                .find(|s| !before.contains(s))
            {
                serial = Some(new_serial.clone());
                break;
            }
        }
        let Some(serial) = serial else {
            return ToolOutcome::error(format!(
                "{avd} didn't appear in `adb devices` within {}s",
                BOOT_TIMEOUT.as_secs()
            ));
        };

        while tokio::time::Instant::now() < deadline {
            if let Ok(out) = mobile::run_cmd(
                "adb",
                &["-s", &serial, "shell", "getprop", "sys.boot_completed"],
                CMD_TIMEOUT,
                &ctx.cancel,
                ADB_HINT,
            )
            .await
            {
                if out.stdout.trim() == "1" {
                    return ToolOutcome::ok(format!("Booted {avd} as {serial}."));
                }
            }
            tokio::select! {
                _ = ctx.cancel.cancelled() => return ToolOutcome::error("cancelled"),
                _ = tokio::time::sleep(BOOT_POLL_INTERVAL) => {}
            }
        }
        ToolOutcome::error(format!(
            "{avd} started (serial {serial}) but didn't finish booting within {}s",
            BOOT_TIMEOUT.as_secs()
        ))
    }
}

pub struct ShutdownEmulator;

#[async_trait]
impl ToolExecutor for ShutdownEmulator {
    fn name(&self) -> &str {
        "android_shutdown_emulator"
    }
    fn description(&self) -> &str {
        "Shut down a running Android emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial (e.g. `emulator-5554`). Omit if only one device is running."}
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
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "emu", "kill"],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(_) => ToolOutcome::ok(format!("Shut down {serial}.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct InstallApp;

#[async_trait]
impl ToolExecutor for InstallApp {
    fn name(&self) -> &str {
        "android_install_app"
    }
    fn description(&self) -> &str {
        "Install an APK onto a running Android device/emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
                "apk_path": {"type": "string", "description": "Path to the .apk to install."}
            },
            "required": ["apk_path"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let apk_path = match arg_str(&args, "apk_path") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "install", "-r", &apk_path],
            Duration::from_secs(120),
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(out) if out.stdout.contains("Success") => {
                ToolOutcome::ok(format!("Installed {apk_path} on {serial}."))
            }
            Ok(out) => ToolOutcome::error(format!(
                "install failed: {}{}",
                out.stdout.trim(),
                out.stderr.trim()
            )),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct UninstallApp;

#[async_trait]
impl ToolExecutor for UninstallApp {
    fn name(&self) -> &str {
        "android_uninstall_app"
    }
    fn description(&self) -> &str {
        "Uninstall an app by package name from a running Android device/emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
                "package": {"type": "string", "description": "Package name, e.g. `com.example.app`."}
            },
            "required": ["package"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let package = match arg_str(&args, "package") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "uninstall", &package],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(out) if out.stdout.contains("Success") => {
                ToolOutcome::ok(format!("Uninstalled {package} from {serial}."))
            }
            Ok(out) => ToolOutcome::error(format!(
                "uninstall failed: {}{}",
                out.stdout.trim(),
                out.stderr.trim()
            )),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct LaunchApp;

#[async_trait]
impl ToolExecutor for LaunchApp {
    fn name(&self) -> &str {
        "android_launch_app"
    }
    fn description(&self) -> &str {
        "Launch an installed app by package name (optionally a specific activity) on a running Android device/emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
                "package": {"type": "string", "description": "Package name, e.g. `com.example.app`."},
                "activity": {"type": "string", "description": "Specific activity to launch (e.g. `.MainActivity`). Omit to launch the app's default launcher activity."}
            },
            "required": ["package"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let package = match arg_str(&args, "package") {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e),
        };
        let activity = arg_str_opt(&args, "activity");
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let result = if let Some(activity) = &activity {
            let component = format!("{package}/{activity}");
            mobile::run_cmd(
                "adb",
                &["-s", &serial, "shell", "am", "start", "-n", &component],
                CMD_TIMEOUT,
                &ctx.cancel,
                ADB_HINT,
            )
            .await
        } else {
            mobile::run_cmd(
                "adb",
                &[
                    "-s",
                    &serial,
                    "shell",
                    "monkey",
                    "-p",
                    &package,
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
                CMD_TIMEOUT,
                &ctx.cancel,
                ADB_HINT,
            )
            .await
        };
        match result {
            Ok(out) if out.code == Some(0) => {
                ToolOutcome::ok(format!("Launched {package} on {serial}."))
            }
            Ok(out) => ToolOutcome::error(format!(
                "launch failed: {}{}",
                out.stdout.trim(),
                out.stderr.trim()
            )),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct Screenshot;

#[async_trait]
impl ToolExecutor for Screenshot {
    fn name(&self) -> &str {
        "android_screenshot"
    }
    fn description(&self) -> &str {
        "Capture a screenshot of a running Android device/emulator. Returns the image so you can visually inspect the current screen."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."}
            }
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let (bytes, code) = match mobile::run_cmd_bytes(
            "adb",
            &["-s", &serial, "exec-out", "screencap", "-p"],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return ToolOutcome::error(e),
        };
        if code != Some(0) || bytes.is_empty() {
            return ToolOutcome::error("screencap returned no image data");
        }
        let dir: PathBuf = mobile::screenshot_dir(ctx);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return ToolOutcome::error(format!("couldn't create {}: {e}", dir.display()));
        }
        let path = dir.join(format!("android-{serial}-{}.png", mobile::timestamp_slug()));
        if let Err(e) = std::fs::write(&path, &bytes) {
            return ToolOutcome::error(format!("couldn't write {}: {e}", path.display()));
        }
        mobile::prune_screenshots(&dir);
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        ToolOutcome::ok(format!("Captured a screenshot of {serial}."))
            .with_image(
                "image/png",
                data_base64,
                Some(format!("Android screenshot ({serial})")),
            )
            .with_location(path.display().to_string(), None)
    }
}

pub struct Tap;

#[async_trait]
impl ToolExecutor for Tap {
    fn name(&self) -> &str {
        "android_tap"
    }
    fn description(&self) -> &str {
        "Simulate a tap at (x, y) pixel coordinates on a running Android device/emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
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
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let xs = x.to_string();
        let ys = y.to_string();
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "shell", "input", "tap", &xs, &ys],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(_) => ToolOutcome::ok(format!("Tapped ({x}, {y}) on {serial}.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct Swipe;

#[async_trait]
impl ToolExecutor for Swipe {
    fn name(&self) -> &str {
        "android_swipe"
    }
    fn description(&self) -> &str {
        "Simulate a swipe from (x1, y1) to (x2, y2) pixel coordinates on a running Android device/emulator."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
                "x1": {"type": "integer"},
                "y1": {"type": "integer"},
                "x2": {"type": "integer"},
                "y2": {"type": "integer"},
                "duration_ms": {"type": "integer", "description": "Swipe duration in milliseconds. Omit for the device default."}
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
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let (x1s, y1s, x2s, y2s) = (
            x1.to_string(),
            y1.to_string(),
            x2.to_string(),
            y2.to_string(),
        );
        let mut cmd_args = vec![
            "-s", &serial, "shell", "input", "swipe", &x1s, &y1s, &x2s, &y2s,
        ];
        let duration_str = duration_ms.map(|d| d.to_string());
        if let Some(d) = &duration_str {
            cmd_args.push(d);
        }
        match mobile::run_cmd("adb", &cmd_args, CMD_TIMEOUT, &ctx.cancel, ADB_HINT).await {
            Ok(_) => ToolOutcome::ok(format!("Swiped ({x1},{y1}) -> ({x2},{y2}) on {serial}.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct TypeText;

#[async_trait]
impl ToolExecutor for TypeText {
    fn name(&self) -> &str {
        "android_type_text"
    }
    fn description(&self) -> &str {
        "Type text into the currently focused field on a running Android device/emulator. Basic escaping only (spaces and common shell metacharacters) — very unusual text (emoji, quotes-within-quotes) may not type reliably."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
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
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        let escaped = escape_adb_text(&text);
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "shell", "input", "text", &escaped],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(_) => ToolOutcome::ok(format!("Typed text on {serial}.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

pub struct PressButton;

#[async_trait]
impl ToolExecutor for PressButton {
    fn name(&self) -> &str {
        "android_press_button"
    }
    fn description(&self) -> &str {
        "Press a hardware/software button on a running Android device/emulator: home, back, power, volume_up, volume_down, app_switch, enter, menu."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "serial": {"type": "string", "description": "Device serial. Omit if only one device is running."},
                "button": {"type": "string", "enum": ["home", "back", "power", "volume_up", "volume_down", "app_switch", "enter", "menu"]}
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
        let code = match keyevent_code(&button) {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };
        let serial = match resolve_serial(&args, ctx).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        match mobile::run_cmd(
            "adb",
            &["-s", &serial, "shell", "input", "keyevent", code],
            CMD_TIMEOUT,
            &ctx.cancel,
            ADB_HINT,
        )
        .await
        {
            Ok(_) => ToolOutcome::ok(format!("Pressed {button} on {serial}.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_devices_output() {
        let stdout =
            "List of devices attached\nemulator-5554\tdevice product:sdk_gphone64_arm64\n\n";
        let devices: Vec<Device> = stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let serial = parts.next()?.to_string();
                let state = parts.next()?.to_string();
                Some(Device { serial, state })
            })
            .collect();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].state, "device");
    }

    #[test]
    fn escapes_spaces_and_shell_metacharacters() {
        assert_eq!(escape_adb_text("hello world"), "hello%sworld");
        assert_eq!(escape_adb_text("a&b"), "a\\&b");
        assert_eq!(escape_adb_text("it's"), "it\\'s");
    }

    #[test]
    fn keyevent_code_maps_known_buttons() {
        assert_eq!(keyevent_code("home").unwrap(), "3");
        assert_eq!(keyevent_code("back").unwrap(), "4");
        assert!(keyevent_code("bogus").is_err());
    }
}
