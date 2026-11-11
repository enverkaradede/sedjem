//! Dictation service: wires the press controller, microphone capture, the
//! active engine, post-processing and text injection together.
//!
//! I/O lives behind small traits (`AudioSource`, `EngineProvider`,
//! `TextOutput`, `DictationEvents`) so the whole press → record → transcribe
//! → paste flow is unit-tested with fakes. Heavy work (stopping the device,
//! decoding, pasting) always runs on a worker thread, never under the state
//! lock and never on the thread that delivered the input.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::audio::capture::{
    CaptureConfig, CaptureError, CaptureResult, CaptureSignal, Recording, SignalFn,
};
use crate::controller::{AutoStopReason, Command, Controller, Input, InputSource, Latch, Phase};
use crate::engine::manager::EngineManager;
use crate::engine::{EngineError, StreamingSession, TranscribeOptions, Transcriber};
use crate::error::{AppError, ErrorCode};
use crate::settings::{InsertMethod, Settings};
use crate::text;

/// Shorter than this much detected speech is treated as an accidental press.
const MIN_SPEECH: Duration = Duration::from_millis(250);
/// How long a finished recording waits for an engine that is still loading.
const ENGINE_WAIT: Duration = Duration::from_secs(90);
const HISTORY_LIMIT: usize = 20;

// ---------------------------------------------------------------- traits --

pub trait ActiveRecording: Send {
    fn stop(self: Box<Self>) -> Result<CaptureResult, CaptureError>;
}

pub trait AudioSource: Send + Sync {
    fn start(
        &self,
        config: CaptureConfig,
        stream: Option<Box<dyn StreamingSession>>,
        on_signal: SignalFn,
    ) -> Result<Box<dyn ActiveRecording>, CaptureError>;
}

pub trait EngineProvider: Send + Sync {
    fn wait_ready(&self, timeout: Duration) -> Result<Arc<dyn Transcriber>, EngineError>;
}

pub trait TextOutput: Send + Sync {
    fn insert(&self, text: &str, options: &InsertOptions) -> Result<(), String>;
    fn copy(&self, text: &str) -> Result<(), String>;
}

/// How and with what extras a transcript is put into the focused app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertOptions {
    pub method: InsertMethod,
    pub press_enter: bool,
    pub restore_clipboard: bool,
}

impl InsertOptions {
    pub fn from_settings(general: &crate::settings::GeneralSettings) -> Self {
        Self {
            method: general.insert_method,
            press_enter: general.auto_enter,
            restore_clipboard: general.restore_clipboard,
        }
    }
}

pub trait DictationEvents: Send + Sync {
    fn state(&self, payload: &StatePayload);
    fn level(&self, value: f32);
    fn partial(&self, text: &str);
    fn result(&self, payload: &ResultPayload);
    fn no_speech(&self);
    fn busy(&self);
    fn error(&self, error: &AppError);
}

