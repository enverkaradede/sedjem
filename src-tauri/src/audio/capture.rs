//! Microphone capture on a dedicated thread.
//!
//! The cpal `Stream` is created, owned and dropped on one worker thread so it
//! never crosses threads. The real-time audio callback only downmixes to mono
//! and forwards the chunk over a channel; resampling, level metering, silence
//! detection and streaming-engine feeding all happen on the worker, never in
//! the callback.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use super::level::{meter_value, LevelTracker, SpeechStats};
use super::resample::MonoResampler;
use crate::engine::StreamingSession;

/// How long `start` waits for the device to open before giving up.
const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// Worker wake-up period when no audio arrives (bounds stop latency).
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// UI level meter refresh period (~30 fps).
const LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, thiserror::Error)]
pub enum CaptureError {
    #[error("Mikrofon bulunamadı.")]
    NoDevice,
    #[error("Seçili mikrofon bulunamadı: {0}")]
    DeviceNotFound(String),
    #[error("Mikrofon açılamadı: {0}")]
    Open(String),
    #[error("Kayıt sırasında hata: {0}")]
    Stream(String),
    #[error("Ses dönüştürme hatası: {0}")]
    Resample(String),
}

/// Asynchronous notifications from the capture worker.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureSignal {
    Level(f32),
    Partial(String),
    SilenceTimeout,
    MaxDurationReached,
    Failed(String),
}

pub type SignalFn = Arc<dyn Fn(CaptureSignal) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub device_name: Option<String>,
    /// `Some` enables silence auto-stop with this timeout.
    pub silence_timeout: Option<Duration>,
    pub max_duration: Duration,
}

/// Everything a finished recording produced.
pub struct CaptureResult {
    /// 16 kHz mono samples.
    pub audio: Vec<f32>,
    pub stats: SpeechStats,
    /// Streaming session fed during capture, returned for `finish()`.
    pub stream: Option<Box<dyn StreamingSession>>,
}

pub struct Recording {
    stop_tx: Sender<()>,
    join: JoinHandle<Result<CaptureResult, CaptureError>>,
}

/// Lists input device names for the settings UI.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => devices
            .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "could not enumerate input devices");
            Vec::new()
        }
    }
}

fn pick_device(name: Option<&str>) -> Result<cpal::Device, CaptureError> {
    let host = cpal::default_host();
    match name {
        None => host.default_input_device().ok_or(CaptureError::NoDevice),
        Some(wanted) => host
            .input_devices()
            .map_err(|err| CaptureError::Open(err.to_string()))?
            .find(|d| {
                d.description()
                    .map(|desc| desc.name() == wanted)
                    .unwrap_or(false)
            })
            .ok_or_else(|| CaptureError::DeviceNotFound(wanted.to_owned())),
    }
}

enum WorkerMessage {
    Samples(Vec<f32>),
    Error(String),
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    tx: Sender<WorkerMessage>,
) -> Result<cpal::Stream, CaptureError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let error_tx = tx.clone();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _info| {
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| {
                        frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / channels as f32
                    })
                    .collect();
                // A closed receiver means the recording is over; dropping is correct.
                let _ = tx.send(WorkerMessage::Samples(mono));
            },
            move |err| {
                let _ = error_tx.send(WorkerMessage::Error(err.to_string()));
            },
            None,
        )
        .map_err(|err| CaptureError::Open(err.to_string()))
}

fn open_stream(
    device_name: Option<&str>,
    tx: Sender<WorkerMessage>,
) -> Result<(cpal::Stream, u32), CaptureError> {
    let device = pick_device(device_name)?;
    let supported = device
        .default_input_config()
        .map_err(|err| CaptureError::Open(err.to_string()))?;
    let sample_rate = supported.sample_rate();
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, config, tx)?,
        SampleFormat::I16 => build_stream::<i16>(&device, config, tx)?,
        SampleFormat::I32 => build_stream::<i32>(&device, config, tx)?,
        SampleFormat::U16 => build_stream::<u16>(&device, config, tx)?,
        SampleFormat::F64 => build_stream::<f64>(&device, config, tx)?,
        other => {
            return Err(CaptureError::Open(format!(
                "desteklenmeyen örnek biçimi: {other:?}"
            )))
        }
    };
    stream
        .play()
        .map_err(|err| CaptureError::Open(err.to_string()))?;
    Ok((stream, sample_rate))
}

/// Per-recording processing state living on the worker thread.
struct Pipeline {
    resampler: MonoResampler,
    tracker: LevelTracker,
    audio: Vec<f32>,
    stream: Option<Box<dyn StreamingSession>>,
    on_signal: SignalFn,
    config: CaptureConfig,
    last_level_emit: Instant,
    silence_signalled: bool,
    limit_signalled: bool,
}

