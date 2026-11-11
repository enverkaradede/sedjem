//! Floating panel and settings window management.
//!
//! On macOS the panel window is converted into a non-activating `NSPanel`:
//! clicking it never steals focus from the app the user is dictating into,
//! which is what makes paste-into-focused-app work at all.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::{
    AppHandle, LogicalPosition, Manager, PhysicalPosition, Runtime, WebviewWindow, WindowEvent,
};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::settings::PanelPosition;
use crate::state::{events, AppState};

pub const PANEL_LABEL: &str = "panel";
pub const SETTINGS_LABEL: &str = "settings";
/// Logical size of the panel window (button plus glow margin).
const PANEL_SIZE: f64 = 112.0;
/// Default distance from the bottom of the screen.
const PANEL_BOTTOM_MARGIN: f64 = 140.0;
/// Persist the dragged position once the user has stopped moving it.
const MOVE_SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

#[cfg(target_os = "macos")]
mod mac {
    use tauri_nspanel::{
        tauri_panel, CollectionBehavior, PanelLevel, StyleMask, TrackingAreaOptions,
    };

    tauri_panel! {
        panel!(SedjemPanel {
            config: {
                can_become_key_window: false,
                can_become_main_window: false,
                is_floating_panel: true
            }
            with: {
                // Hover states must work while another app is active.
                tracking_area: {
                    options: TrackingAreaOptions::new()
                        .active_always()
                        .mouse_entered_and_exited()
                        .mouse_moved(),
                    auto_resize: true
                }
            }
        })

        // Only the built-in mouse callbacks are used; no delegate methods.
        panel_event!(SedjemPanelEvents {})
    }

    pub fn convert<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> Result<(), String> {
        use tauri_nspanel::WebviewWindowExt;
        let panel = window
            .to_panel::<SedjemPanel<R>>()
            .map_err(|err| err.to_string())?;
        // Above other always-on-top windows, but below menus and alerts.
        panel.set_level(PanelLevel::Status.value());
        panel
            .add_style_mask(StyleMask::empty().nonactivating_panel().into())
            .map_err(|err| err.to_string())?;
        // Visible on every Space and over full-screen apps; not in ⌘` cycling.
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .ignores_cycle()
                .into(),
        );
        // NSPanel hides itself when its app deactivates by default.
        panel.set_hides_on_deactivate(false);
        forward_hover(window, &panel);
        Ok(())
    }

    /// The webview gets no mouseenter/mousemove while the app is inactive (the
    /// normal case for a non-activating panel), so hover-only controls never
    /// appeared. The panel's always-active tracking area still sees the
    /// pointer; forward enter/exit to the panel webview as `panel:hover`.
    fn forward_hover<R: tauri::Runtime>(
        window: &tauri::WebviewWindow<R>,
        panel: &tauri_nspanel::PanelHandle<R>,
    ) {
        use tauri::{Emitter, Manager};
        use tauri_nspanel::objc2_app_kit::NSEvent;

        let notify = |hovered: bool| {
            let app = window.app_handle().clone();
            move |_event: &NSEvent| {
                if let Err(err) = app.emit_to(super::PANEL_LABEL, super::events::PANEL_HOVER, hovered) {
                    tracing::debug!(%err, "could not emit panel hover");
                }
            }
        };
        let handler = SedjemPanelEvents::new();
        handler.on_mouse_entered(notify(true));
        handler.on_mouse_exited(notify(false));
        panel.set_event_handler(Some(handler.as_ref()));
        // AppKit holds window delegates weakly; the panel lives as long as the
        // app, so its delegate is intentionally kept alive for the same span.
        std::mem::forget(handler);
    }
}

fn default_position<R: Runtime>(window: &WebviewWindow<R>) -> Option<LogicalPosition<f64>> {
    let monitor = window.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let size = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    Some(LogicalPosition::new(
        origin.x + (size.width - PANEL_SIZE) / 2.0,
        origin.y + size.height - PANEL_SIZE - PANEL_BOTTOM_MARGIN,
    ))
}

