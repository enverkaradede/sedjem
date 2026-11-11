//! Global shortcut: maps key down/up of the configured accelerator onto the
//! same press/release inputs the floating button produces.

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::controller::InputSource;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::settings::ShortcutSettings;
use crate::state::AppState;

/// Plugin handler; registered once in `lib.rs`.
pub fn on_shortcut<R: Runtime>(app: &AppHandle<R>, _shortcut: &Shortcut, event: ShortcutEvent) {
    let state = app.state::<AppState>();
    // Enqueue only: ordering and timestamps are preserved by the input queue.
    match event.state {
        ShortcutState::Pressed => state.dictation.press(InputSource::Hotkey),
        ShortcutState::Released => state.dictation.release(InputSource::Hotkey),
    }
}

pub fn parse(accelerator: &str) -> AppResult<Shortcut> {
    accelerator
        .trim()
        .parse::<Shortcut>()
        .map_err(|err| AppError::new(ErrorCode::Shortcut, format!("Kısayol tanınmadı: {err}")))
}

/// Replaces the registered shortcut with the one in `settings`.
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &ShortcutSettings) -> AppResult<()> {
    let manager = app.global_shortcut();
    manager
        .unregister_all()
        .map_err(|err| AppError::new(ErrorCode::Shortcut, err.to_string()))?;
    if !settings.enabled {
        return Ok(());
    }
    let shortcut = parse(&settings.accelerator)?;
    manager.register(shortcut).map_err(|err| {
        AppError::new(
            ErrorCode::Shortcut,
            format!("Kısayol kaydedilemedi (başka bir uygulama kullanıyor olabilir): {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_accelerators() {
        for accelerator in [
            "Alt+Space",
            "Control+Shift+KeyD",
            "Super+Alt+KeyK",
            "F5",
            "CmdOrCtrl+Shift+Space",
        ] {
            assert!(parse(accelerator).is_ok(), "{accelerator}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("Alt+NotAKey").is_err());
        assert!(parse("").is_err());
    }
}
