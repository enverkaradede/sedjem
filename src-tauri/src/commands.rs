//! IPC commands exposed to the webviews. Every command validates its input
//! and returns `AppResult`; blocking work runs on the blocking thread pool so
//! the main (UI) thread never stalls.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;

use crate::audio::capture::{self, CaptureConfig, Recording};
use crate::controller::InputSource;
use crate::dictation::{HistoryEntry, StatePayload};
use crate::engine::manager::EngineStatus;
use crate::engine::mlx_env::{self, MlxModel, PythonCheck};
use crate::engine::models::{self, ModelStatus};
use crate::engine::wyoming::WyomingEngine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::platform::{self, MicrophoneStatus, PrivacyPane};
use crate::settings::{validate_base_url, Settings};
use crate::state::{events, AppState};
use crate::updates::UpdateInfo;
use crate::{hotkey, windows};

const MAX_CLIPBOARD_CHARS: usize = 100_000;
const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const MIC_PROBE_DURATION: Duration = Duration::from_millis(400);
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

fn internal(err: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Internal, err.to_string())
}

/// Runs blocking work off the main thread.
async fn blocking<T, F>(work: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(internal)?
}

// ------------------------------------------------------------- settings --

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings_snapshot()
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    let autolaunch = app.autolaunch();
    let current = autolaunch.is_enabled().unwrap_or(!enabled);
    if current == enabled {
        return Ok(());
    }
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    result.map_err(|err| {
        AppError::new(
            ErrorCode::Autostart,
            format!("Başlangıçta çalıştırma ayarlanamadı: {err}"),
        )
    })
}

/// Validates, applies side effects that can fail (shortcut, autostart) and
/// only then persists. On failure nothing is saved and prior state restored.
#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> AppResult<Settings> {
    blocking(move || {
        settings.validate()?;
        let state = app.state::<AppState>();
        let previous = state.settings_snapshot();
        let mut next = settings;
        // The panel position is owned by the drag handler, not the form.
        next.general.panel_position = previous.general.panel_position;

        if next.shortcut != previous.shortcut {
            hotkey::apply(&app, &next.shortcut)?;
        }
        if next.general.launch_at_login != previous.general.launch_at_login {
            if let Err(err) = apply_autostart(&app, next.general.launch_at_login) {
                if next.shortcut != previous.shortcut {
                    if let Err(restore_err) = hotkey::apply(&app, &previous.shortcut) {
                        tracing::error!(%restore_err, "could not restore previous shortcut");
                    }
                }
                return Err(err);
            }
        }
        state.store.save(&next)?;
        if let Ok(mut current) = state.settings.write() {
            *current = next.clone();
        }

        if next.general.show_in_dock != previous.general.show_in_dock {
            windows::apply_dock_visibility(&app, next.general.show_in_dock);
        }
        if next.general.panel_visible != previous.general.panel_visible {
            windows::set_panel_visible(&app, next.general.panel_visible)?;
        }
        state.engines.apply(state.engine_config(&next), false);
        windows::broadcast_settings(&app, &next);
        Ok(next)
    })
    .await
}

#[tauri::command]
pub fn take_startup_notices(state: State<'_, AppState>) -> Vec<String> {
    state
        .startup_notices
        .lock()
        .map(|mut n| std::mem::take(&mut *n))
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub app_data_dir: String,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        app_data_dir: state.engines.env().app_data.to_string_lossy().into_owned(),
    }
}

// ------------------------------------------------------------ dictation --

/// Press/release/cancel only enqueue (microseconds); the dictation input
/// thread does the work, in order.
#[tauri::command]
pub fn panel_press(state: State<'_, AppState>) {
    state.dictation.press(InputSource::Panel);
}

#[tauri::command]
pub fn panel_release(state: State<'_, AppState>) {
    state.dictation.release(InputSource::Panel);
}

#[tauri::command]
pub fn cancel_dictation(state: State<'_, AppState>) {
    state.dictation.cancel();
}

#[tauri::command]
pub fn get_dictation_state(state: State<'_, AppState>) -> StatePayload {
    state.dictation.current_state()
}

#[tauri::command]
pub fn get_history(state: State<'_, AppState>) -> Vec<HistoryEntry> {
    state.dictation.history()
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) {
    state.dictation.clear_history();
}

#[tauri::command]
pub fn copy_to_clipboard(text: String) -> AppResult<()> {
    if text.chars().count() > MAX_CLIPBOARD_CHARS {
        return Err(AppError::new(ErrorCode::InvalidSettings, "Metin çok uzun."));
    }
    platform::copy_text(&text).map_err(|err| AppError::new(ErrorCode::Inject, err))
}

// --------------------------------------------------------------- engine --

