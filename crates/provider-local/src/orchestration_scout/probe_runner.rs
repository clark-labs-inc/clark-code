use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_orchestration::ScoutRunnerId;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{ProbeOperation, ProbeReceipt, ProbeRecipe};
use crate::tools::ToolCtx;

const MAX_PROBE_BYTES: usize = 2 * 1024 * 1024;

pub(super) async fn execute_recipe(
    recipe: &ProbeRecipe,
    ctx: &ToolCtx,
) -> Result<ProbeReceipt, String> {
    let path = ctx.sandbox.resolve_existing(&recipe.path)?;
    if sensitive_path(&path) {
        return Err("refusing to probe a secret-bearing path".into());
    }
    let bytes = ctx.executor.read(&path).await?;
    if bytes.len() > MAX_PROBE_BYTES {
        return Err(format!(
            "probe input exceeds the {MAX_PROBE_BYTES} byte limit"
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "probe input is not UTF-8")?;
    let (selector, result) = match recipe.operation {
        ProbeOperation::SourceSlice => {
            let start = recipe.line_start.unwrap_or(1);
            let end = recipe.line_end.unwrap_or(start);
            let lines = text
                .lines()
                .enumerate()
                .filter(|(index, _)| {
                    let line = *index as u64 + 1;
                    line >= start && line <= end
                })
                .map(|(index, line)| json!({"line": index + 1, "text": redact_source_line(line)}))
                .collect::<Vec<_>>();
            (json!({"line_start": start, "line_end": end}), json!(lines))
        }
        ProbeOperation::TextCount => {
            let needle = recipe.needle.as_deref().unwrap_or_default();
            (
                json!({"needle_sha256": format!("{:x}", Sha256::digest(needle.as_bytes()))}),
                json!({"count": text.matches(needle).count()}),
            )
        }
        ProbeOperation::JsonArrayCount => {
            let pointer = recipe.json_pointer.as_deref().unwrap_or_default();
            let document: Value =
                serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
            let value = document
                .pointer(pointer)
                .ok_or_else(|| format!("JSON pointer not found: {pointer}"))?;
            let array = value
                .as_array()
                .ok_or_else(|| format!("JSON pointer is not an array: {pointer}"))?;
            (
                json!({"json_pointer": pointer}),
                json!({"count": array.len()}),
            )
        }
    };
    Ok(ProbeReceipt {
        schema_version: "scout-probe-receipt-v1",
        operation: recipe.operation,
        path: ctx.sandbox.display(&path),
        input_sha256: format!("{:x}", Sha256::digest(&bytes)),
        selector,
        result,
    })
}

pub(super) fn receipt_digest(receipt: &ProbeReceipt) -> String {
    let encoded = serde_json::to_vec(receipt).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

pub(in super::super) fn resolve_probe_path(ctx: &ToolCtx, path: &str) -> Result<PathBuf, String> {
    if !project_relative_path(path) {
        return Err("scout_probe paths must be project-relative".into());
    }
    ctx.sandbox.resolve_existing(path)
}

pub(super) fn validate_path_scope(ctx: &ToolCtx, path: &Path, scope: &str) -> Result<(), String> {
    if !path.starts_with(ctx.sandbox.root()) {
        return Err("probe path is outside the project root".into());
    }
    if scope == "." || scope == "repo" {
        return Ok(());
    }
    let scope_path = ctx.sandbox.resolve_existing(scope)?;
    if !scope_path.starts_with(ctx.sandbox.root()) {
        Err(format!(
            "declared scope {scope} is outside the project root"
        ))
    } else if path.starts_with(scope_path) {
        Ok(())
    } else {
        Err(format!("probe path is outside declared scope {scope}"))
    }
}

pub(super) fn project_relative_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    !Path::new(path).is_absolute()
        && !path.starts_with('\\')
        && !path.starts_with('/')
        && !matches!(bytes, [drive, b':', ..] if drive.is_ascii_alphabetic())
}

pub(super) fn source_label(recipe: &ProbeRecipe) -> String {
    match recipe.operation {
        ProbeOperation::SourceSlice => format!(
            "{}:{}-{}",
            recipe.path,
            recipe.line_start.unwrap_or(1),
            recipe.line_end.unwrap_or(recipe.line_start.unwrap_or(1))
        ),
        ProbeOperation::TextCount => format!("{}:text_count", recipe.path),
        ProbeOperation::JsonArrayCount => format!("{}:json_array_count", recipe.path),
    }
}

pub(in super::super) fn sensitive_path(path: &Path) -> bool {
    let lower = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let rooted = format!("/{}", lower.trim_start_matches('/'));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".env")
        || name == "credentials"
        || name == ".npmrc"
        || name == ".pypirc"
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.contains("secret")
        || [
            "/.aws/",
            "/.ssh/",
            "/.config/gh/",
            "/.config/gcloud/",
            "/.azure/",
            "/.docker/",
            "/.kube/",
        ]
        .iter()
        .any(|segment| rooted.contains(segment))
}

pub(super) fn redact_source_line(line: &str) -> &str {
    let lower = line.to_ascii_lowercase();
    let looks_assigned = lower.contains('=')
        || lower.contains(':')
        || lower.contains("bearer ")
        || lower.contains("basic ");
    let has_secret_marker = [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "api-key",
        "access_key",
        "access-key",
        "private_key",
        "private-key",
        "authorization",
        "cookie",
        "session_key",
        "session-key",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if looks_assigned && has_secret_marker {
        "[REDACTED: possible secret-bearing line]"
    } else {
        line
    }
}

pub(super) fn runner_id(label: &str) -> ScoutRunnerId {
    ScoutRunnerId::new(format!("{label}:{}", Uuid::new_v4())).expect("valid generated runner id")
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
