//! Clark Desktop — Tauri host.
//!
//! The host owns the native `agent-core` engine: it holds provider instances,
//! drives transports/sidecars, and bridges to the web UI via Tauri commands
//! (`invoke`) and events (`emit`): provider discovery, the command surface, the
//! live ACP provider, and snapshot streaming.

use agent_core::ProviderCapabilities;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

mod commands;
mod mobile_remote;
// Public so the gated `tests/remote_e2e.rs` harness can drive the real
// orchestration against a live host; otherwise host-internal.
pub mod ssh;
mod state;
mod terminal;
mod trajectory;

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

    let mut builder = tauri::Builder::default();

    // Single-instance must be registered FIRST so a second launch (e.g. the OS
    // opening a `clark://` URL on Windows/Linux) is funneled into the already
    // running process rather than spawning a new window. Its `deep-link` feature
    // re-emits the URL, so the `on_open_url` handler below still fires; this
    // callback just raises the existing window. macOS routes deep links to the
    // running instance natively, so this is a belt-and-suspenders no-op there.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_google_auth::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(AppState::new())
        .manage(terminal::Terminals::default())
        .invoke_handler(tauri::generate_handler![
            commands::provider_list,
            commands::provider_connect,
            commands::provider_reconfigure,
            commands::ssh_connect,
            commands::ssh_disconnect,
            commands::ssh_probe,
            commands::claude_discover,
            commands::list_commands,
            commands::session_new,
            commands::session_load,
            commands::session_close,
            commands::session_configure_cloud,
            commands::update_cloud_token,
            commands::prompt,
            commands::cancel,
            commands::respond,
            commands::set_mode,
            commands::set_output_style,
            commands::local_list_memory,
            commands::local_list_global_memory,
            commands::local_list_files,
            commands::read_doc_text,
            commands::read_image_data_url,
            commands::save_doc_text,
            commands::open_path,
            commands::clark_exchange_google_idtoken,
            commands::clark_provision_code_key,
            commands::clark_billing_me,
            commands::clark_checkpoint_restore,
            commands::clark_repository_inspect,
            commands::clark_repository_discover,
            commands::clark_repository_history,
            commands::changes_summary,
            commands::changes_diff,
            commands::changes_revert,
            commands::clark_mcp_probe,
            commands::desktop_conv_list,
            commands::desktop_conv_get,
            commands::desktop_conv_put,
            commands::desktop_conv_share,
            commands::desktop_conv_unshare,
            commands::desktop_conv_delete,
            commands::desktop_conv_set_archived,
            mobile_remote::desktop_code_host_upsert,
            mobile_remote::desktop_code_command_poll,
            mobile_remote::desktop_code_command_ack,
            mobile_remote::desktop_code_repository_sync,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
        ])
        .setup(|app| {
            // The Google sign-in success page (served by the loopback) redirects
            // to `clark://auth-complete`; the OS routes that URL here so we can
            // pull the window back to the foreground instead of leaving the user
            // stranded on a browser tab.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |_event| {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window minimizes it instead of quitting, so the local
            // agent loop, any in-flight run, and background sync keep running.
            // The app still quits via Cmd+Q or the app menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.minimize();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Clark Code");
}