#[tauri::command]
pub fn get_engine_status(state: State<'_, AppState>) -> EngineStatus {
    state.engines.status()
}

#[tauri::command]
pub fn reload_engine(state: State<'_, AppState>) {
    let settings = state.settings_snapshot();
    state.engines.apply(state.engine_config(&settings), true);
}

#[tauri::command]
pub fn list_ggml_models(state: State<'_, AppState>) -> Vec<ModelStatus> {
    models::list(&state.engines.env().app_data)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum DownloadEvent {
    Progress {
        id: String,
        downloaded: u64,
        total: u64,
    },
    Done {
        id: String,
        path: String,
    },
    Failed {
        id: String,
        message: String,
    },
    Cancelled {
        id: String,
    },
}

#[tauri::command]
pub fn download_ggml_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    if models::find(&id).is_none() {
        return Err(AppError::new(ErrorCode::Download, "Bilinmeyen model."));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = state.downloads.lock().map_err(internal)?;
        if downloads.contains_key(&id) {
            return Err(AppError::new(
                ErrorCode::Download,
                "Bu model zaten indiriliyor.",
            ));
        }
        downloads.insert(id.clone(), cancel.clone());
    }
    let app_data = state.engines.env().app_data.clone();
    std::thread::Builder::new()
        .name("sedjem-download".into())
        .spawn(move || {
            let mut last_emit = Instant::now() - DOWNLOAD_PROGRESS_INTERVAL;
            let result = models::download(&id, &app_data, &cancel, &mut |downloaded, total| {
                if last_emit.elapsed() >= DOWNLOAD_PROGRESS_INTERVAL || downloaded == total {
                    last_emit = Instant::now();
                    let _ = app.emit(
                        events::MODEL_DOWNLOAD,
                        DownloadEvent::Progress {
                            id: id.clone(),
                            downloaded,
                            total,
                        },
                    );
                }
            });
            let event = match result {
                Ok(path) => DownloadEvent::Done {
                    id: id.clone(),
                    path: path.to_string_lossy().into_owned(),
                },
                Err(models::DownloadError::Cancelled) => {
                    DownloadEvent::Cancelled { id: id.clone() }
                }
                Err(err) => {
                    tracing::warn!(%err, model = %id, "model download failed");
                    DownloadEvent::Failed {
                        id: id.clone(),
                        message: err.to_string(),
                    }
                }
            };
            if let Ok(mut downloads) = app.state::<AppState>().downloads.lock() {
                downloads.remove(&id);
            }
            let _ = app.emit(events::MODEL_DOWNLOAD, event);
        })
        .map_err(internal)?;
    Ok(())
}

