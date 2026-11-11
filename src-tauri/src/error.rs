//! Application-level error type shared by every Tauri command.
//!
//! Commands return `Result<T, AppError>`; the error serialises to
//! `{ code, message }` so the UI can branch on a stable code and show a
//! human message without parsing strings.

use serde::Serialize;

/// Stable, machine-readable error codes surfaced to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidSettings,
    Io,
    Shortcut,
    Autostart,
    Audio,
    Engine,
    EngineNotReady,
    NoSpeech,
    Inject,
    Download,
    Keychain,
    Permission,
    Internal,
}

/// Error returned across the IPC boundary. The message is safe to show to
/// the user: it never contains transcript text, audio or credentials.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, err.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
