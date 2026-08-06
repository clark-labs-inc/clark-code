use serde_json::Value;

use super::{arg_str_opt, mobile, ToolCtx, ADB_HINT, CMD_TIMEOUT};

pub(super) struct Device {
    pub(super) serial: String,
    pub(super) state: String,
}

pub(super) async fn list_adb_devices(
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<Device>, String> {
    let out = mobile::run_cmd("adb", &["devices", "-l"], CMD_TIMEOUT, cancel, ADB_HINT).await?;
    Ok(parse_adb_devices(&out.stdout))
}

pub(super) fn parse_adb_devices(stdout: &str) -> Vec<Device> {
    stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some(Device {
                serial: parts.next()?.to_string(),
                state: parts.next()?.to_string(),
            })
        })
        .collect()
}

pub(super) async fn resolve_serial(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    if let Some(serial) = arg_str_opt(args, "serial") {
        return Ok(serial);
    }
    let devices = list_adb_devices(&ctx.cancel).await?;
    match devices.len() {
        0 => Err("No Android device/emulator is running. Call android_list_devices, then android_boot_emulator.".to_string()),
        1 => Ok(devices.into_iter().next().unwrap().serial),
        _ => {
            let serials: Vec<_> = devices.iter().map(|device| device.serial.as_str()).collect();
            Err(format!(
                "Multiple devices are running ({}); pass `serial` to pick one.",
                serials.join(", ")
            ))
        }
    }
}

pub(super) fn escape_adb_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            ' ' => "%s".to_string(),
            '\'' | '"' | '\\' | '$' | '`' | '(' | ')' | ';' | '&' | '|' | '<' | '>' | '*' | '?'
            | '~' | '#' => format!("\\{character}"),
            _ => character.to_string(),
        })
        .collect()
}

pub(super) fn keyevent_code(button: &str) -> Result<&'static str, String> {
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
