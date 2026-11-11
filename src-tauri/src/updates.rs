//! Self-update via signed GitHub releases (tauri-plugin-updater).
//!
//! Every artifact is verified against the minisign public key compiled into
//! the app, so a compromised download URL cannot install code. Updates never
//! interrupt a dictation: installing (which restarts the app) waits until the
//! dictation pipeline is idle. Only one check/install runs at a time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::dictation::PhaseName;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::{events, AppState};

/// Let the app settle (engine warm-up, first paint) before the first check.
const STARTUP_DELAY: Duration = Duration::from_secs(8);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// How often an auto-install re-checks whether dictation became idle.
const IDLE_POLL: Duration = Duration::from_secs(5);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate {
        checked_at_ms: u64,
    },
    Available {
        version: String,
        notes: Option<String>,
        date: Option<String>,
    },
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Installing {
        version: String,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    #[serde(flatten)]
    pub state: UpdateState,
}

pub struct Updates {
    state: Mutex<UpdateState>,
    pending: Mutex<Option<Update>>,
    busy: AtomicBool,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            state: Mutex::new(UpdateState::Idle),
            pending: Mutex::new(None),
            busy: AtomicBool::new(false),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn update_error(err: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Internal, format!("Güncelleme hatası: {err}"))
}

/// Clears the busy flag however the guarded operation ends.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Updates {
    pub fn info<R: Runtime>(&self, app: &AppHandle<R>) -> UpdateInfo {
        let state = self
            .state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(UpdateState::Idle);
        UpdateInfo {
            current_version: app.package_info().version.to_string(),
            state,
        }
    }

    fn set<R: Runtime>(&self, app: &AppHandle<R>, next: UpdateState) {
        if let Ok(mut state) = self.state.lock() {
            *state = next;
        }
        if let Err(err) = app.emit(events::UPDATE_STATE, self.info(app)) {
            tracing::warn!(%err, "could not emit update state");
        }
    }

    fn acquire(&self) -> AppResult<BusyGuard<'_>> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Bir güncelleme işlemi zaten sürüyor.",
            ));
        }
        Ok(BusyGuard(&self.busy))
    }

    /// Asks the release endpoint whether a newer signed version exists.
    pub async fn check<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<UpdateInfo> {
        let _guard = self.acquire()?;
        self.set(app, UpdateState::Checking);
        let result = async {
            let updater = app
                .updater_builder()
                .timeout(CHECK_TIMEOUT)
                .build()
                .map_err(update_error)?;
            updater.check().await.map_err(update_error)
        }
        .await;
        match result {
            Ok(Some(update)) => {
                tracing::info!(current = %update.current_version, available = %update.version, "update available");
                let state = UpdateState::Available {
                    version: update.version.clone(),
                    notes: update.body.clone().filter(|b| !b.trim().is_empty()),
                    date: update.date.map(|d| d.to_string()),
                };
                if let Ok(mut pending) = self.pending.lock() {
                    *pending = Some(update);
                }
                self.set(app, state);
            }
            Ok(None) => self.set(
                app,
                UpdateState::UpToDate {
                    checked_at_ms: now_ms(),
                },
            ),
            Err(err) => {
                tracing::warn!(message = %err.message, "update check failed");
                self.set(
                    app,
                    UpdateState::Failed {
                        message: err.message.clone(),
                    },
                );
                return Err(err);
            }
        }
        Ok(self.info(app))
    }

    /// Downloads, verifies and installs the pending update, then restarts.
    pub async fn install<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        if dictation_busy(app) {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Dikte sürerken güncelleme kurulamaz; bitince tekrar deneyin.",
            ));
        }
        let _guard = self.acquire()?;
        let update = self
            .pending
            .lock()
            .ok()
            .and_then(|p| p.clone())
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Internal,
                    "Kurulacak bir güncelleme yok; önce denetleyin.",
                )
            })?;
        let version = update.version.clone();
        let mut downloaded = 0u64;
        let mut last_emit = Instant::now() - PROGRESS_EMIT_INTERVAL;
        self.set(
            app,
            UpdateState::Downloading {
                version: version.clone(),
                downloaded: 0,
                total: None,
            },
        );
        let result = update
            .download_and_install(
                |chunk, total| {
                    downloaded += chunk as u64;
                    if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                        last_emit = Instant::now();
                        self.set(
                            app,
                            UpdateState::Downloading {
                                version: version.clone(),
                                downloaded,
                                total,
                            },
                        );
                    }
                },
                || {
                    self.set(
                        app,
                        UpdateState::Installing {
                            version: version.clone(),
                        },
                    )
                },
            )
            .await;
        if let Err(err) = result {
            let err = update_error(err);
            self.set(
                app,
                UpdateState::Failed {
                    message: err.message.clone(),
                },
            );
            return Err(err);
        }
        tracing::info!(%version, "update installed; restarting");
        app.restart();
    }
}

fn dictation_busy<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<AppState>().dictation.current_state().phase != PhaseName::Idle
}

/// Background scheduler: first check after `STARTUP_DELAY`, then every
/// `CHECK_INTERVAL`, honouring the user's auto-check / auto-install settings.
pub fn spawn_scheduler<R: Runtime>(app: AppHandle<R>) {
    let spawned = thread::Builder::new()
        .name("sedjem-updates".into())
        .spawn(move || {
            thread::sleep(STARTUP_DELAY);
            loop {
                run_scheduled_check(&app);
                thread::sleep(CHECK_INTERVAL);
            }
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "could not start update scheduler");
    }
}

fn run_scheduled_check<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let settings = state.settings_snapshot().updates;
    if !settings.auto_check {
        return;
    }
    let found = matches!(
        tauri::async_runtime::block_on(state.updates.check(app)).map(|info| info.state),
        Ok(UpdateState::Available { .. })
    );
    if !(found && settings.auto_install) {
        return;
    }
    // Never restart mid-dictation; bounded by the next scheduled check.
    let deadline = Instant::now() + CHECK_INTERVAL;
    while dictation_busy(app) && Instant::now() < deadline {
        thread::sleep(IDLE_POLL);
    }
    if let Err(err) = tauri::async_runtime::block_on(state.updates.install(app)) {
        tracing::warn!(message = %err.message, "automatic update install failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_serialises_with_a_tag_for_the_ui() {
        let info = UpdateInfo {
            current_version: "0.1.0".into(),
            state: UpdateState::Available {
                version: "0.2.0".into(),
                notes: None,
                date: None,
            },
        };
        let json = serde_json::to_value(info).unwrap();
        assert_eq!(json["state"], "available");
        assert_eq!(json["currentVersion"], "0.1.0");
        assert_eq!(json["version"], "0.2.0");
    }

    #[test]
    fn busy_guard_rejects_concurrent_operations_and_releases_on_drop() {
        let updates = Updates::default();
        let guard = updates.acquire().unwrap();
        assert!(updates.acquire().is_err());
        drop(guard);
        assert!(updates.acquire().is_ok());
    }
}