/// True when the saved position still lies on a connected monitor (it may
/// not after unplugging an external display).
fn position_is_visible<R: Runtime>(window: &WebviewWindow<R>, pos: PanelPosition) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let origin = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        let center_x = pos.x + PANEL_SIZE / 2.0;
        let center_y = pos.y + PANEL_SIZE / 2.0;
        center_x >= origin.x
            && center_x <= origin.x + size.width
            && center_y >= origin.y
            && center_y <= origin.y + size.height
    })
}

fn schedule_position_save<R: Runtime>(
    app: &AppHandle<R>,
    physical: PhysicalPosition<i32>,
    scale: f64,
) {
    let state = app.state::<AppState>();
    let generation = state.panel_move_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let logical = physical.to_logical::<f64>(scale);
    let app = app.clone();
    let spawned = thread::Builder::new()
        .name("sedjem-panel-pos".into())
        .spawn(move || {
            thread::sleep(MOVE_SAVE_DEBOUNCE);
            let state = app.state::<AppState>();
            if state.panel_move_generation.load(Ordering::SeqCst) != generation {
                return; // still moving; a later event will save
            }
            let snapshot = {
                let Ok(mut settings) = state.settings.write() else {
                    return;
                };
                settings.general.panel_position = Some(PanelPosition {
                    x: logical.x,
                    y: logical.y,
                });
                settings.clone()
            };
            if let Err(err) = state.store.save(&snapshot) {
                tracing::warn!(%err, "could not persist panel position");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not schedule panel position save");
    }
}

/// Positions the panel, converts it to an NSPanel and wires position saving.
pub fn setup_panel<R: Runtime>(
    app: &AppHandle<R>,
    saved: Option<PanelPosition>,
    visible: bool,
) -> AppResult<()> {
    let window = app
        .get_webview_window(PANEL_LABEL)
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "panel penceresi yok"))?;
    let position = saved
        .filter(|p| position_is_visible(&window, *p))
        .map(|p| LogicalPosition::new(p.x, p.y))
        .or_else(|| default_position(&window));
    if let Some(position) = position {
        if let Err(err) = window.set_position(position) {
            tracing::warn!(%err, "could not position panel");
        }
    }

    #[cfg(target_os = "macos")]
    mac::convert(&window).map_err(|err| AppError::new(ErrorCode::Internal, err))?;

    let handle = app.clone();
    let observed = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(physical) = event {
            let scale = observed.scale_factor().unwrap_or(1.0);
            schedule_position_save(&handle, *physical, scale);
        }
    });
    set_panel_visible(app, visible)
}

pub fn set_panel_visible<R: Runtime>(app: &AppHandle<R>, visible: bool) -> AppResult<()> {
    let window = app
        .get_webview_window(PANEL_LABEL)
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "panel penceresi yok"))?;
    // show() on a non-activating panel orders it front without activating.
    let result = if visible {
        window.show()
    } else {
        window.hide()
    };
    result.map_err(|err| AppError::new(ErrorCode::Internal, err.to_string()))
}

pub fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(SETTINGS_LABEL) else {
        tracing::error!("settings window missing");
        return;
    };
    if let Err(err) = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus())
    {
        tracing::warn!(%err, "could not show settings window");
    }
}

/// Closing the settings window only hides it: Sedjem keeps running in the
/// background (panel, shortcut and tray stay alive).
pub fn keep_settings_alive<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(SETTINGS_LABEL) else {
        return;
    };
    let observed = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(err) = observed.hide() {
                tracing::warn!(%err, "could not hide settings window");
            }
        }
    });
}

/// Applies Dock visibility (Regular vs Accessory activation policy).
#[cfg(target_os = "macos")]
pub fn apply_dock_visibility<R: Runtime>(app: &AppHandle<R>, show_in_dock: bool) {
    let policy = if show_in_dock {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    if let Err(err) = app.set_activation_policy(policy) {
        tracing::warn!(%err, "could not change activation policy");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_dock_visibility<R: Runtime>(_app: &AppHandle<R>, _show_in_dock: bool) {}

/// Tells every window the settings changed (e.g. the tray toggled the panel).
pub fn broadcast_settings<R: Runtime>(app: &AppHandle<R>, settings: &crate::settings::Settings) {
    use tauri::Emitter;
    if let Err(err) = app.emit(events::SETTINGS_CHANGED, settings.clone()) {
        tracing::warn!(%err, "could not broadcast settings");
    }
}
