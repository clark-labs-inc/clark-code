//! Clark Desktop — Tauri host.
//!
//! The host owns the native `agent-core` engine: it holds provider instances,
//! drives transports/sidecars, and bridges to the web UI via Tauri commands
//! (`invoke`) and events (`emit`): provider discovery, the command surface, the
//! live ACP provider, and snapshot streaming.

use agent_core::ProviderCapabilities;
use serde::Serialize;

mod commands;
mod state;
mod terminal;

pub use state::AppState;

/// Metadata for a provider the UI can offer. Mirrors the frontend `ProviderInfo`.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    pub capabilities: ProviderCapabilities,
}

/// The providers this build ships with. Clark Desktop is a coding client: it
/// ships the local coding agent only (the model + research route through the
/// production Clark Platform API).
pub fn builtin_providers() -> Vec<ProviderInfo> {
    vec![ProviderInfo {
        id: "local".into(),
        label: "Clark Code".into(),
        capabilities: ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            modes: Vec::new(),
        },
    }]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clark_desktop_lib=info,agent_core=info".into()),
        )
        .try_init()
        .ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_google_auth::init())
        .manage(AppState::new())
        .manage(terminal::Terminals::default())
        .invoke_handler(tauri::generate_handler![
            commands::provider_list,
            commands::provider_connect,
            commands::session_new,
            commands::session_load,
            commands::prompt,
            commands::cancel,
            commands::respond,
            commands::local_extract_memory,
            commands::local_list_memory,
            commands::local_list_files,
            commands::open_path,
            commands::clark_exchange_google_idtoken,
            commands::clark_provision_code_key,
            commands::clark_billing_me,
            commands::clark_checkpoint_restore,
            commands::clark_mcp_probe,
            commands::desktop_conv_list,
            commands::desktop_conv_get,
            commands::desktop_conv_put,
            commands::desktop_conv_delete,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Clark Code");
}
