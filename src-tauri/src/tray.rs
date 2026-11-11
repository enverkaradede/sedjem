//! Menu bar (tray) icon: the always-available entry point while Sedjem runs
//! in the background without a Dock icon.

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::state::{events, AppState};
use crate::windows;

const TRAY_ID: &str = "sedjem-tray";
const ITEM_TOGGLE_PANEL: &str = "toggle-panel";
const ITEM_SETTINGS: &str = "settings";
const ITEM_UPDATES: &str = "updates";
const ITEM_QUIT: &str = "quit";
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray-template.png");

fn toggle_panel<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let snapshot = {
        let Ok(mut settings) = state.settings.write() else {
            return;
        };
        settings.general.panel_visible = !settings.general.panel_visible;
        settings.clone()
    };
    if let Err(err) = windows::set_panel_visible(app, snapshot.general.panel_visible) {
        tracing::warn!(%err, "could not toggle panel");
    }
    if let Err(err) = state.store.save(&snapshot) {
        tracing::warn!(%err, "could not persist panel visibility");
    }
    windows::broadcast_settings(app, &snapshot);
}

/// Opens the Updates section and starts a check in the background.
fn check_updates<R: Runtime>(app: &AppHandle<R>) {
    windows::show_settings(app);
    if let Err(err) = app.emit(events::NAVIGATE, "updates") {
        tracing::warn!(%err, "could not navigate to updates");
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<AppState>();
        // Failures are already reflected in the update state shown in the UI.
        let _ = state.updates.check(&handle).await;
    });
}

fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        ITEM_TOGGLE_PANEL => toggle_panel(app),
        ITEM_SETTINGS => windows::show_settings(app),
        ITEM_UPDATES => check_updates(app),
        ITEM_QUIT => app.exit(0),
        _ => {}
    }
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(
        app,
        ITEM_TOGGLE_PANEL,
        "Paneli Göster / Gizle",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, ITEM_SETTINGS, "Ayarlar…", true, Some("CmdOrCtrl+,"))?;
    let updates = MenuItem::with_id(
        app,
        ITEM_UPDATES,
        "Güncellemeleri Denetle…",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, ITEM_QUIT, "Sedjem'den Çık", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(app, &[&toggle, &settings, &updates, &separator, &quit])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(true)
        .tooltip("Sedjem")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    Ok(())
}
