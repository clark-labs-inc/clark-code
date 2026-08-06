use tauri::{
    menu::{Menu, MenuEvent, MenuItem},
    AppHandle, Emitter, Runtime,
};

#[cfg(target_os = "macos")]
use tauri::menu::MenuItemKind;
#[cfg(not(target_os = "macos"))]
use tauri::menu::Submenu;

const CHECK_FOR_UPDATES_ID: &str = "check-for-updates";
const CHECK_FOR_UPDATES_TEXT: &str = "Check for Updates…";
const SETTINGS_ID: &str = "open-settings";
const SETTINGS_TEXT: &str = "Settings…";
const SETTINGS_ACCELERATOR: &str = "CmdOrCtrl+,";

const UPDATE_MENU_EVENT: &str = "update-menu-requested";
const SETTINGS_MENU_EVENT: &str = "settings-menu-requested";

fn event_name(id: &str) -> Option<&'static str> {
    match id {
        CHECK_FOR_UPDATES_ID => Some(UPDATE_MENU_EVENT),
        SETTINGS_ID => Some(SETTINGS_MENU_EVENT),
        _ => None,
    }
}

pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    let update_item = MenuItem::with_id(
        app,
        CHECK_FOR_UPDATES_ID,
        CHECK_FOR_UPDATES_TEXT,
        true,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(
        app,
        SETTINGS_ID,
        SETTINGS_TEXT,
        true,
        Some(SETTINGS_ACCELERATOR),
    )?;

    #[cfg(target_os = "macos")]
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        // Keep the native application-menu order familiar: About, update,
        // Settings, then the default separator and Services/Hide/Quit items.
        app_menu.insert(&update_item, 1)?;
        app_menu.insert(&settings_item, 2)?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Windows and Linux do not get Tauri's macOS application submenu, so
        // add the same Clark Code entry explicitly instead of leaving these
        // actions platform-exclusive.
        let app_menu = Submenu::with_items(
            app,
            app.package_info().name.clone(),
            true,
            &[&settings_item, &update_item],
        )?;
        menu.prepend(&app_menu)?;
    }

    Ok(menu)
}

pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let Some(event_name) = event_name(event.id().as_ref()) else {
        return;
    };

    // Native menu items only express intent. The frontend owns Settings state
    // and the updater's download/drain coordinator, including Windows' forced
    // exit during install.
    let _ = app.emit(event_name, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_only_known_native_menu_actions() {
        assert_eq!(event_name(CHECK_FOR_UPDATES_ID), Some(UPDATE_MENU_EVENT));
        assert_eq!(event_name(SETTINGS_ID), Some(SETTINGS_MENU_EVENT));
        assert_eq!(event_name("quit"), None);
    }
}
