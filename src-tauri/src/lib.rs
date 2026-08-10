//! Clark Code — Tauri host.
//!
//! The host owns the native `agent-core` engine: it holds provider instances,
//! drives transports/sidecars, and bridges to the web UI via Tauri commands
//! (`invoke`) and events (`emit`): provider discovery, the command surface, the
//! live ACP provider, and snapshot streaming.

use agent_core::{CollaborationMode, ProviderCapabilities};
use serde::Serialize;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

mod commands;
mod diagnostics;
mod document_preview;
mod file_actions;
mod markdown_export;
pub mod product;
mod project_context;
mod project_worktree;
mod remote_worker_executor;
mod runtime_registry;
mod sandbox_setup;
mod security_report;
mod session_credentials;
// Public so the gated `tests/remote_e2e.rs` harness can drive the real
// orchestration against a live host; otherwise host-internal.
pub mod ssh;
mod state;
mod terminal;
mod trajectory;
#[cfg(desktop)]
mod updater_menu;
mod windows_release_smoke;

pub use state::AppState;
pub use windows_release_smoke::run_windows_sandbox_smoke_if_requested;

/// Install diagnostics before any optional packaged-product smoke initializes
/// a legacy logging bridge. The binary calls this as its first operation; the
/// library entry point remains idempotent for mobile and embedding callers.
pub fn init_diagnostics() -> diagnostics::DiagnosticsGuard {
    diagnostics::init()
}

/// Metadata for a provider the UI can offer. Mirrors the frontend `ProviderInfo`.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    pub capabilities: ProviderCapabilities,
    pub internal: bool,
}

const SIGNED_COMPUTER_USE_SMOKE_ARG: &str = "--computer-use-signed-smoke";
const WINDOWS_CONSOLE_SMOKE_ARG: &str = "--windows-console-smoke";

/// Read-only packaged-app probe for the release workflow. Calling
/// `permissions` forces the real parent/helper handshake and both code-signing
/// checks, but does not trigger either macOS privacy prompt.
pub fn run_signed_computer_use_smoke_if_requested() -> bool {
    if !signed_computer_use_smoke_requested(std::env::args_os().skip(1)) {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        let result = computer_use::native_backend()
            .and_then(|backend| backend.permissions())
            .and_then(|permissions| {
                println!(
                    "{}",
                    serde_json::to_string(&permissions).map_err(|error| {
                        computer_use::ComputerUseError::HelperProtocol(format!(
                            "could not serialize signed smoke result: {error}"
                        ))
                    })?
                );
                Ok(())
            });
        if let Err(error) = result {
            eprintln!("signed computer-use smoke failed: {error}");
            std::process::exit(1);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("signed computer-use smoke is supported only on macOS");
        std::process::exit(2);
    }

    true
}

/// Packaged Windows release probe for process-launch visibility. It exercises
/// the same pipe-backed ordinary executor, ConPTY executor, and portable
/// Computer Use child used by the app, then writes a private machine-readable
/// receipt for the UTM monitor. The caller owns visible-window observation.
pub fn run_windows_console_smoke_if_requested() -> bool {
    let Some(output) = windows_console_smoke_output(std::env::args_os().skip(1)) else {
        return false;
    };

    #[cfg(windows)]
    {
        use exec_core::{Executor, ShellKind};
        use serde_json::json;
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| {
                eprintln!("Windows console smoke runtime failed: {error}");
                std::process::exit(1);
            });
        let cwd = std::env::current_dir().unwrap_or_else(|error| {
            eprintln!("Windows console smoke cwd failed: {error}");
            std::process::exit(1);
        });
        let command = match exec_core::scripted_shell_kind() {
            ShellKind::PowerShell => "Write-Output AGENT_PIPE_OK; Start-Sleep -Milliseconds 750",
            ShellKind::Cmd => "echo AGENT_PIPE_OK & ping -n 2 127.0.0.1 >NUL",
            ShellKind::Posix => unreachable!(),
        };
        let result = runtime.block_on(async {
            let executor = exec_core::LocalExecutor;
            let ordinary = executor
                .exec_streaming(
                    command,
                    &cwd,
                    Duration::from_secs(10),
                    &CancellationToken::new(),
                    &|_, _| {},
                )
                .await?;
            let terminal = executor
                .exec_streaming_pty(
                    command,
                    &cwd,
                    Duration::from_secs(10),
                    &CancellationToken::new(),
                    &|_, _| {},
                )
                .await?;
            let permissions = computer_use::native_backend()
                .and_then(|backend| backend.permissions())
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((ordinary, terminal, permissions))
        });
        let receipt = match result {
            Ok((ordinary, terminal, permissions)) => json!({
                "status": "passed",
                "ordinary_exit_code": ordinary.code,
                "ordinary_output_seen": String::from_utf8_lossy(&ordinary.stdout)
                    .contains("AGENT_PIPE_OK"),
                "pty_exit_code": terminal.code,
                "pty_output_seen": String::from_utf8_lossy(&terminal.stdout)
                    .contains("AGENT_PIPE_OK"),
                "computer_use_permissions": permissions,
            }),
            Err(error) => json!({ "status": "failed", "error": error }),
        };
        if let Err(error) =
            std::fs::write(&output, serde_json::to_vec(&receipt).unwrap_or_default())
        {
            eprintln!("Windows console smoke receipt failed: {error}");
            std::process::exit(1);
        }
        if receipt["status"] != "passed" {
            std::process::exit(1);
        }
        true
    }

    #[cfg(not(windows))]
    {
        let _ = output;
        eprintln!("Windows console smoke is supported only on Windows");
        std::process::exit(2);
    }
}

