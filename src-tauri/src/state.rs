//! Process-wide application state and the Tauri event bridge.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

use tauri::{AppHandle, Emitter};

use crate::dictation::{Dictation, DictationEvents, ResultPayload, StatePayload};
use crate::engine::manager::{EngineConfig, EngineManager};
use crate::error::AppError;
use crate::settings::{Settings, SettingsStore};
use crate::updates::Updates;

pub mod events {
    pub const STATE: &str = "dictation:state";
    pub const LEVEL: &str = "dictation:level";
    pub const PARTIAL: &str = "dictation:partial";
    pub const RESULT: &str = "dictation:result";
    pub const NO_SPEECH: &str = "dictation:no-speech";
    pub const BUSY: &str = "dictation:busy";
    pub const ERROR: &str = "dictation:error";
    pub const ENGINE_STATUS: &str = "engine:status";
    pub const MODEL_DOWNLOAD: &str = "model:download";
    pub const MLX_SETUP: &str = "mlx:setup";
    pub const SETTINGS_CHANGED: &str = "settings:changed";
    pub const UPDATE_STATE: &str = "update:state";
    /// Asks the settings window to show a section (e.g. from the tray).
    pub const NAVIGATE: &str = "ui:navigate";
}

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub store: SettingsStore,
    pub engines: Arc<EngineManager>,
    pub dictation: Arc<Dictation>,
    pub updates: Updates,
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub mlx_setup_running: AtomicBool,
    /// Bumped whenever the Keychain API key changes, forcing an engine rebuild.
    pub secret_epoch: AtomicU64,
    /// Debounce token for persisting the panel position while dragging.
    pub panel_move_generation: AtomicU64,
    pub home: PathBuf,
    /// Problems found during startup, shown once by the settings window.
    pub startup_notices: Mutex<Vec<String>>,
}

impl AppState {
    pub fn settings_snapshot(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn engine_config(&self, settings: &Settings) -> EngineConfig {
        EngineConfig::from_settings(
            &settings.transcription,
            self.engines.env(),
            self.secret_epoch.load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    pub fn push_notice(&self, notice: impl Into<String>) {
        if let Ok(mut notices) = self.startup_notices.lock() {
            notices.push(notice.into());
        }
    }
}

/// Forwards dictation events to every webview window.
pub struct TauriEvents {
    pub app: AppHandle,
}

impl TauriEvents {
    fn emit<S: serde::Serialize + Clone>(&self, event: &str, payload: S) {
        if let Err(err) = self.app.emit(event, payload) {
            tracing::warn!(%err, event, "event emit failed");
        }
    }
}

impl DictationEvents for TauriEvents {
    fn state(&self, payload: &StatePayload) {
        self.emit(events::STATE, payload.clone());
    }
    fn level(&self, value: f32) {
        self.emit(events::LEVEL, value);
    }
    fn partial(&self, text: &str) {
        self.emit(events::PARTIAL, text.to_owned());
    }
    fn result(&self, payload: &ResultPayload) {
        self.emit(events::RESULT, payload.clone());
    }
    fn no_speech(&self) {
        self.emit(events::NO_SPEECH, ());
    }
    fn busy(&self) {
        self.emit(events::BUSY, ());
    }
    fn error(&self, error: &AppError) {
        tracing::warn!(code = ?error.code, message = %error.message, "dictation error");
        self.emit(events::ERROR, error.clone());
    }
}