impl Pipeline {
    fn ingest(&mut self, native: &[f32]) -> Result<(), CaptureError> {
        let resampled = self
            .resampler
            .push(native)
            .map_err(|err| CaptureError::Resample(err.to_string()))?;
        self.consume(&resampled);
        Ok(())
    }

    fn consume(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        self.tracker.push(samples);
        self.audio.extend_from_slice(samples);
        if let Some(stream) = self.stream.as_mut() {
            match stream.push(samples) {
                Ok(Some(partial)) => (self.on_signal)(CaptureSignal::Partial(partial)),
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(%err, "streaming engine rejected audio; falling back to batch");
                    self.stream = None;
                }
            }
        }
    }

    fn tick(&mut self) {
        if self.last_level_emit.elapsed() >= LEVEL_EMIT_INTERVAL {
            self.last_level_emit = Instant::now();
            (self.on_signal)(CaptureSignal::Level(meter_value(self.tracker.last_rms())));
        }
        if !self.limit_signalled && self.tracker.stats().total >= self.config.max_duration {
            self.limit_signalled = true;
            (self.on_signal)(CaptureSignal::MaxDurationReached);
        }
        if let Some(timeout) = self.config.silence_timeout {
            if !self.silence_signalled && self.tracker.silence_after_speech() >= timeout {
                self.silence_signalled = true;
                (self.on_signal)(CaptureSignal::SilenceTimeout);
            }
        }
    }

    fn finish(mut self) -> Result<CaptureResult, CaptureError> {
        let tail = self
            .resampler
            .finish()
            .map_err(|err| CaptureError::Resample(err.to_string()))?;
        self.consume(&tail);
        Ok(CaptureResult {
            audio: self.audio,
            stats: self.tracker.stats(),
            stream: self.stream,
        })
    }
}

fn run_worker(
    config: CaptureConfig,
    stream_session: Option<Box<dyn StreamingSession>>,
    on_signal: SignalFn,
    stop_rx: Receiver<()>,
    ready_tx: Sender<Result<(), CaptureError>>,
) -> Result<CaptureResult, CaptureError> {
    let (tx, rx) = mpsc::channel::<WorkerMessage>();
    let (stream, sample_rate) = match open_stream(config.device_name.as_deref(), tx) {
        Ok(opened) => opened,
        Err(err) => {
            let _ = ready_tx.send(Err(err.clone()));
            return Err(err);
        }
    };
    let resampler = match MonoResampler::new(sample_rate) {
        Ok(r) => r,
        Err(err) => {
            let err = CaptureError::Resample(err.to_string());
            let _ = ready_tx.send(Err(err.clone()));
            return Err(err);
        }
    };
    let _ = ready_tx.send(Ok(()));
    tracing::info!(sample_rate, "recording started");

    let mut pipeline = Pipeline {
        resampler,
        tracker: LevelTracker::default(),
        audio: Vec::new(),
        stream: stream_session,
        on_signal: on_signal.clone(),
        config,
        last_level_emit: Instant::now(),
        silence_signalled: false,
        limit_signalled: false,
    };

    loop {
        match stop_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(WorkerMessage::Samples(chunk)) => pipeline.ingest(&chunk)?,
            Ok(WorkerMessage::Error(message)) => {
                on_signal(CaptureSignal::Failed(message.clone()));
                return Err(CaptureError::Stream(message));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(CaptureError::Stream("ses akışı kapandı".into()))
            }
        }
        pipeline.tick();
    }

    // Stop the device first, then drain what the callback already delivered.
    drop(stream);
    while let Ok(message) = rx.try_recv() {
        if let WorkerMessage::Samples(chunk) = message {
            pipeline.ingest(&chunk)?;
        }
    }
    pipeline.finish()
}

impl Recording {
    /// Opens the microphone and starts capturing. Returns once the device is
    /// actually streaming, or with the reason it could not be opened.
    pub fn start(
        config: CaptureConfig,
        stream_session: Option<Box<dyn StreamingSession>>,
        on_signal: SignalFn,
    ) -> Result<Self, CaptureError> {
        let (stop_tx, stop_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("sedjem-capture".into())
            .spawn(move || run_worker(config, stream_session, on_signal, stop_rx, ready_tx))
            .map_err(|err| CaptureError::Open(err.to_string()))?;
        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { stop_tx, join }),
            Ok(Err(err)) => {
                let _ = join.join();
                Err(err)
            }
            Err(_) => {
                let _ = stop_tx.send(());
                Err(CaptureError::Open(
                    "mikrofon zamanında yanıt vermedi".into(),
                ))
            }
        }
    }

    /// Stops capture and returns the recorded audio.
    pub fn stop(self) -> Result<CaptureResult, CaptureError> {
        let _ = self.stop_tx.send(());
        self.join
            .join()
            .map_err(|_| CaptureError::Stream("kayıt iş parçacığı çöktü".into()))?
    }
}