#[tauri::command]
pub fn cancel_ggml_download(state: State<'_, AppState>, id: String) {
    if let Ok(downloads) = state.downloads.lock() {
        if let Some(flag) = downloads.get(&id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

#[tauri::command]
pub fn delete_ggml_model(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let entry =
        models::find(&id).ok_or_else(|| AppError::new(ErrorCode::Download, "Bilinmeyen model."))?;
    let settings = state.settings_snapshot();
    let in_use = settings
        .transcription
        .whisper_cpp
        .model_path
        .as_deref()
        .is_some_and(|p| p.ends_with(entry.file_name));
    if in_use {
        return Err(AppError::new(
            ErrorCode::Download,
            "Bu model şu an kullanılıyor; önce başka bir model seçin.",
        ));
    }
    models::delete(&id, &state.engines.env().app_data)
        .map_err(|err| AppError::new(ErrorCode::Download, err.to_string()))
}

#[tauri::command]
pub fn discover_mlx_models(state: State<'_, AppState>) -> Vec<MlxModel> {
    mlx_env::discover_models(&state.home)
}

#[tauri::command]
pub async fn check_mlx_python(
    state: State<'_, AppState>,
    python_path: Option<String>,
) -> AppResult<PythonCheck> {
    let python = python_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| mlx_env::default_venv_python(&state.engines.env().app_data));
    if !python.is_absolute() {
        return Err(AppError::new(
            ErrorCode::InvalidSettings,
            "Python yolu tam yol olmalı.",
        ));
    }
    blocking(move || Ok(mlx_env::check_python(&python))).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum MlxSetupEvent {
    Log { line: String },
    Done { python_path: String },
    Failed { message: String },
}

#[tauri::command]
pub fn setup_mlx_env(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    if state.mlx_setup_running.swap(true, Ordering::SeqCst) {
        return Err(AppError::new(ErrorCode::Engine, "Kurulum zaten sürüyor."));
    }
    let app_data = state.engines.env().app_data.clone();
    let home = state.home.clone();
    std::thread::Builder::new()
        .name("sedjem-mlx-setup".into())
        .spawn(move || {
            let emitter = app.clone();
            let log = move |line: &str| {
                let _ = emitter.emit(
                    events::MLX_SETUP,
                    MlxSetupEvent::Log {
                        line: line.to_owned(),
                    },
                );
            };
            let result = mlx_env::provision_env(&app_data, &home, &log);
            let state = app.state::<AppState>();
            state.mlx_setup_running.store(false, Ordering::SeqCst);
            let event = match result {
                Ok(python) => {
                    let settings = state.settings_snapshot();
                    state.engines.apply(state.engine_config(&settings), true);
                    MlxSetupEvent::Done {
                        python_path: python.to_string_lossy().into_owned(),
                    }
                }
                Err(message) => MlxSetupEvent::Failed { message },
            };
            let _ = app.emit(events::MLX_SETUP, event);
        })
        .map_err(|err| {
            state.mlx_setup_running.store(false, Ordering::SeqCst);
            internal(err)
        })?;
    Ok(())
}

#[tauri::command]
pub async fn test_wyoming(host: String, port: u16) -> AppResult<String> {
    blocking(move || {
        let engine = WyomingEngine::new(host, port)
            .map_err(|err| AppError::new(ErrorCode::Engine, err.to_string()))?;
        engine
            .probe()
            .map_err(|err| AppError::new(ErrorCode::Engine, err.to_string()))
    })
    .await
}

/// Checks the server answers at all; any HTTP status proves reachability.
#[tauri::command]
pub async fn test_openai(base_url: String) -> AppResult<String> {
    validate_base_url(&base_url)?;
    blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(HTTP_PROBE_TIMEOUT)
            .build()
            .map_err(internal)?;
        let url = format!("{}/models", base_url.trim().trim_end_matches('/'));
        let response = client
            .get(url)
            .send()
            .map_err(|err| AppError::new(ErrorCode::Engine, err.without_url().to_string()))?;
        Ok(format!(
            "Sunucu yanıt verdi (HTTP {}).",
            response.status().as_u16()
        ))
    })
    .await
}

#[tauri::command]
pub fn has_openai_api_key() -> bool {
    crate::platform::secrets::has_openai_api_key()
}

#[tauri::command]
pub fn set_openai_api_key(state: State<'_, AppState>, key: Option<String>) -> AppResult<()> {
    crate::platform::secrets::set_openai_api_key(key.as_deref())
        .map_err(|err| AppError::new(ErrorCode::Keychain, err))?;
    state.secret_epoch.fetch_add(1, Ordering::SeqCst);
    let settings = state.settings_snapshot();
    state.engines.apply(state.engine_config(&settings), false);
    Ok(())
}

// ---------------------------------------------------------- permissions --

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub accessibility: bool,
    pub microphone: MicrophoneStatus,
}

#[tauri::command]
pub fn get_permissions() -> Permissions {
    Permissions {
        accessibility: platform::accessibility_trusted(),
        microphone: platform::microphone_status(),
    }
}

#[tauri::command]
pub fn open_privacy_settings(pane: PrivacyPane) -> AppResult<()> {
    platform::open_privacy_pane(pane).map_err(|err| AppError::new(ErrorCode::Permission, err))
}

/// Opens the microphone briefly so macOS shows its permission prompt.
#[tauri::command]
pub async fn request_microphone(state: State<'_, AppState>) -> AppResult<MicrophoneStatus> {
    let device_name = state.settings_snapshot().recording.input_device;
    blocking(move || {
        let config = CaptureConfig {
            device_name,
            silence_timeout: None,
            max_duration: MIC_PROBE_DURATION * 2,
        };
        let recording = Recording::start(config, None, Arc::new(|_| {}))
            .map_err(|err| AppError::new(ErrorCode::Audio, err.to_string()))?;
        std::thread::sleep(MIC_PROBE_DURATION);
        recording
            .stop()
            .map_err(|err| AppError::new(ErrorCode::Audio, err.to_string()))?;
        Ok(platform::microphone_status())
    })
    .await
}

#[tauri::command]
pub async fn list_input_devices() -> AppResult<Vec<String>> {
    blocking(|| Ok(capture::list_input_devices())).await
}

// -------------------------------------------------------------- windows --

#[tauri::command]
pub fn show_settings(app: AppHandle) {
    windows::show_settings(&app);
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

// -------------------------------------------------------------- updates --

#[tauri::command]
pub fn get_update_info(app: AppHandle, state: State<'_, AppState>) -> UpdateInfo {
    state.updates.info(&app)
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> AppResult<UpdateInfo> {
    let state = app.state::<AppState>();
    state.updates.check(&app).await
}

/// Downloads, verifies and installs the update; the app restarts on success.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    state.updates.install(&app).await
}
