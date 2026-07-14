use tauri::{
    menu::{Menu, MenuEvent, MenuItem, MenuItemKind},
    AppHandle, Runtime,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

const CHECK_FOR_UPDATES_ID: &str = "check-for-updates";
const CHECK_FOR_UPDATES_TEXT: &str = "Check for Updates…";

pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;

    #[cfg(target_os = "macos")]
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        let item = MenuItem::with_id(
            app,
            CHECK_FOR_UPDATES_ID,
            CHECK_FOR_UPDATES_TEXT,
            true,
            None::<&str>,
        )?;
        app_menu.insert(&item, 1)?;
    }

    Ok(menu)
}

pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() != CHECK_FOR_UPDATES_ID {
        return;
    }

    let Some(item) = update_menu_item(app) else {
        return;
    };
    let _ = item.set_enabled(false);
    let _ = item.set_text("Checking for Updates…");

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(error) => {
                show_error(&app, &error.to_string());
                reset_item(&item);
                return;
            }
        };
        let update = match updater.check().await {
            Ok(Some(update)) => update,
            Ok(None) => {
                app.dialog()
                    .message("Clark Code is already up to date.")
                    .title("No Updates Available")
                    .show(|_| {});
                reset_item(&item);
                return;
            }
            Err(error) => {
                show_error(&app, &error.to_string());
                reset_item(&item);
                return;
            }
        };

        let version = update.version.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .message(format!(
                "Clark Code {version} is available. Download, install, and restart now?"
            ))
            .title("Update Available")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Update and Restart".into(),
                "Cancel".into(),
            ))
            .show(move |confirmed| {
                let _ = sender.send(confirmed);
            });

        if !receiver.await.unwrap_or(false) {
            reset_item(&item);
            return;
        }

        let _ = item.set_text(format!("Downloading Clark Code {version}…"));
        if let Err(error) = update.download_and_install(|_, _| {}, || {}).await {
            show_error(&app, &error.to_string());
            reset_item(&item);
            return;
        }
        app.restart();
    });
}

fn update_menu_item<R: Runtime>(app: &AppHandle<R>) -> Option<MenuItem<R>> {
    let menu = app.menu()?;
    let MenuItemKind::Submenu(app_menu) = menu.items().ok()?.into_iter().next()? else {
        return None;
    };
    let MenuItemKind::MenuItem(item) = app_menu.get(CHECK_FOR_UPDATES_ID)? else {
        return None;
    };
    Some(item)
}

fn reset_item<R: Runtime>(item: &MenuItem<R>) {
    let _ = item.set_text(CHECK_FOR_UPDATES_TEXT);
    let _ = item.set_enabled(true);
}

fn show_error<R: Runtime>(app: &AppHandle<R>, error: &str) {
    app.dialog()
        .message(format!(
            "Clark Code could not complete the update.\n\n{error}"
        ))
        .title("Update Failed")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}
