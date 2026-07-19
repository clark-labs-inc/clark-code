use tauri::{
    menu::{Menu, MenuEvent, MenuItem, MenuItemKind},
    AppHandle, Emitter, Runtime,
};

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

    // Route the native menu through the same frontend-owned download/drain
    // coordinator as every visible update button. This prevents the menu from
    // bypassing active-session safety (and Windows' install-time forced exit).
    let _ = app.emit("update-menu-requested", ());
}
