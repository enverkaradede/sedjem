//! Persistent user settings: schema, defaults, validation and atomic storage.
//!
//! Every struct uses `#[serde(default)]` so a settings file written by an
//! older build still loads after new fields are added (forward migration by
//! defaulting). Validation runs on every write coming from the UI; a file
//! that fails validation on load is replaced by defaults and kept aside as
//! `settings.invalid.json` so nothing the user typed is silently lost.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorCode};

pub const SETTINGS_VERSION: u32 = 1;
const SETTINGS_FILE: &str = "settings.json";
const INVALID_SETTINGS_FILE: &str = "settings.invalid.json";

pub const DEFAULT_ACCELERATOR: &str = "Alt+Space";
pub const TAP_THRESHOLD_RANGE_MS: (u32, u32) = (120, 1_000);
pub const SILENCE_TIMEOUT_RANGE_MS: (u32, u32) = (500, 10_000);
pub const MAX_DURATION_RANGE_S: (u32, u32) = (10, 1_800);
pub const MAX_SNIPPETS: usize = 500;
pub const MAX_TRIGGER_CHARS: usize = 100;
pub const MAX_EXPANSION_CHARS: usize = 10_000;
pub const MAX_INITIAL_PROMPT_CHARS: usize = 600;
const MAX_ACCELERATOR_CHARS: usize = 64;
const MAX_PATH_CHARS: usize = 4_096;

/// Languages offered in the UI. `auto` lets the engine detect per utterance.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "auto", "tr", "en", "de", "fr", "es", "it", "pt", "nl", "ru", "ar", "fa", "ja", "zh", "ko",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub general: GeneralSettings,
    pub shortcut: ShortcutSettings,
    pub recording: RecordingSettings,
    pub transcription: TranscriptionSettings,
    pub text: TextSettings,
    pub updates: UpdateSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: GeneralSettings::default(),
            shortcut: ShortcutSettings::default(),
            recording: RecordingSettings::default(),
            transcription: TranscriptionSettings::default(),
            text: TextSettings::default(),
            updates: UpdateSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    /// Check GitHub Releases shortly after launch and periodically.
    pub auto_check: bool,
    /// Install found updates without asking (only while dictation is idle).
    pub auto_install: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            auto_install: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelPosition {
    pub x: f64,
    pub y: f64,
}

