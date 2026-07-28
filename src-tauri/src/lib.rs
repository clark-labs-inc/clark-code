//! Clark Desktop — Tauri host.
//!
//! The host owns the native `agent-core` engine: it holds provider instances,
//! drives transports/sidecars, and bridges to the web UI via Tauri commands
//! (`invoke`) and events (`emit`): provider discovery, the command surface, the
//! live ACP provider, and snapshot streaming.

use agent_core::{CollaborationMode, ProviderCapabilities};
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

mod commands;
mod document_preview;
mod file_actions;
mod markdown_export;
mod mobile_remote;
mod project_context;
mod project_worktree;
mod sandbox_setup;
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

/// Metadata for a provider the UI can offer. Mirrors the frontend `ProviderInfo`.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    pub capabilities: ProviderCapabilities,
}

const PRODUCTION_BUNDLE_IDENTIFIER: &str = "com.clark.desktop";
const SIGNED_COMPUTER_USE_SMOKE_ARG: &str = "--computer-use-signed-smoke";
const WINDOWS_CONSOLE_SMOKE_ARG: &str = "--windows-console-smoke";

fn updates_enabled_for(debug_build: bool, identifier: &str) -> bool {
    !debug_build && identifier == PRODUCTION_BUNDLE_IDENTIFIER
}

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
            ShellKind::PowerShell => "Write-Output CLARK_PIPE_OK; Start-Sleep -Milliseconds 750",
            ShellKind::Cmd => "echo CLARK_PIPE_OK & ping -n 2 127.0.0.1 >NUL",
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
                    .contains("CLARK_PIPE_OK"),
                "pty_exit_code": terminal.code,
                "pty_output_seen": String::from_utf8_lossy(&terminal.stdout)
                    .contains("CLARK_PIPE_OK"),
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
fn app_updates_enabled(app: tauri::AppHandle) -> bool {
    updates_enabled_for(cfg!(debug_assertions), &app.config().identifier)
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
            collaboration_modes: vec![CollaborationMode::Default, CollaborationMode::Plan],
        },
    }]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tauri sidecars live beside the main executable. Activate PATH-visible
    // helpers before tracing/Tauri/Tokio create worker threads so every child
    // surface (agent shell, background jobs, MCP, terminal) inherits one
    // deterministic toolchain.
    if let Err(error) = clark_install_context::activate_bundled_path() {
        eprintln!("failed to activate bundled tool PATH: {error}");
    }

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
            commands::session_new,
            commands::session_load,
            commands::session_close,
            commands::session_configure_cloud,
            commands::clark_clear_cloud_session,
            commands::clark_refresh_cloud_session,
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
            commands::local_list_memory,
            commands::local_list_global_memory,
            commands::local_list_files,
            commands::project_context,
            commands::read_doc_text,
            commands::read_image_data_url,
            document_preview::render_document_preview,
            document_preview::read_document_preview_page,
            document_preview::cleanup_document_preview,
            commands::save_doc_text,
            file_actions::copy_local_file,
            markdown_export::export_markdown_pdf,
            commands::open_path,
            project_worktree::project_branch_list,
            project_worktree::project_branch_switch,
            project_worktree::project_worktree_create,
            commands::clark_exchange_google_idtoken,
            commands::clark_provision_code_key,
            commands::clark_billing_me,
            commands::clark_repository_inspect,
            commands::clark_repository_discover,
            commands::clark_repository_history,
            commands::changes_summary,
            commands::changes_diff,
            commands::changes_revert,
            commands::changes_release_checkpoints,
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
            mobile_remote::desktop_code_attachment_download,
            mobile_remote::desktop_code_repository_sync,
            mobile_remote::desktop_organization_knowledge_status,
            mobile_remote::desktop_organization_repository_sync,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
        ])
        .setup(|app| {
            #[cfg(all(desktop, not(debug_assertions)))]
            {
                use tauri_plugin_autostart::ManagerExt;

                if let Err(error) = app.autolaunch().enable() {
                    tracing::warn!(%error, "failed to enable launch at login");
                }
            }

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
            // Closing the window minimizes it instead of quitting. The main
            // webview disables background throttling so remote control, the local
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

#[cfg(test)]
mod tests {
    use super::{
        signed_computer_use_smoke_requested, updates_enabled_for, windows_console_smoke_output,
        SIGNED_COMPUTER_USE_SMOKE_ARG, WINDOWS_CONSOLE_SMOKE_ARG,
    };
    use serde_json::Value;

    #[test]
    fn updater_is_limited_to_non_debug_production_flavors() {
        assert!(updates_enabled_for(false, "com.clark.desktop"));
        assert!(!updates_enabled_for(true, "com.clark.desktop"));
        assert!(!updates_enabled_for(false, "com.clark.desktop.dev"));
    }

    #[test]
    fn development_and_release_bundle_identities_are_distinct() {
        let development: Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("development config");
        let release: Value = serde_json::from_str(include_str!("../tauri.release.conf.json"))
            .expect("release config");

        assert_eq!(development["productName"], "Clark Code Dev");
        assert_eq!(development["identifier"], "com.clark.desktop.dev");
        assert_eq!(development["bundle"]["createUpdaterArtifacts"], false);
        assert_eq!(release["productName"], "Clark Code");
        assert_eq!(release["identifier"], "com.clark.desktop");
        assert_eq!(release["bundle"]["createUpdaterArtifacts"], true);
        assert_eq!(
            release["bundle"]["windows"]["nsis"]["installerHooks"],
            "./windows/preserve-sandbox-state.nsh",
        );
        let windows_hooks = include_str!("../windows/preserve-sandbox-state.nsh");
        assert!(windows_hooks.contains("NSIS_HOOK_PREINSTALL"));
        assert!(windows_hooks.contains("NSIS_HOOK_PREUNINSTALL"));
        assert!(windows_hooks.contains(r"$LOCALAPPDATA\Clark Code\sandbox"));
        assert!(windows_hooks.contains(r"$LOCALAPPDATA\Clark\Code\sandbox"));
        let windows_signing: Value =
            serde_json::from_str(include_str!("../tauri.windows-signing.conf.json"))
                .expect("Windows signing config");
        let sign_command = &windows_signing["bundle"]["windows"]["signCommand"];
        assert_eq!(sign_command["cmd"], "powershell.exe");
        assert_eq!(
            sign_command["args"],
            serde_json::json!([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "sign-windows-artifact.ps1",
                "-FilePath",
                "%1",
            ])
        );
        let signing_script = include_str!("../sign-windows-artifact.ps1");
        assert!(signing_script.contains("http://timestamp.acs.microsoft.com"));
        assert!(signing_script.contains("/dlib"));
        assert!(signing_script.contains("/dmdf"));
        assert!(signing_script.contains("verify /v /pa /all"));
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

    #[test]
    fn native_computer_use_fixture_is_debug_only_and_never_bundled() {
        let fixture_info = include_str!("../../harness/fixtures/computer-use-native/Info.plist");
        assert!(fixture_info.contains("<key>ClarkDebugOnlyFixture</key>"));
        for production_config in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.release.conf.json"),
            include_str!("../tauri.computer-use.macos.conf.json"),
        ] {
            assert!(!production_config.contains("computer-use-fixture"));
            assert!(!production_config.contains("Clark Computer Use Fixture"));
        }
    }
}
