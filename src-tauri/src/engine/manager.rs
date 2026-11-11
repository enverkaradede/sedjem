//! Owns the active engine and rebuilds it when its configuration changes.
//!
//! Loading (a 1.6 GB model, a Python worker) happens on a background thread;
//! a generation counter discards results from loads that were superseded by
//! a newer configuration while they were still running. Callers that need
//! the engine wait on a condvar with a timeout instead of polling.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::mlx::MlxEngine;
use super::mlx_env::default_venv_python;
use super::openai::OpenAiEngine;
use super::whisper_cpp::WhisperCppEngine;
use super::wyoming::WyomingEngine;
use super::{EngineError, Transcriber};
use crate::settings::{EngineKind, TranscriptionSettings};

/// Everything that determines which engine instance is built. Two equal
/// configs never trigger a reload.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub kind: EngineKind,
    pub whisper_model: Option<String>,
    pub whisper_gpu: bool,
    pub whisper_flash_attention: bool,
    pub mlx_python: PathBuf,
    pub mlx_model_dir: Option<String>,
    pub wyoming_host: String,
    pub wyoming_port: u16,
    pub openai_base_url: String,
    pub openai_model: String,
    /// Bumped when the API key in the Keychain changes.
    pub secret_epoch: u64,
}

/// Paths the engines need that do not come from user settings.
#[derive(Debug, Clone)]
pub struct EngineEnvironment {
    pub app_data: PathBuf,
    pub mlx_worker_script: PathBuf,
}

