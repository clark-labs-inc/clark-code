//! Owner-local structured diagnostics for the native Desktop boundary.
//!
//! `AGENT_DESKTOP_LOGS` and `AGENT_DESKTOP_CAPTURED_LOGS` are deliberately only directory
//! selectors. Events are emitted through `tracing`, so providers and commands
//! do not know or care which local sinks are configured. Conversation content,
//! credentials, and request bodies must never be attached to these events.

#[cfg(all(feature = "debug-diagnostics", not(debug_assertions)))]
compile_error!("debug-diagnostics is forbidden in release/production builds");

use serde::Deserialize;
#[cfg(feature = "debug-diagnostics")]
use {
    diagnostic_capture::{CaptureClient, CaptureConfig, CaptureLayer, EventInput, Level},
    serde_json::{json, Map, Value},
    std::path::{Path, PathBuf},
    std::sync::atomic::{AtomicBool, Ordering},
    std::sync::OnceLock,
    tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard},
    tracing_appender::rolling::{RollingFileAppender, Rotation},
    tracing_subscriber::layer::SubscriberExt,
    tracing_subscriber::Layer,
};

#[cfg(feature = "debug-diagnostics")]
const RETAINED_LOG_FILES: usize = 15;
#[cfg(feature = "debug-diagnostics")]
const CAPTURE_FILTER: &str = "desktop_foundation=info,agent_core=info,provider_local=info,code_remote=info,exec_core=warn,tauri=warn";
#[cfg(feature = "debug-diagnostics")]
static INITIALIZED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "debug-diagnostics")]
static CAPTURE_CLIENT: OnceLock<CaptureClient> = OnceLock::new();

pub struct DiagnosticsGuard {
    #[cfg(feature = "debug-diagnostics")]
    _file_guard: Option<WorkerGuard>,
}

#[cfg(feature = "debug-diagnostics")]
fn configured_log_directory() -> Option<PathBuf> {
    std::env::var_os("AGENT_DESKTOP_LOGS")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(feature = "debug-diagnostics")]
fn prepare_log_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("AGENT_DESKTOP_LOGS must be an absolute directory".into());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| format!("create AGENT_DESKTOP_LOGS directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect AGENT_DESKTOP_LOGS directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("AGENT_DESKTOP_LOGS must be a real directory, not a symlink".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure AGENT_DESKTOP_LOGS directory: {error}"))?;
    }
    Ok(())
}

#[cfg(feature = "debug-diagnostics")]
fn file_appender(
    path: &Path,
) -> Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard), String> {
    prepare_log_directory(path)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("agent-desktop")
        .filename_suffix("jsonl")
        // The appender documents that retention can briefly dip below the
        // configured maximum, so keep one spare beyond the intended 14 days.
        .max_log_files(RETAINED_LOG_FILES)
        .build(path)
        .map_err(|error| format!("open AGENT_DESKTOP_LOGS appender: {error}"))?;
    Ok(NonBlockingBuilder::default().lossy(false).finish(appender))
}

#[cfg(feature = "debug-diagnostics")]
fn capture_layer() -> Option<CaptureLayer> {
    let path = std::env::var_os("AGENT_DESKTOP_CAPTURED_LOGS")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    if let Err(error) = prepare_capture_directory(&path) {
        eprintln!("Clark Code capture sink unavailable: {error}");
        return None;
    }
    let mut config = CaptureConfig::with_root("agent-desktop", path);
    config.release = Some(env!("CARGO_PKG_VERSION").to_owned());
    config.environment = Some("development".to_owned());
    let client = match CaptureClient::new(config) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Clark Code capture client unavailable: {error}");
            return None;
        }
    };
    client.set_tag("application", "agent-desktop");
    client.set_tag("capture_transport", "local-disk");
    client.install_panic_hook();
    let _ = CAPTURE_CLIENT.set(client.clone());
    Some(CaptureLayer::new(client))
}

#[cfg(feature = "debug-diagnostics")]
fn prepare_capture_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("AGENT_DESKTOP_CAPTURED_LOGS must be an absolute directory".into());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| format!("create AGENT_DESKTOP_CAPTURED_LOGS directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect AGENT_DESKTOP_CAPTURED_LOGS directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("AGENT_DESKTOP_CAPTURED_LOGS must be a real directory, not a symlink".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure AGENT_DESKTOP_CAPTURED_LOGS directory: {error}"))?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontendDiagnosticKind {
    Exception,
    Rejection,
    Boundary,
}

#[derive(Deserialize)]
pub(crate) struct FrontendDiagnostic {
    kind: FrontendDiagnosticKind,
    name: Option<String>,
    reference: String,
    stack_frames: Option<String>,
    source: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    component_stack: Option<String>,
}

