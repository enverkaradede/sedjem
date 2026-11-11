//! Speech-to-text engine abstraction.
//!
//! Every backend implements [`Transcriber`]. Batch engines (all of v1)
//! implement `transcribe`; a future streaming model additionally overrides
//! `open_stream`. The capture pipeline already feeds a [`StreamingSession`]
//! chunk by chunk and forwards partial text to the UI whenever the active
//! engine offers one, so adding a streaming model is a new adapter plus a
//! settings entry — no change to capture, controller or UI flow.

pub mod manager;
pub mod mlx;
pub mod mlx_env;
pub mod models;
pub mod openai;
pub mod whisper_cpp;
pub mod wyoming;

use crate::settings::EngineKind;

/// Per-request options shared by all engines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscribeOptions {
    /// ISO-639-1 code, or `None` for automatic detection.
    pub language: Option<String>,
    /// Vocabulary hint; empty strings are normalised to `None`.
    pub initial_prompt: Option<String>,
}

impl TranscribeOptions {
    pub fn from_settings(language: &str, initial_prompt: &str) -> Self {
        let language = (language != "auto").then(|| language.to_owned());
        let prompt = initial_prompt.trim();
        let initial_prompt = (!prompt.is_empty()).then(|| prompt.to_owned());
        Self {
            language,
            initial_prompt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    /// Language the engine detected or used, when it reports one.
    pub language: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("Motor yapılandırılmamış: {0}")]
    NotConfigured(String),
    #[error("Model yüklenemedi: {0}")]
    Load(String),
    #[error("Yazıya dökme başarısız: {0}")]
    Transcribe(String),
    #[error("Sunucuya bağlanılamadı: {0}")]
    Connection(String),
    #[error("Motor zamanında yanıt vermedi.")]
    Timeout,
    #[error("Beklenmeyen yanıt: {0}")]
    Protocol(String),
    #[error("Bu motor canlı (streaming) yazıya dökmeyi desteklemiyor.")]
    StreamingUnsupported,
}

/// A loaded, ready-to-use speech-to-text backend.
pub trait Transcriber: Send + Sync {
    fn kind(&self) -> EngineKind;

    /// Short human description for the settings UI (model name, server...).
    fn describe(&self) -> String;

    /// Transcribes a complete 16 kHz mono clip.
    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError>;

    fn supports_streaming(&self) -> bool {
        false
    }

    /// Opens an incremental session; only streaming engines override this.
    fn open_stream(
        &self,
        _options: &TranscribeOptions,
    ) -> Result<Box<dyn StreamingSession>, EngineError> {
        Err(EngineError::StreamingUnsupported)
    }
}

/// Incremental transcription fed while the user is still speaking.
pub trait StreamingSession: Send {
    /// Pushes 16 kHz mono samples; may return an updated partial transcript.
    fn push(&mut self, samples: &[f32]) -> Result<Option<String>, EngineError>;

    /// Ends the utterance and returns the final transcript.
    fn finish(self: Box<Self>) -> Result<Transcript, EngineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_language_and_blank_prompt_become_none() {
        let opts = TranscribeOptions::from_settings("auto", "   ");
        assert_eq!(
            opts,
            TranscribeOptions {
                language: None,
                initial_prompt: None
            }
        );
        let opts = TranscribeOptions::from_settings("tr", " Sedjem, Tauri ");
        assert_eq!(opts.language.as_deref(), Some("tr"));
        assert_eq!(opts.initial_prompt.as_deref(), Some("Sedjem, Tauri"));
    }
}