impl EngineConfig {
    pub fn from_settings(
        settings: &TranscriptionSettings,
        env: &EngineEnvironment,
        secret_epoch: u64,
    ) -> Self {
        Self {
            kind: settings.engine,
            whisper_model: settings.whisper_cpp.model_path.clone(),
            whisper_gpu: settings.whisper_cpp.use_gpu,
            whisper_flash_attention: settings.whisper_cpp.flash_attention,
            mlx_python: settings
                .mlx
                .python_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_venv_python(&env.app_data)),
            mlx_model_dir: settings.mlx.model_dir.clone(),
            wyoming_host: settings.wyoming.host.clone(),
            wyoming_port: settings.wyoming.port,
            openai_base_url: settings.openai.base_url.clone(),
            openai_model: settings.openai.model.clone(),
            secret_epoch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum EngineStatus {
    Loading {
        kind: EngineKind,
    },
    Ready {
        kind: EngineKind,
        description: String,
    },
    Failed {
        kind: EngineKind,
        message: String,
    },
}

pub type StatusFn = Arc<dyn Fn(&EngineStatus) + Send + Sync>;
pub type ApiKeyFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;

struct Slot {
    generation: u64,
    config: Option<EngineConfig>,
    engine: Option<Arc<dyn Transcriber>>,
    status: EngineStatus,
}

pub struct EngineManager {
    slot: Mutex<Slot>,
    changed: Condvar,
    env: EngineEnvironment,
    on_status: StatusFn,
    api_key: ApiKeyFn,
}

fn build(
    config: &EngineConfig,
    env: &EngineEnvironment,
    api_key: &ApiKeyFn,
) -> Result<Arc<dyn Transcriber>, EngineError> {
    match config.kind {
        EngineKind::WhisperCpp => {
            let path = config.whisper_model.as_ref().ok_or_else(|| {
                EngineError::NotConfigured("Bir ggml modeli indirin veya seçin.".into())
            })?;
            Ok(Arc::new(WhisperCppEngine::load(
                std::path::Path::new(path),
                config.whisper_gpu,
                config.whisper_flash_attention,
            )?))
        }
        EngineKind::Mlx => {
            let model_dir = config
                .mlx_model_dir
                .clone()
                .ok_or_else(|| EngineError::NotConfigured("Bir MLX model klasörü seçin.".into()))?;
            Ok(Arc::new(MlxEngine::start(
                config.mlx_python.clone(),
                env.mlx_worker_script.clone(),
                model_dir,
            )?))
        }
        EngineKind::Wyoming => Ok(Arc::new(WyomingEngine::new(
            config.wyoming_host.clone(),
            config.wyoming_port,
        )?)),
        EngineKind::Openai => Ok(Arc::new(OpenAiEngine::new(
            &config.openai_base_url,
            &config.openai_model,
            api_key(),
        )?)),
    }
}

impl EngineManager {
    pub fn new(env: EngineEnvironment, on_status: StatusFn, api_key: ApiKeyFn) -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(Slot {
                generation: 0,
                config: None,
                engine: None,
                status: EngineStatus::Failed {
                    kind: EngineKind::Mlx,
                    message: "Henüz yüklenmedi.".into(),
                },
            }),
            changed: Condvar::new(),
            env,
            on_status,
            api_key,
        })
    }

    pub fn env(&self) -> &EngineEnvironment {
        &self.env
    }

    pub fn status(&self) -> EngineStatus {
        self.slot
            .lock()
            .map(|s| s.status.clone())
            .unwrap_or_else(|_| EngineStatus::Failed {
                kind: EngineKind::Mlx,
                message: "durum kilidi bozuk".into(),
            })
    }

    /// Applies a configuration. No-op when unchanged and healthy; `force`
    /// reloads anyway (e.g. after installing the MLX environment).
    pub fn apply(self: &Arc<Self>, config: EngineConfig, force: bool) {
        let generation = {
            let Ok(mut slot) = self.slot.lock() else {
                return;
            };
            let healthy = !matches!(slot.status, EngineStatus::Failed { .. });
            if !force && healthy && slot.config.as_ref() == Some(&config) {
                return;
            }
            slot.generation += 1;
            slot.config = Some(config.clone());
            // Release the old engine now: frees GPU memory / stops the worker.
            slot.engine = None;
            slot.status = EngineStatus::Loading { kind: config.kind };
            (self.on_status)(&slot.status);
            slot.generation
        };

        let manager = Arc::clone(self);
        let spawned = thread::Builder::new().name("sedjem-engine-load".into()).spawn(move || {
            let started = Instant::now();
            let result = build(&config, &manager.env, &manager.api_key);
            let Ok(mut slot) = manager.slot.lock() else { return };
            if slot.generation != generation {
                return; // superseded; the newer load owns the slot
            }
            slot.status = match result {
                Ok(engine) => {
                    tracing::info!(kind = ?config.kind, elapsed_ms = started.elapsed().as_millis() as u64, "engine ready");
                    let description = engine.describe();
                    slot.engine = Some(engine);
                    EngineStatus::Ready { kind: config.kind, description }
                }
                Err(err) => {
                    tracing::warn!(kind = ?config.kind, %err, "engine failed to load");
                    EngineStatus::Failed { kind: config.kind, message: err.to_string() }
                }
            };
            (manager.on_status)(&slot.status);
            manager.changed.notify_all();
        });
        if let Err(err) = spawned {
            tracing::error!(%err, "could not spawn engine loader");
        }
    }

    /// Returns the ready engine, waiting up to `timeout` if it is still loading.
    pub fn wait_ready(&self, timeout: Duration) -> Result<Arc<dyn Transcriber>, EngineError> {
        let deadline = Instant::now() + timeout;
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| EngineError::Load("durum kilidi bozuk".into()))?;
        loop {
            match (&slot.engine, &slot.status) {
                (Some(engine), EngineStatus::Ready { .. }) => return Ok(Arc::clone(engine)),
                (_, EngineStatus::Failed { message, .. }) => {
                    return Err(EngineError::NotConfigured(message.clone()))
                }
                _ => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(EngineError::Timeout);
            }
            let (next, _) = self
                .changed
                .wait_timeout(slot, remaining)
                .map_err(|_| EngineError::Load("durum kilidi bozuk".into()))?;
            slot = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn env() -> EngineEnvironment {
        EngineEnvironment {
            app_data: PathBuf::from("/tmp/sedjem-test"),
            mlx_worker_script: PathBuf::from("/nope.py"),
        }
    }

    fn manager(counter: Arc<AtomicUsize>) -> Arc<EngineManager> {
        EngineManager::new(
            env(),
            Arc::new(move |_status: &EngineStatus| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(|| None),
        )
    }

    fn wyoming_config(port: u16) -> EngineConfig {
        let mut settings = Settings::default().transcription;
        settings.engine = EngineKind::Wyoming;
        settings.wyoming.host = "127.0.0.1".into();
        settings.wyoming.port = port;
        EngineConfig::from_settings(&settings, &env(), 0)
    }

    #[test]
    fn loads_engine_and_reports_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let mgr = manager(counter.clone());
        mgr.apply(wyoming_config(listener.local_addr().unwrap().port()), false);
        let engine = mgr.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(engine.kind(), EngineKind::Wyoming);
        assert!(matches!(mgr.status(), EngineStatus::Ready { .. }));
        assert_eq!(counter.load(Ordering::SeqCst), 2); // loading + ready
    }

    #[test]
    fn identical_config_does_not_reload() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mgr = manager(counter.clone());
        let config = wyoming_config(10_300);
        mgr.apply(config.clone(), false);
        mgr.wait_ready(Duration::from_secs(5)).unwrap();
        mgr.apply(config, false);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn missing_configuration_fails_fast() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mgr = manager(counter);
        let mut settings = Settings::default().transcription;
        settings.engine = EngineKind::WhisperCpp;
        mgr.apply(EngineConfig::from_settings(&settings, &env(), 0), false);
        let err = mgr.wait_ready(Duration::from_secs(5)).err().unwrap();
        assert!(matches!(err, EngineError::NotConfigured(_)));
    }
}