/// How the transcript reaches the focused app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InsertMethod {
    /// Clipboard + ⌘V, previous clipboard restored. Works everywhere, instant.
    #[default]
    Paste,
    /// Synthetic Unicode key events; the clipboard is never touched.
    Type,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralSettings {
    /// Off by default: the user opts in explicitly.
    pub launch_at_login: bool,
    pub show_in_dock: bool,
    pub panel_visible: bool,
    pub panel_position: Option<PanelPosition>,
    pub insert_method: InsertMethod,
    /// Press Return after the transcript is inserted.
    pub auto_enter: bool,
    /// Put the user's previous clipboard back after pasting.
    pub restore_clipboard: bool,
    /// Keep the last transcripts in memory (never on disk) for re-copying.
    pub keep_history: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            show_in_dock: false,
            panel_visible: true,
            panel_position: None,
            insert_method: InsertMethod::Paste,
            auto_enter: false,
            restore_clipboard: true,
            keep_history: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutSettings {
    pub enabled: bool,
    /// Accelerator in global-hotkey syntax, e.g. `Alt+Space`, `Control+Shift+KeyD`.
    pub accelerator: String,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            accelerator: DEFAULT_ACCELERATOR.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingSettings {
    /// A press released before this many ms is a tap (toggle mode);
    /// held longer it is push-to-talk and the release ends the recording.
    pub tap_threshold_ms: u32,
    /// In toggle mode only: stop automatically after this much silence.
    pub silence_auto_stop: bool,
    pub silence_timeout_ms: u32,
    /// Hard ceiling so a forgotten toggle cannot grow memory without bound.
    pub max_duration_s: u32,
    /// Input device name; `None` follows the system default.
    pub input_device: Option<String>,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            tap_threshold_ms: 300,
            silence_auto_stop: false,
            silence_timeout_ms: 2_000,
            max_duration_s: 300,
            input_device: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    WhisperCpp,
    Mlx,
    Wyoming,
    Openai,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptionSettings {
    pub engine: EngineKind,
    /// `auto` or an ISO-639-1 code from [`SUPPORTED_LANGUAGES`].
    pub language: String,
    /// Optional vocabulary hint (names, jargon) passed to engines that support it.
    pub initial_prompt: String,
    pub whisper_cpp: WhisperCppSettings,
    pub mlx: MlxSettings,
    pub wyoming: WyomingSettings,
    pub openai: OpenAiSettings,
}

impl Default for TranscriptionSettings {
    fn default() -> Self {
        Self {
            engine: EngineKind::Mlx,
            language: "auto".to_owned(),
            initial_prompt: String::new(),
            whisper_cpp: WhisperCppSettings::default(),
            mlx: MlxSettings::default(),
            wyoming: WyomingSettings::default(),
            openai: OpenAiSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WhisperCppSettings {
    /// Absolute path to a ggml model file (downloaded or user-picked).
    pub model_path: Option<String>,
    pub use_gpu: bool,
    pub flash_attention: bool,
}

impl Default for WhisperCppSettings {
    fn default() -> Self {
        Self {
            model_path: None,
            use_gpu: true,
            flash_attention: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MlxSettings {
    /// Python interpreter with `mlx-whisper` installed. `None` = Sedjem's own venv.
    pub python_path: Option<String>,
    /// Local MLX model folder (a Hugging Face snapshot directory).
    pub model_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WyomingSettings {
    pub host: String,
    pub port: u16,
}

impl Default for WyomingSettings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 10_300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenAiSettings {
    /// Base URL including the API version segment, e.g. `http://localhost:8000/v1`.
    pub base_url: String,
    pub model: String,
}

impl Default for OpenAiSettings {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: "whisper-1".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnippetMatch {
    /// The whole utterance must be the trigger (punctuation ignored).
    Exact,
    /// The trigger is replaced wherever it appears as whole words.
    Inline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    pub match_mode: SnippetMatch,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextSettings {
    /// Drop phrases Whisper invents over silence (subtitle credits etc).
    pub filter_hallucinations: bool,
    pub snippets: Vec<Snippet>,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            filter_hallucinations: true,
            snippets: Vec::new(),
        }
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidSettings, message)
}

fn check_range(value: u32, (min, max): (u32, u32), label: &str) -> AppResult<()> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(invalid(format!("{label} {min} ile {max} arasında olmalı.")))
    }
}

fn check_optional_path(path: Option<&String>, label: &str) -> AppResult<()> {
    match path {
        Some(p) if p.trim().is_empty() || p.len() > MAX_PATH_CHARS => {
            Err(invalid(format!("{label} geçersiz.")))
        }
        Some(p) if !Path::new(p).is_absolute() => Err(invalid(format!("{label} tam yol olmalı."))),
        _ => Ok(()),
    }
}

/// Hosts that may be reached over plain HTTP: loopback, private LAN, link-local, `.local`.
fn is_local_host(host: &str) -> bool {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.ends_with(".local") {
        return true;
    }
    match trimmed.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// Validates an OpenAI-compatible base URL. Plain HTTP is only allowed for
/// local hosts so voice audio never crosses the internet unencrypted.
pub fn validate_base_url(raw: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|_| invalid("Sunucu adresi geçerli bir URL değil."))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("Sunucu adresinde host yok."))?;
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_local_host(host) => Ok(()),
        "http" => Err(invalid(
            "Uzak sunucular için https gerekli; ses şifresiz gönderilmez.",
        )),
        _ => Err(invalid("Sunucu adresi http veya https olmalı.")),
    }
}

fn validate_hostname(host: &str) -> AppResult<()> {
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if ok {
        Ok(())
    } else {
        Err(invalid("Wyoming sunucu adresi geçersiz."))
    }
}

impl Settings {
    /// Validates every field the UI can write. Engine-specific connection
    /// fields are only required for the engine that is currently active, so
    /// a half-configured inactive engine never blocks saving.
    pub fn validate(&self) -> AppResult<()> {
        let accelerator = self.shortcut.accelerator.trim();
        if accelerator.is_empty() || accelerator.len() > MAX_ACCELERATOR_CHARS {
            return Err(invalid("Kısayol geçersiz."));
        }
        check_range(
            self.recording.tap_threshold_ms,
            TAP_THRESHOLD_RANGE_MS,
            "Dokunma eşiği (ms)",
        )?;
        check_range(
            self.recording.silence_timeout_ms,
            SILENCE_TIMEOUT_RANGE_MS,
            "Sessizlik süresi (ms)",
        )?;
        check_range(
            self.recording.max_duration_s,
            MAX_DURATION_RANGE_S,
            "Azami kayıt süresi (sn)",
        )?;

        let t = &self.transcription;
        if !SUPPORTED_LANGUAGES.contains(&t.language.as_str()) {
            return Err(invalid("Desteklenmeyen dil."));
        }
        if t.initial_prompt.chars().count() > MAX_INITIAL_PROMPT_CHARS {
            return Err(invalid(format!(
                "Sözlük ipucu en fazla {MAX_INITIAL_PROMPT_CHARS} karakter olabilir."
            )));
        }
        check_optional_path(t.whisper_cpp.model_path.as_ref(), "Model dosyası yolu")?;
        check_optional_path(t.mlx.python_path.as_ref(), "Python yolu")?;
        check_optional_path(t.mlx.model_dir.as_ref(), "MLX model klasörü")?;
        match t.engine {
            EngineKind::Wyoming => validate_hostname(t.wyoming.host.trim())?,
            EngineKind::Openai => {
                validate_base_url(&t.openai.base_url)?;
                if t.openai.model.trim().is_empty() {
                    return Err(invalid("Model adı boş olamaz."));
                }
            }
            EngineKind::WhisperCpp | EngineKind::Mlx => {}
        }
        if t.wyoming.port == 0 {
            return Err(invalid("Wyoming portu 0 olamaz."));
        }
        self.validate_snippets()
    }

    fn validate_snippets(&self) -> AppResult<()> {
        let snippets = &self.text.snippets;
        if snippets.len() > MAX_SNIPPETS {
            return Err(invalid(format!(
                "En fazla {MAX_SNIPPETS} snippet tanımlanabilir."
            )));
        }
        let mut seen_ids = std::collections::HashSet::with_capacity(snippets.len());
        for snippet in snippets {
            if snippet.id.is_empty() || !seen_ids.insert(snippet.id.as_str()) {
                return Err(invalid("Snippet kimlikleri benzersiz olmalı."));
            }
            let trigger_len = snippet.trigger.trim().chars().count();
            if trigger_len == 0 || trigger_len > MAX_TRIGGER_CHARS {
                return Err(invalid(format!(
                    "Snippet tetikleyicisi 1–{MAX_TRIGGER_CHARS} karakter olmalı."
                )));
            }
            if !snippet.trigger.chars().any(char::is_alphanumeric) {
                return Err(invalid(
                    "Snippet tetikleyicisi en az bir harf veya rakam içermeli.",
                ));
            }
            if snippet.expansion.chars().count() > MAX_EXPANSION_CHARS {
                return Err(invalid(format!(
                    "Snippet metni en fazla {MAX_EXPANSION_CHARS} karakter olabilir."
                )));
            }
        }
        Ok(())
    }
}

/// Outcome of loading settings from disk, so the caller can tell the user
/// when an unreadable file was set aside.
#[derive(Debug)]
pub enum LoadOutcome {
    Loaded(Settings),
    FirstRun(Settings),
    Recovered { settings: Settings, reason: String },
}

/// Reads and writes `settings.json` in the app config directory.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    pub fn load(&self) -> LoadOutcome {
        let raw = match fs::read_to_string(self.path()) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return LoadOutcome::FirstRun(Settings::default());
            }
            Err(err) => {
                return LoadOutcome::Recovered {
                    settings: Settings::default(),
                    reason: err.to_string(),
                };
            }
        };
        let parsed = serde_json::from_str::<Settings>(&raw)
            .map_err(|err| err.to_string())
            .and_then(|settings| {
                settings
                    .validate()
                    .map(|()| settings)
                    .map_err(|err| err.message)
            });
        match parsed {
            Ok(mut settings) => {
                settings.version = SETTINGS_VERSION;
                LoadOutcome::Loaded(settings)
            }
            Err(reason) => {
                if let Err(err) = fs::write(self.dir.join(INVALID_SETTINGS_FILE), raw) {
                    tracing::warn!(%err, "could not preserve invalid settings file");
                }
                LoadOutcome::Recovered {
                    settings: Settings::default(),
                    reason,
                }
            }
        }
    }

    /// Atomic write: temp file in the same directory, fsync, rename.
    pub fn save(&self, settings: &Settings) -> AppResult<()> {
        fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(settings)
            .map_err(|err| AppError::new(ErrorCode::Internal, err.to_string()))?;
        let tmp = self.dir.join(format!("{SETTINGS_FILE}.tmp"));
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, self.path())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(id: &str, trigger: &str) -> Snippet {
        Snippet {
            id: id.into(),
            trigger: trigger.into(),
            expansion: "x".into(),
            match_mode: SnippetMatch::Exact,
            enabled: true,
        }
    }

    #[test]
    fn defaults_are_valid_and_launch_at_login_is_off() {
        let settings = Settings::default();
        assert!(settings.validate().is_ok());
        assert!(!settings.general.launch_at_login);
        assert!(!settings.general.auto_enter);
        assert_eq!(settings.general.insert_method, InsertMethod::Paste);
        assert!(settings.updates.auto_check);
        assert!(!settings.updates.auto_install);
        assert_eq!(settings.transcription.language, "auto");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let parsed: Settings = serde_json::from_str(r#"{"general":{"autoEnter":true}}"#).unwrap();
        assert!(parsed.general.auto_enter);
        assert!(parsed.general.restore_clipboard);
        assert_eq!(parsed.shortcut.accelerator, DEFAULT_ACCELERATOR);
    }

    #[test]
    fn rejects_out_of_range_tap_threshold() {
        let mut settings = Settings::default();
        settings.recording.tap_threshold_ms = 50;
        assert_eq!(
            settings.validate().unwrap_err().code,
            ErrorCode::InvalidSettings
        );
    }

    #[test]
    fn rejects_unsupported_language() {
        let mut settings = Settings::default();
        settings.transcription.language = "xx".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_snippet_ids() {
        let mut settings = Settings::default();
        settings.text.snippets = vec![snippet("a", "imzam"), snippet("a", "adresim")];
        assert!(settings.validate().is_err());
    }

    #[test]
    fn rejects_punctuation_only_trigger() {
        let mut settings = Settings::default();
        settings.text.snippets = vec![snippet("a", "...")];
        assert!(settings.validate().is_err());
    }

    #[test]
    fn inactive_engine_fields_do_not_block_saving() {
        let mut settings = Settings::default();
        settings.transcription.openai.base_url = "not a url".into();
        settings.transcription.engine = EngineKind::Mlx;
        assert!(settings.validate().is_ok());
        settings.transcription.engine = EngineKind::Openai;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn plain_http_is_only_allowed_for_local_hosts() {
        assert!(validate_base_url("http://localhost:8000/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8000/v1").is_ok());
        assert!(validate_base_url("http://192.168.1.20:8000/v1").is_ok());
        assert!(validate_base_url("http://monster.local:8000/v1").is_ok());
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("http://api.example.com/v1").is_err());
        assert!(validate_base_url("ftp://localhost/v1").is_err());
    }

    #[test]
    fn relative_model_path_is_rejected() {
        let mut settings = Settings::default();
        settings.transcription.whisper_cpp.model_path = Some("models/x.bin".into());
        assert!(settings.validate().is_err());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());
        let mut settings = Settings::default();
        settings.general.auto_enter = true;
        settings.text.snippets = vec![snippet("1", "imzam")];
        store.save(&settings).unwrap();
        match store.load() {
            LoadOutcome::Loaded(loaded) => assert_eq!(loaded, settings),
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn corrupt_file_is_set_aside_and_defaults_used() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SETTINGS_FILE), "{ not json").unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());
        assert!(matches!(store.load(), LoadOutcome::Recovered { .. }));
        assert!(dir.path().join(INVALID_SETTINGS_FILE).exists());
    }

    #[test]
    fn first_run_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());
        assert!(matches!(store.load(), LoadOutcome::FirstRun(_)));
    }
}