fn signed_computer_use_smoke_requested(
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> bool {
    let mut arguments = arguments.into_iter();
    arguments
        .next()
        .is_some_and(|argument| argument.as_ref() == SIGNED_COMPUTER_USE_SMOKE_ARG)
        && arguments.next().is_none()
}

fn windows_console_smoke_output(
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> Option<std::path::PathBuf> {
    let mut arguments = arguments.into_iter();
    let first = arguments.next()?;
    if first.as_ref() != WINDOWS_CONSOLE_SMOKE_ARG {
        return None;
    }
    let output = arguments.next()?;
    if arguments.next().is_some() {
        return None;
    }
    Some(std::path::PathBuf::from(output.as_ref()))
}

/// Only a signed production flavor may consult or install production updates.
/// Development builds have a distinct bundle identity and must never replace
/// themselves with a release artifact from the production update channel.
#[tauri::command]
fn app_updates_enabled(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> bool {
    state
        .product
        .updates_enabled(cfg!(debug_assertions), &app.config().identifier)
}

/// Provider-neutral adapters shipped by the open foundation.
pub fn builtin_providers() -> Vec<ProviderInfo> {
    vec![ProviderInfo {
        id: "local".into(),
        label: "Local agent".into(),
        capabilities: ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            attachment_kinds: vec![
                agent_core::AttachmentKind::Text,
                agent_core::AttachmentKind::Image,
                agent_core::AttachmentKind::Pdf,
                agent_core::AttachmentKind::Docx,
            ],
            modes: Vec::new(),
            collaboration_modes: vec![CollaborationMode::Default, CollaborationMode::Plan],
        },
        internal: false,
    }]
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn run() {
    run_with_product_and_context(
        Arc::new(product::NeutralProduct),
        tauri::generate_context!(),
    );
}

pub fn run_with_product_and_context(
    product: Arc<dyn product::ProductIntegration>,
    context: tauri::Context<tauri::Wry>,
) {
    if let Err(error) = desktop_install_context::activate_macos_user_path() {
        eprintln!("failed to activate macOS user-tool PATH: {error}");
    }
    // Tauri sidecars live beside the main executable. Activate PATH-visible
    // helpers before tracing/Tauri/Tokio create worker threads so every child
    // surface (agent shell, background jobs, MCP, terminal) inherits one
    // deterministic toolchain.
    if let Err(error) = desktop_install_context::activate_bundled_path() {
        eprintln!("failed to activate bundled tool PATH: {error}");
    }

    // Keep the guard alive for the full Tauri process so the non-blocking file
    // writer flushes every accepted event before shutdown.
    let _diagnostics_guard = init_diagnostics();

    let mut builder = tauri::Builder::default();

    // Single-instance must be registered FIRST so a second launch (e.g. the OS
    // opening a a product deep link URL on Windows/Linux) is funneled into the already
    // running process rather than spawning a new window. Its `deep-link` feature
    // re-emits the URL, so the `on_open_url` handler below still fires; this
    // callback just raises the existing window. macOS routes deep links to the
    // running instance natively, so this is a belt-and-suspenders no-op there.
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }))
            .menu(updater_menu::build_menu)
            .on_menu_event(updater_menu::handle_menu_event);
    }

    let app = builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_google_auth::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(AppState::with_product(product))
        .manage(terminal::Terminals::default())
        .invoke_handler(tauri::generate_handler![
            commands::provider_list,
            commands::product_request,
            commands::provider_reconfigure,
            commands::ssh_probe,
            commands::ssh_list_directories,
            commands::remote_worker_connect,
            commands::external_agent_discover,
            commands::list_commands,
            commands::skills_list,
            commands::skills_reload,
            commands::skills_changes,
            commands::instructions_list,
            commands::skill_packs_list,
            commands::skill_pack_install,
            commands::skill_pack_uninstall,
            commands::computer_use_platform_status,
            commands::computer_use_request_permissions,
            commands::computer_use_approval_snapshot,
            commands::computer_use_revoke_approval,
            commands::computer_use_revoke_all_approvals,
            commands::computer_use_recent_receipts,
            commands::session_open::session_open,
            commands::session_close,
            commands::session_configure_cloud,
            app_updates_enabled,
            commands::update_begin_drain,
            commands::update_cancel_drain,
            commands::prompt,
            commands::compact,
            commands::steer,
            commands::cancel,
            commands::respond,
            commands::set_mode,
            commands::set_collaboration_mode,
            commands::set_output_style,
            sandbox_setup::local_sandbox_status,
            sandbox_setup::local_sandbox_setup,
            commands::side_question,
            diagnostics::capture_frontend_diagnostic,
            commands::prepare_quick_chat_workspace,
            commands::local_list_memory,
            commands::local_list_global_memory,
            commands::local_list_files,
            commands::local_list_security_scans,
            commands::project_context,
            commands::read_doc_text,
            commands::read_image_data_url,
            document_preview::render_document_preview,
            document_preview::read_document_preview_page,
            document_preview::cleanup_document_preview,
            commands::save_doc_text,
            file_actions::copy_local_file,
            markdown_export::export_markdown_pdf,
            security_report::export_security_scan_pdf,
            commands::open_path,
            project_worktree::project_branch_list,
            project_worktree::project_branch_switch,
            project_worktree::project_worktree_create,
            project_worktree::managed::project_worktree_transition_plan,
            project_worktree::managed::project_managed_worktree_create,
            project_worktree::managed::project_managed_worktree_list,
            project_worktree::managed::project_managed_worktree_cleanup,
            project_worktree::managed::project_managed_worktree_save_branch,
            commands::repository_inspect,
            commands::repository_discover,
            commands::repository_history,
            commands::changes_summary,
            commands::changes_diff,
            commands::changes_revert,
            commands::changes_release_checkpoints,
            commands::mcp_probe,
            commands::mcp_credentials_sync,
            commands::desktop_conv_list,
            commands::desktop_conv_get,
            commands::desktop_conv_put,
            commands::desktop_conv_delete,
            commands::desktop_conv_set_archived,
            commands::workspace_artifact_read,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
        ])
        .setup(|app| {
            let credential_root = app.path().app_data_dir()?.join("credentials");
            app.state::<AppState>()
                .credentials
                .configure(credential_root)
                .map_err(std::io::Error::other)?;

            // Tauri 2.11's config token generator currently emits a Vec for
            // `dataStoreIdentifier` even though WindowConfig requires
            // `[u8; 16]`. The QA config therefore suppresses automatic window
            // creation and this macOS-only path builds the same configured
            // window with the pinned persistent data store through the typed
            // builder API. Production and ordinary development windows remain
            // config-created and never enter this branch.
            #[cfg(target_os = "macos")]
            if app.get_webview_window("main").is_none() {
                let qa_window = app
                    .config()
                    .app
                    .windows
                    .first()
                    .filter(|window| !window.create)
                    .cloned();
                let data_store_identifier =
                    app.state::<AppState>().product.qa_data_store_identifier();
                if let (Some(config), Some(data_store_identifier)) =
                    (qa_window, data_store_identifier)
                {
                    tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                        .data_store_identifier(data_store_identifier)
                        .build()?;
                }
            }

            #[cfg(all(desktop, not(debug_assertions)))]
            {
                use tauri_plugin_autostart::ManagerExt;

                if let Err(error) = app.autolaunch().enable() {
                    tracing::warn!(%error, "failed to enable launch at login");
                }
            }

            // Product deep links may route back from a system-browser flow. Pull
            // the existing window to the foreground when the OS opens one.
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
            // Closing the window minimizes it instead of quitting. The main
            // webview disables background throttling so remote control, the local
            // agent loop, any in-flight run, and background sync keep running.
            // The app still quits via Cmd+Q or the app menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.minimize();
            }
        })
        .build(context)
        .expect("error while building desktop app");
    app.run(|app, event| {
        if !matches!(event, tauri::RunEvent::Exit) {
            return;
        }
        let state = app.state::<AppState>().inner().clone();
        let receipt = tauri::async_runtime::block_on(state.runtime_registry.shutdown_all());
        let terminals = app.state::<terminal::Terminals>().shutdown_all();
        tracing::info!(
            event = "runtime_registry_shutdown",
            sessions = receipt.sessions,
            workers = receipt.workers,
            skill_catalogs = receipt.skill_catalogs,
            terminals,
            "native runtime shutdown complete"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::{
        signed_computer_use_smoke_requested, windows_console_smoke_output,
        SIGNED_COMPUTER_USE_SMOKE_ARG, WINDOWS_CONSOLE_SMOKE_ARG,
    };
    use serde_json::Value;

    #[test]
    fn webview_cannot_invoke_raw_google_token_commands() {
        let capability: Value = serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("default capability");
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(!permissions.iter().any(|permission| {
            permission
                .as_str()
                .is_some_and(|name| name.starts_with("google-auth:"))
        }));
    }

    #[test]
    fn neutral_bundle_has_no_release_authority() {
        let development: Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("development config");
        assert_eq!(development["productName"], "Clark Code Dev");
        assert_eq!(development["identifier"], "dev.clarkcode.desktop");
        assert_eq!(development["bundle"]["createUpdaterArtifacts"], false);
        assert!(include_str!("../Info.plist").contains("NSDocumentsFolderUsageDescription"));
    }

    #[test]
    fn signed_computer_use_smoke_requires_the_exact_standalone_argument() {
        assert!(signed_computer_use_smoke_requested([
            SIGNED_COMPUTER_USE_SMOKE_ARG
        ]));
        assert!(!signed_computer_use_smoke_requested(
            std::iter::empty::<&str>()
        ));
        assert!(!signed_computer_use_smoke_requested([
            "--not-the-smoke-flag"
        ]));
        assert!(!signed_computer_use_smoke_requested([
            SIGNED_COMPUTER_USE_SMOKE_ARG,
            "unexpected-extra-argument",
        ]));
    }

    #[test]
    fn windows_console_smoke_requires_one_explicit_output_path() {
        assert_eq!(
            windows_console_smoke_output([
                WINDOWS_CONSOLE_SMOKE_ARG,
                r"C:\Users\Public\console-smoke.json",
            ]),
            Some(std::path::PathBuf::from(
                r"C:\Users\Public\console-smoke.json",
            )),
        );
        assert_eq!(
            windows_console_smoke_output([WINDOWS_CONSOLE_SMOKE_ARG]),
            None,
        );
        assert_eq!(
            windows_console_smoke_output(
                [WINDOWS_CONSOLE_SMOKE_ARG, "receipt.json", "unexpected",]
            ),
            None,
        );
    }
}