// -------------------------------------------------------------- payloads --

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PhaseName {
    Idle,
    Recording,
    Processing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatePayload {
    pub phase: PhaseName,
    pub latch: Option<Latch>,
    pub source: Option<InputSource>,
}

impl From<Phase> for StatePayload {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Idle => Self {
                phase: PhaseName::Idle,
                latch: None,
                source: None,
            },
            Phase::Recording { source, latch, .. } => Self {
                phase: PhaseName::Recording,
                latch: Some(latch),
                source: Some(source),
            },
            Phase::Processing => Self {
                phase: PhaseName::Processing,
                latch: None,
                source: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPayload {
    pub text: String,
    pub audio_ms: u64,
    pub latency_ms: u64,
    pub language: Option<String>,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: u64,
    pub text: String,
    pub created_at_ms: u64,
    pub language: Option<String>,
}

// ------------------------------------------------------- real adapters --

/// cpal-backed microphone.
pub struct Microphone;

impl ActiveRecording for Recording {
    fn stop(self: Box<Self>) -> Result<CaptureResult, CaptureError> {
        Recording::stop(*self)
    }
}

impl AudioSource for Microphone {
    fn start(
        &self,
        config: CaptureConfig,
        stream: Option<Box<dyn StreamingSession>>,
        on_signal: SignalFn,
    ) -> Result<Box<dyn ActiveRecording>, CaptureError> {
        Ok(Box::new(Recording::start(config, stream, on_signal)?))
    }
}

impl EngineProvider for EngineManager {
    fn wait_ready(&self, timeout: Duration) -> Result<Arc<dyn Transcriber>, EngineError> {
        EngineManager::wait_ready(self, timeout)
    }
}

/// Pastes into the focused app via the platform layer.
pub struct SystemTextOutput;

impl TextOutput for SystemTextOutput {
    fn insert(&self, text: &str, options: &InsertOptions) -> Result<(), String> {
        match options.method {
            InsertMethod::Paste => {
                crate::platform::paste_text(text, options.press_enter, options.restore_clipboard)
            }
            InsertMethod::Type => crate::platform::type_text(text, options.press_enter),
        }
    }
    fn copy(&self, text: &str) -> Result<(), String> {
        crate::platform::copy_text(text)
    }
}

// --------------------------------------------------------------- service --

struct State {
    controller: Controller,
    recording: Option<(u64, Box<dyn ActiveRecording>)>,
    next_recording_id: u64,
}

pub struct Dictation {
    state: Mutex<State>,
    /// User inputs are processed strictly in arrival order on one thread:
    /// a quick tap's release must never overtake its press.
    inputs: Sender<Input>,
    settings: Arc<RwLock<Settings>>,
    engines: Arc<dyn EngineProvider>,
    audio: Arc<dyn AudioSource>,
    output: Arc<dyn TextOutput>,
    events: Arc<dyn DictationEvents>,
    history: Mutex<VecDeque<HistoryEntry>>,
    me: Weak<Dictation>,
}

/// What a finished recording turned into.
struct Completed {
    text: String,
    language: Option<String>,
    audio: Duration,
    latency: Duration,
    insert_error: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn engine_error(err: EngineError) -> AppError {
    let code = match err {
        EngineError::NotConfigured(_) | EngineError::Timeout => ErrorCode::EngineNotReady,
        _ => ErrorCode::Engine,
    };
    AppError::new(code, err.to_string())
}

impl Dictation {
    pub fn new(
        settings: Arc<RwLock<Settings>>,
        engines: Arc<dyn EngineProvider>,
        audio: Arc<dyn AudioSource>,
        output: Arc<dyn TextOutput>,
        events: Arc<dyn DictationEvents>,
    ) -> Arc<Self> {
        let tap = Self::tap_threshold(&settings);
        let (inputs, queue) = mpsc::channel::<Input>();
        let dictation = Arc::new_cyclic(|me| Self {
            state: Mutex::new(State {
                controller: Controller::new(tap),
                recording: None,
                next_recording_id: 1,
            }),
            inputs,
            settings,
            engines,
            audio,
            output,
            events,
            history: Mutex::new(VecDeque::with_capacity(HISTORY_LIMIT)),
            me: me.clone(),
        });
        let me = Arc::downgrade(&dictation);
        let spawned = thread::Builder::new()
            .name("sedjem-input".into())
            .spawn(move || {
                // Ends when the Dictation (and with it the sender) is dropped.
                while let Ok(input) = queue.recv() {
                    match me.upgrade() {
                        Some(dictation) => dictation.handle(input),
                        None => break,
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "could not start dictation input thread");
        }
        dictation
    }

    /// Queues a user input with the timestamp of the moment it happened.
    fn submit(&self, input: Input) {
        if self.inputs.send(input).is_err() {
            tracing::error!("dictation input queue closed");
        }
    }

    fn tap_threshold(settings: &RwLock<Settings>) -> Duration {
        let ms = settings
            .read()
            .map(|s| s.recording.tap_threshold_ms)
            .unwrap_or(300);
        Duration::from_millis(u64::from(ms))
    }

    fn settings_snapshot(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn press(&self, source: InputSource) {
        self.submit(Input::Press {
            source,
            at: Instant::now(),
        });
    }

    pub fn release(&self, source: InputSource) {
        self.submit(Input::Release {
            source,
            at: Instant::now(),
        });
    }

    pub fn cancel(&self) {
        self.submit(Input::Cancel);
    }

    pub fn current_state(&self) -> StatePayload {
        self.state
            .lock()
            .map(|s| s.controller.phase().into())
            .unwrap_or(StatePayload {
                phase: PhaseName::Idle,
                latch: None,
                source: None,
            })
    }

    pub fn history(&self) -> Vec<HistoryEntry> {
        self.history
            .lock()
            .map(|h| h.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear_history(&self) {
        if let Ok(mut history) = self.history.lock() {
            history.clear();
        }
    }

    pub fn handle(&self, input: Input) {
        let Ok(mut state) = self.state.lock() else {
            tracing::error!("dictation state lock poisoned");
            return;
        };
        self.handle_locked(&mut state, input);
    }

    /// Applies a capture-originated input only if it belongs to the recording
    /// that is still active, so a late signal cannot stop a newer recording.
    fn handle_if_current(&self, recording_id: u64, input: Input) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.recording.as_ref().map(|(id, _)| *id) == Some(recording_id) {
            self.handle_locked(&mut state, input);
        }
    }

    fn handle_locked(&self, state: &mut State, input: Input) {
        state
            .controller
            .set_tap_threshold(Self::tap_threshold(&self.settings));
        match state.controller.handle(input) {
            Command::StartRecording => {
                if let Err(err) = self.start_recording(state) {
                    state.controller.reset();
                    self.events.error(&err);
                }
            }
            Command::StopAndTranscribe => match state.recording.take() {
                Some((_, recording)) => self.spawn_finish(recording),
                None => {
                    state.controller.handle(Input::ProcessingFinished);
                }
            },
            Command::DiscardRecording => {
                if let Some((_, recording)) = state.recording.take() {
                    Self::spawn_discard(recording);
                }
            }
            Command::Busy => self.events.busy(),
            Command::LatchChanged(_) | Command::None => {}
        }
        self.events.state(&state.controller.phase().into());
    }

    fn start_recording(&self, state: &mut State) -> Result<(), AppError> {
        let settings = self.settings_snapshot();
        let options = TranscribeOptions::from_settings(
            &settings.transcription.language,
            &settings.transcription.initial_prompt,
        );
        // Only engines that are already warm get a live stream; a loading
        // engine must never delay the microphone opening.
        let stream = self
            .engines
            .wait_ready(Duration::ZERO)
            .ok()
            .filter(|engine| engine.supports_streaming())
            .and_then(|engine| match engine.open_stream(&options) {
                Ok(session) => Some(session),
                Err(err) => {
                    tracing::warn!(%err, "streaming session unavailable; using batch");
                    None
                }
            });

        let id = state.next_recording_id;
        state.next_recording_id += 1;
        let me = self.me.clone();
        let on_signal: SignalFn = Arc::new(move |signal| {
            if let Some(dictation) = me.upgrade() {
                dictation.on_capture_signal(id, signal);
            }
        });
        let recording_settings = &settings.recording;
        let config = CaptureConfig {
            device_name: recording_settings.input_device.clone(),
            silence_timeout: recording_settings
                .silence_auto_stop
                .then(|| Duration::from_millis(u64::from(recording_settings.silence_timeout_ms))),
            max_duration: Duration::from_secs(u64::from(recording_settings.max_duration_s)),
        };
        let recording = self
            .audio
            .start(config, stream, on_signal)
            .map_err(|err| AppError::new(ErrorCode::Audio, err.to_string()))?;
        state.recording = Some((id, recording));
        Ok(())
    }

    fn on_capture_signal(&self, recording_id: u64, signal: CaptureSignal) {
        match signal {
            CaptureSignal::Level(value) => self.events.level(value),
            CaptureSignal::Partial(text) => self.events.partial(&text),
            CaptureSignal::SilenceTimeout => {
                self.handle_if_current(recording_id, Input::AutoStop(AutoStopReason::Silence));
            }
            CaptureSignal::MaxDurationReached => {
                self.handle_if_current(recording_id, Input::AutoStop(AutoStopReason::MaxDuration));
            }
            CaptureSignal::Failed(message) => {
                self.handle_if_current(recording_id, Input::Cancel);
                self.events.error(&AppError::new(ErrorCode::Audio, message));
            }
        }
    }

    fn spawn_discard(recording: Box<dyn ActiveRecording>) {
        let spawned = thread::Builder::new()
            .name("sedjem-discard".into())
            .spawn(move || {
                if let Err(err) = recording.stop() {
                    tracing::warn!(%err, "error while discarding recording");
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "could not spawn discard thread");
        }
    }

    fn spawn_finish(&self, recording: Box<dyn ActiveRecording>) {
        let me = self.me.clone();
        let spawned = thread::Builder::new()
            .name("sedjem-transcribe".into())
            .spawn(move || {
                let Some(dictation) = me.upgrade() else {
                    return;
                };
                dictation.finish(recording);
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "could not spawn transcription thread");
            self.events.error(&AppError::new(
                ErrorCode::Internal,
                "İş parçacığı başlatılamadı.",
            ));
            if let Ok(mut state) = self.state.lock() {
                state.controller.handle(Input::ProcessingFinished);
                self.events.state(&state.controller.phase().into());
            }
        }
    }

    fn finish(&self, recording: Box<dyn ActiveRecording>) {
        match self.process(recording) {
            Ok(Some(done)) => {
                let inserted = done.insert_error.is_none();
                self.remember(&done);
                self.events.result(&ResultPayload {
                    text: done.text.clone(),
                    audio_ms: done.audio.as_millis() as u64,
                    latency_ms: done.latency.as_millis() as u64,
                    language: done.language.clone(),
                    inserted,
                });
                if let Some(message) = done.insert_error {
                    let copied = self.output.copy(&done.text).is_ok();
                    let suffix = if copied {
                        " Metin panoya kopyalandı."
                    } else {
                        ""
                    };
                    self.events.error(&AppError::new(
                        ErrorCode::Inject,
                        format!("{message}{suffix}"),
                    ));
                }
            }
            Ok(None) => self.events.no_speech(),
            Err(err) => self.events.error(&err),
        }
        self.handle(Input::ProcessingFinished);
    }

    fn process(&self, recording: Box<dyn ActiveRecording>) -> Result<Option<Completed>, AppError> {
        let capture = recording
            .stop()
            .map_err(|err| AppError::new(ErrorCode::Audio, err.to_string()))?;
        if capture.stats.is_dead_air() {
            return Err(AppError::new(
                ErrorCode::Permission,
                "Mikrofondan hiç ses gelmedi. Mikrofon iznini kontrol edin.",
            ));
        }
        if capture.stats.speech < MIN_SPEECH {
            return Ok(None);
        }
        let settings = self.settings_snapshot();
        let options = TranscribeOptions::from_settings(
            &settings.transcription.language,
            &settings.transcription.initial_prompt,
        );
        let started = Instant::now();
        let transcript = match capture.stream {
            Some(stream) => stream.finish(),
            None => self
                .engines
                .wait_ready(ENGINE_WAIT)
                .and_then(|engine| engine.transcribe(&capture.audio, &options)),
        }
        .map_err(engine_error)?;
        let latency = started.elapsed();

        let text = text::process(&transcript.text, &settings.text);
        // Lengths only: transcripts are personal data and never logged.
        tracing::info!(
            audio_ms = capture.stats.total.as_millis() as u64,
            latency_ms = latency.as_millis() as u64,
            chars = text.chars().count(),
            "transcribed"
        );
        if text.is_empty() {
            return Ok(None);
        }
        let insert_error = self
            .output
            .insert(&text, &InsertOptions::from_settings(&settings.general))
            .err();
        Ok(Some(Completed {
            text,
            language: transcript.language,
            audio: capture.stats.total,
            latency,
            insert_error,
        }))
    }

    fn remember(&self, done: &Completed) {
        if !self
            .settings
            .read()
            .map(|s| s.general.keep_history)
            .unwrap_or(false)
        {
            return;
        }
        if let Ok(mut history) = self.history.lock() {
            let id = history.back().map_or(1, |last| last.id + 1);
            if history.len() == HISTORY_LIMIT {
                history.pop_front();
            }
            history.push_back(HistoryEntry {
                id,
                text: done.text.clone(),
                created_at_ms: now_ms(),
                language: done.language.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::level::SpeechStats;
    use crate::settings::EngineKind;
    use std::sync::mpsc::{self, Receiver, Sender};

    // ---- fakes ----

    struct FakeRecording {
        stats: SpeechStats,
    }

    impl ActiveRecording for FakeRecording {
        fn stop(self: Box<Self>) -> Result<CaptureResult, CaptureError> {
            Ok(CaptureResult {
                audio: vec![0.1; 16_000],
                stats: self.stats,
                stream: None,
            })
        }
    }

    struct FakeMic {
        stats: SpeechStats,
        signals: Mutex<Vec<SignalFn>>,
    }

    impl AudioSource for FakeMic {
        fn start(
            &self,
            _config: CaptureConfig,
            _stream: Option<Box<dyn StreamingSession>>,
            on_signal: SignalFn,
        ) -> Result<Box<dyn ActiveRecording>, CaptureError> {
            self.signals.lock().unwrap().push(on_signal);
            Ok(Box::new(FakeRecording { stats: self.stats }))
        }
    }

    struct FakeEngine(&'static str);

    impl Transcriber for FakeEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Wyoming
        }
        fn describe(&self) -> String {
            "fake".into()
        }
        fn transcribe(
            &self,
            _audio: &[f32],
            _o: &TranscribeOptions,
        ) -> Result<crate::engine::Transcript, EngineError> {
            Ok(crate::engine::Transcript {
                text: self.0.into(),
                language: Some("tr".into()),
            })
        }
    }

    struct FakeEngines(Arc<dyn Transcriber>);

    impl EngineProvider for FakeEngines {
        fn wait_ready(&self, _timeout: Duration) -> Result<Arc<dyn Transcriber>, EngineError> {
            Ok(self.0.clone())
        }
    }

    #[derive(Default)]
    struct FakeOutput {
        inserted: Mutex<Vec<(String, bool)>>,
        methods: Mutex<Vec<InsertMethod>>,
        fail: bool,
    }

    impl TextOutput for FakeOutput {
        fn insert(&self, text: &str, options: &InsertOptions) -> Result<(), String> {
            if self.fail {
                return Err("no accessibility".into());
            }
            self.inserted
                .lock()
                .unwrap()
                .push((text.to_owned(), options.press_enter));
            self.methods.lock().unwrap().push(options.method);
            Ok(())
        }
        fn copy(&self, _text: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Ev {
        State(PhaseName),
        Result(String),
        NoSpeech,
        Busy,
        Error(ErrorCode),
    }

    struct Events(Mutex<Sender<Ev>>);

    impl DictationEvents for Events {
        fn state(&self, p: &StatePayload) {
            let _ = self.0.lock().unwrap().send(Ev::State(p.phase));
        }
        fn level(&self, _v: f32) {}
        fn partial(&self, _t: &str) {}
        fn result(&self, p: &ResultPayload) {
            let _ = self.0.lock().unwrap().send(Ev::Result(p.text.clone()));
        }
        fn no_speech(&self) {
            let _ = self.0.lock().unwrap().send(Ev::NoSpeech);
        }
        fn busy(&self) {
            let _ = self.0.lock().unwrap().send(Ev::Busy);
        }
        fn error(&self, e: &AppError) {
            let _ = self.0.lock().unwrap().send(Ev::Error(e.code));
        }
    }

    fn speech(ms: u64) -> SpeechStats {
        SpeechStats {
            total: Duration::from_millis(ms + 200),
            speech: Duration::from_millis(ms),
            peak_rms: 0.2,
        }
    }

    struct Harness {
        dictation: Arc<Dictation>,
        output: Arc<FakeOutput>,
        mic: Arc<FakeMic>,
        events: Receiver<Ev>,
    }

    fn harness(
        stats: SpeechStats,
        transcript: &'static str,
        settings: Settings,
        fail_insert: bool,
    ) -> Harness {
        let (tx, events) = mpsc::channel();
        let output = Arc::new(FakeOutput {
            fail: fail_insert,
            ..Default::default()
        });
        let mic = Arc::new(FakeMic {
            stats,
            signals: Mutex::new(Vec::new()),
        });
        let dictation = Dictation::new(
            Arc::new(RwLock::new(settings)),
            Arc::new(FakeEngines(Arc::new(FakeEngine(transcript)))),
            mic.clone(),
            output.clone(),
            Arc::new(Events(Mutex::new(tx))),
        );
        Harness {
            dictation,
            output,
            mic,
            events,
        }
    }

    /// Collects events until the dictation returns to idle after processing.
    fn drain_until_idle(rx: &Receiver<Ev>) -> Vec<Ev> {
        let mut seen = Vec::new();
        let mut processing_seen = false;
        while let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) {
            if ev == Ev::State(PhaseName::Processing) {
                processing_seen = true;
            }
            let done = processing_seen && ev == Ev::State(PhaseName::Idle);
            seen.push(ev);
            if done {
                break;
            }
        }
        seen
    }

    fn hold(d: &Dictation) {
        let t0 = Instant::now();
        d.handle(Input::Press {
            source: InputSource::Hotkey,
            at: t0,
        });
        d.handle(Input::Release {
            source: InputSource::Hotkey,
            at: t0 + Duration::from_secs(2),
        });
    }

    #[test]
    fn hold_to_talk_transcribes_and_inserts() {
        let mut settings = Settings::default();
        settings.general.auto_enter = true;
        let h = harness(speech(1_500), "merhaba dünya", settings, false);
        hold(&h.dictation);
        let events = drain_until_idle(&h.events);
        assert!(
            events.contains(&Ev::Result("merhaba dünya".into())),
            "{events:?}"
        );
        assert_eq!(
            h.output.inserted.lock().unwrap().as_slice(),
            &[("merhaba dünya".to_owned(), true)]
        );
        assert_eq!(h.dictation.history().len(), 1);
        assert_eq!(
            h.output.methods.lock().unwrap().as_slice(),
            &[InsertMethod::Paste]
        );
    }

    #[test]
    fn direct_typing_method_reaches_the_output() {
        let mut settings = Settings::default();
        settings.general.insert_method = InsertMethod::Type;
        let h = harness(speech(1_500), "doğrudan", settings, false);
        hold(&h.dictation);
        drain_until_idle(&h.events);
        assert_eq!(
            h.output.methods.lock().unwrap().as_slice(),
            &[InsertMethod::Type]
        );
    }

    #[test]
    fn tap_then_tap_runs_in_toggle_mode() {
        let h = harness(speech(1_500), "toggle", Settings::default(), false);
        let t0 = Instant::now();
        h.dictation.handle(Input::Press {
            source: InputSource::Panel,
            at: t0,
        });
        h.dictation.handle(Input::Release {
            source: InputSource::Panel,
            at: t0 + Duration::from_millis(100),
        });
        assert_eq!(h.dictation.current_state().latch, Some(Latch::Toggle));
        h.dictation.handle(Input::Press {
            source: InputSource::Panel,
            at: t0 + Duration::from_secs(3),
        });
        let events = drain_until_idle(&h.events);
        assert!(events.contains(&Ev::Result("toggle".into())));
    }

    #[test]
    fn too_little_speech_is_reported_as_no_speech() {
        let h = harness(speech(100), "ignored", Settings::default(), false);
        hold(&h.dictation);
        let events = drain_until_idle(&h.events);
        assert!(events.contains(&Ev::NoSpeech));
        assert!(h.output.inserted.lock().unwrap().is_empty());
    }

    #[test]
    fn dead_air_reports_permission_error() {
        let stats = SpeechStats {
            total: Duration::from_secs(2),
            speech: Duration::ZERO,
            peak_rms: 0.0,
        };
        let h = harness(stats, "x", Settings::default(), false);
        hold(&h.dictation);
        assert!(drain_until_idle(&h.events).contains(&Ev::Error(ErrorCode::Permission)));
    }

    #[test]
    fn hallucination_only_transcript_inserts_nothing() {
        let h = harness(speech(1_000), "Altyazı M.K.", Settings::default(), false);
        hold(&h.dictation);
        assert!(drain_until_idle(&h.events).contains(&Ev::NoSpeech));
    }

    #[test]
    fn insert_failure_still_reports_result_and_error() {
        let h = harness(speech(1_000), "metin", Settings::default(), true);
        hold(&h.dictation);
        let events = drain_until_idle(&h.events);
        assert!(events.contains(&Ev::Result("metin".into())));
        assert!(events.contains(&Ev::Error(ErrorCode::Inject)));
    }

    #[test]
    fn stale_silence_signal_does_not_stop_a_newer_recording() {
        let h = harness(speech(1_000), "x", Settings::default(), false);
        hold(&h.dictation);
        drain_until_idle(&h.events);
        // Second recording, toggled.
        let t0 = Instant::now();
        h.dictation.handle(Input::Press {
            source: InputSource::Panel,
            at: t0,
        });
        h.dictation.handle(Input::Release {
            source: InputSource::Panel,
            at: t0 + Duration::from_millis(50),
        });
        let first_recording_signal = h.mic.signals.lock().unwrap()[0].clone();
        first_recording_signal(CaptureSignal::SilenceTimeout);
        assert_eq!(h.dictation.current_state().phase, PhaseName::Recording);
        // The current recording's own silence signal does stop it.
        let current_signal = h.mic.signals.lock().unwrap()[1].clone();
        current_signal(CaptureSignal::SilenceTimeout);
        assert!(drain_until_idle(&h.events).contains(&Ev::Result("x".into())));
    }

    #[test]
    fn queued_quick_tap_keeps_order_and_latches_toggle() {
        let h = harness(speech(1_000), "sıralı", Settings::default(), false);
        // Back-to-back press/release through the queue: the release must be
        // handled after the press and, being quick, latch toggle mode.
        h.dictation.press(InputSource::Panel);
        h.dictation.release(InputSource::Panel);
        let mut latched = false;
        while let Ok(ev) = h.events.recv_timeout(Duration::from_secs(5)) {
            if ev == Ev::State(PhaseName::Recording)
                && h.dictation.current_state().latch == Some(Latch::Toggle)
            {
                latched = true;
                break;
            }
        }
        assert!(latched, "quick tap through the queue should latch toggle");
        h.dictation.press(InputSource::Panel);
        assert!(drain_until_idle(&h.events).contains(&Ev::Result("sıralı".into())));
    }

    #[test]
    fn press_while_processing_reports_busy() {
        let h = harness(speech(1_000), "x", Settings::default(), false);
        let d = h.dictation.clone();
        // Hold the state lock so processing cannot finish before the press.
        let t0 = Instant::now();
        d.handle(Input::Press {
            source: InputSource::Panel,
            at: t0,
        });
        {
            let mut state = d.state.lock().unwrap();
            d.handle_locked(
                &mut state,
                Input::Release {
                    source: InputSource::Panel,
                    at: t0 + Duration::from_secs(1),
                },
            );
            d.handle_locked(
                &mut state,
                Input::Press {
                    source: InputSource::Panel,
                    at: t0 + Duration::from_secs(2),
                },
            );
        }
        assert!(drain_until_idle(&h.events).contains(&Ev::Busy));
    }
}
