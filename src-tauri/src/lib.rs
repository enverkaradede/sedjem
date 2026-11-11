//! Sedjem — a floating push-to-talk / toggle dictation button that turns
//! speech into text with a local Whisper model and pastes it into the
//! focused app.

pub mod audio;
pub mod commands;
pub mod controller;
pub mod dictation;
pub mod engine;
pub mod error;
pub mod hotkey;
pub mod platform;
pub mod settings;
pub mod state;
pub mod text;
pub mod tray;
pub mod updates;
pub mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};

use crate::dictation::{Dictation, Microphone, SystemTextOutput};
use crate::engine::manager::{EngineEnvironment, EngineManager, EngineStatus};
use crate::engine::{mlx_env, models};
use crate::settings::{EngineKind, LoadOutcome, Settings, SettingsStore};
use crate::state::{events, AppState, TauriEvents};

const MLX_WORKER_RESOURCE: &str = "resources/mlx_worker.py";
/// Passed by the login item so a login launch never pops the settings window.
const AUTOSTART_FLAG: &str = "--autostart";

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("sedjem_lib=info,whisper_rs=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// First run: point the engines at what this machine already has, so the
/// app works without a trip to the settings page whenever possible.
fn first_run_defaults(home: &std::path::Path, app_data: &std::path::Path) -> Settings {
    let mut settings = Settings::default();
    let mlx_models = mlx_env::discover_models(home);
    if let Some(model) = mlx_env::preferred_model(&mlx_models) {
        settings.transcription.mlx.model_dir = Some(model.path.clone());
    }
    if let Some(installed) = models::list(app_data).into_iter().find(|m| m.installed) {
        settings.transcription.whisper_cpp.model_path = Some(installed.path);
    }
    let has_mlx_runtime = mlx_env::default_venv_python(app_data).is_file();
    settings.transcription.engine = match (
        &settings.transcription.mlx.model_dir,
        &settings.transcription.whisper_cpp.model_path,
    ) {
        (_, Some(_)) if !has_mlx_runtime => EngineKind::WhisperCpp,
        _ => EngineKind::Mlx,
    };
    settings
}

fn load_settings(
    store: &SettingsStore,
    home: &std::path::Path,
    app_data: &std::path::Path,
) -> (Settings, bool, Option<String>) {
    match store.load() {
        LoadOutcome::Loaded(settings) => (settings, false, None),
        LoadOutcome::FirstRun(_) => {
            let settings = first_run_defaults(home, app_data);
            if let Err(err) = store.save(&settings) {
                tracing::warn!(%err, "could not write initial settings");
            }
            (settings, true, None)
        }
        LoadOutcome::Recovered { settings, reason } => (
            settings,
            true,
            Some(format!("Ayar dosyası okunamadı ({reason}); varsayılanlar yüklendi, eski dosya settings.invalid.json olarak saklandı.")),
        ),
    }
}

fn reconcile_autostart(app: &AppHandle, wanted: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let enabled = autolaunch.is_enabled().map_err(|err| err.to_string())?;
    match (wanted, enabled) {
        (true, false) => autolaunch.enable().map_err(|err| err.to_string()),
        (false, true) => autolaunch.disable().map_err(|err| err.to_string()),
        _ => Ok(()),
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let config_dir = app.path().app_config_dir()?;
    let app_data = app.path().app_data_dir()?;
    let home = app.path().home_dir()?;
    let worker_script = app
        .path()
        .resolve(MLX_WORKER_RESOURCE, BaseDirectory::Resource)
        .unwrap_or_else(|_| PathBuf::from(MLX_WORKER_RESOURCE));

    let store = SettingsStore::new(config_dir);
    let (settings, show_settings_now, recovered) = load_settings(&store, &home, &app_data);

    let status_app = handle.clone();
    let engines = EngineManager::new(
        EngineEnvironment {
            app_data,
            mlx_worker_script: worker_script,
        },
        Arc::new(move |status: &EngineStatus| {
            if let Err(err) = status_app.emit(events::ENGINE_STATUS, status.clone()) {
                tracing::warn!(%err, "could not emit engine status");
            }
        }),
        Arc::new(platform::secrets::openai_api_key),
    );
    let shared_settings = Arc::new(RwLock::new(settings.clone()));
    let dictation = Dictation::new(
        shared_settings.clone(),
        engines.clone(),
        Arc::new(Microphone),
        Arc::new(SystemTextOutput),
        Arc::new(TauriEvents {
            app: handle.clone(),
        }),
    );
    app.manage(AppState {
        settings: shared_settings,
        store,
        engines: engines.clone(),
        dictation,
        updates: updates::Updates::default(),
        downloads: Mutex::new(HashMap::new()),
        mlx_setup_running: AtomicBool::new(false),
        secret_epoch: AtomicU64::new(0),
        panel_move_generation: AtomicU64::new(0),
        home,
        startup_notices: Mutex::new(Vec::new()),
    });
    let state = app.state::<AppState>();
    if let Some(notice) = recovered {
        state.push_notice(notice);
    }

    windows::apply_dock_visibility(&handle, settings.general.show_in_dock);
    windows::keep_settings_alive(&handle);
    windows::setup_panel(
        &handle,
        settings.general.panel_position,
        settings.general.panel_visible,
    )?;
    tray::setup(&handle)?;

    if let Err(err) = hotkey::apply(&handle, &settings.shortcut) {
        tracing::warn!(%err, "shortcut registration failed at startup");
        state.push_notice(err.message);
    }
    if let Err(err) = reconcile_autostart(&handle, settings.general.launch_at_login) {
        tracing::warn!(%err, "autostart reconcile failed");
    }
    engines.apply(state.engine_config(&settings), false);
    updates::spawn_scheduler(handle.clone());
    tracing::info!(version = %handle.package_info().version, "Sedjem started");

    let launched_at_login = std::env::args().any(|arg| arg == AUTOSTART_FLAG);
    if show_settings_now && !launched_at_login {
        windows::show_settings(&handle);
    }
    Ok(())
}

pub fn run() {
    init_logging();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::show_settings(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkey::on_shortcut)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_FLAG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_nspanel_plugin())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::take_startup_notices,
            commands::get_app_info,
            commands::panel_press,
            commands::panel_release,
            commands::cancel_dictation,
            commands::get_dictation_state,
            commands::get_history,
            commands::clear_history,
            commands::copy_to_clipboard,
            commands::get_engine_status,
            commands::reload_engine,
            commands::list_ggml_models,
            commands::download_ggml_model,
            commands::cancel_ggml_download,
            commands::delete_ggml_model,
            commands::discover_mlx_models,
            commands::check_mlx_python,
            commands::setup_mlx_env,
            commands::test_wyoming,
            commands::test_openai,
            commands::has_openai_api_key,
            commands::set_openai_api_key,
            commands::get_permissions,
            commands::open_privacy_settings,
            commands::request_microphone,
            commands::list_input_devices,
            commands::show_settings,
            commands::quit_app,
            commands::get_update_info,
            commands::check_for_update,
            commands::install_update,
        ])
        .build(tauri::generate_context!());

    match result {
        // No exit interception: windows are hidden rather than closed, so the
        // app only exits on an explicit quit (tray, Dock, logout, shutdown).
        Ok(app) => app.run(|_app, _event| {}),
        Err(err) => {
            tracing::error!(%err, "failed to start Sedjem");
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "macos")]
fn tauri_nspanel_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_nspanel::init()
}

#[cfg(not(target_os = "macos"))]
fn tauri_nspanel_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("nspanel-noop").build()
}