#[tauri::command]
pub(crate) fn capture_frontend_diagnostic(input: FrontendDiagnostic) -> Result<(), String> {
    #[cfg(not(feature = "debug-diagnostics"))]
    {
        let FrontendDiagnostic {
            kind,
            name,
            reference,
            stack_frames,
            source,
            line,
            column,
            component_stack,
        } = input;
        let _ = (
            kind,
            name,
            reference,
            stack_frames,
            source,
            line,
            column,
            component_stack,
        );
        return Ok(());
    }

    #[cfg(feature = "debug-diagnostics")]
    {
        let Some(client) = CAPTURE_CLIENT.get() else {
            return Ok(());
        };
        let mechanism = match input.kind {
            FrontendDiagnosticKind::Exception => "window_error",
            FrontendDiagnosticKind::Rejection => "unhandled_rejection",
            FrontendDiagnosticKind::Boundary => "react_error_boundary",
        };
        let mut payload = Map::new();
        payload.insert("exceptions".into(), json!([{
        "type": safe_error_name(input.name.as_deref()),
        "value": bounded(&input.reference, 64),
        "stacktrace": input.stack_frames.as_deref().map(|stack| bounded(stack, 32_768)).unwrap_or_default(),
    }]));
        payload.insert("mechanism".into(), Value::String(mechanism.into()));
        let mut event = EventInput::new("exception", Level::Error, payload);
        event.tags.insert("surface".into(), "frontend".into());
        event.tags.insert("mechanism".into(), mechanism.into());
        let mut browser = Map::new();
        if let Some(source) = input.source {
            browser.insert("source".into(), Value::String(bounded(&source, 2_048)));
        }
        if let Some(line) = input.line {
            browser.insert("line".into(), Value::Number(line.into()));
        }
        if let Some(column) = input.column {
            browser.insert("column".into(), Value::Number(column.into()));
        }
        if let Some(stack) = input.component_stack {
            browser.insert(
                "component_stack".into(),
                Value::String(bounded(&stack, 16_384)),
            );
        }
        event
            .contexts
            .insert("browser".into(), Value::Object(browser));
        client
            .capture(event)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(feature = "debug-diagnostics")]
fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(feature = "debug-diagnostics")]
fn safe_error_name(value: Option<&str>) -> String {
    let value = value.unwrap_or("Error");
    let sanitized = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .take(160)
        .collect::<String>();
    if sanitized.is_empty() {
        "Error".into()
    } else {
        sanitized
    }
}

#[cfg(feature = "debug-diagnostics")]
pub(crate) fn init() -> DiagnosticsGuard {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return DiagnosticsGuard { _file_guard: None };
    }
    let console_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "desktop_foundation=info,agent_core=info".into());
    let console = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    if let Some(path) = configured_log_directory() {
        match file_appender(&path) {
            Ok((writer, guard)) => {
                let file = tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_ansi(false)
                    .with_writer(writer);
                let subscriber = tracing_subscriber::registry()
                    .with(console.with_filter(console_filter))
                    // Local diagnostics stay useful even when a shell-level
                    // `RUST_LOG=warn` quiets the interactive console.
                    .with(file.with_filter(tracing_subscriber::EnvFilter::new(
                        "desktop_foundation=info,agent_core=info",
                    )))
                    .with(capture_layer().map(|layer| {
                        layer.with_filter(tracing_subscriber::EnvFilter::new(CAPTURE_FILTER))
                    }));
                // Set only the tracing dispatcher here. `try_init` also owns
                // the legacy `log` facade and can fail if a GUI dependency
                // installed that bridge before Clark Code's native host starts.
                let initialized = tracing::subscriber::set_global_default(subscriber).is_ok();
                if initialized {
                    tracing::info!(
                        event = "diagnostics_initialized",
                        log_directory = %path.display(),
                        retained_log_files = RETAINED_LOG_FILES,
                        "owner-local diagnostics enabled"
                    );
                    return DiagnosticsGuard {
                        _file_guard: Some(guard),
                    };
                }
                return DiagnosticsGuard { _file_guard: None };
            }
            Err(error) => eprintln!("Clark Code diagnostics file sink unavailable: {error}"),
        }
    }

    let subscriber = tracing_subscriber::registry()
        .with(console.with_filter(console_filter))
        .with(
            capture_layer()
                .map(|layer| layer.with_filter(tracing_subscriber::EnvFilter::new(CAPTURE_FILTER))),
        );
    let _ = tracing::subscriber::set_global_default(subscriber);
    DiagnosticsGuard { _file_guard: None }
}

#[cfg(not(feature = "debug-diagnostics"))]
pub(crate) fn init() -> DiagnosticsGuard {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "desktop_foundation=info,agent_core=info".into()),
        )
        .try_init()
        .ok();
    DiagnosticsGuard {}
}
