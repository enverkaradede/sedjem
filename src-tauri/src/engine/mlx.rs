//! MLX Whisper engine: a long-lived Python worker that keeps the model warm
//! on the Apple GPU. Speaks the JSON-line protocol in `resources/mlx_worker.py`.
//!
//! Reads go through a dedicated reader thread and a channel so every request
//! has a hard timeout; a worker that times out or dies is killed and lazily
//! respawned on the next request (one bounded restart per call).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;

use super::{EngineError, TranscribeOptions, Transcriber, Transcript};
use crate::settings::EngineKind;

/// Importing MLX and loading 1.6 GB of weights from disk on first launch.
const LOAD_TIMEOUT: Duration = Duration::from_secs(180);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// Base transcription allowance plus a per-second-of-audio budget.
const TRANSCRIBE_BASE_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIBE_TIMEOUT_PER_AUDIO_SECOND: f32 = 1.5;

#[derive(Debug, Deserialize)]
struct WorkerReply {
    id: Option<u64>,
    ok: bool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Parses one protocol line. Separate from I/O so it is unit-testable.
fn parse_reply(line: &str) -> Result<WorkerReply, EngineError> {
    serde_json::from_str(line)
        .map_err(|err| EngineError::Protocol(format!("worker yanıtı çözümlenemedi: {err}")))
}

/// Encodes 16 kHz f32 samples as base64 little-endian float32.
fn encode_audio(audio: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(audio.len() * 4);
    for sample in audio {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Worker {
    fn spawn(python: &Path, script: &Path, model_dir: &str) -> Result<Self, EngineError> {
        let mut child = Command::new(python)
            .arg("-u")
            .arg(script)
            .env("PYTHONIOENCODING", "utf-8")
            .env("HF_HUB_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                EngineError::Load(format!(
                    "Python başlatılamadı ({}): {err}",
                    python.display()
                ))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EngineError::Load("stdin alınamadı".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Load("stdout alınamadı".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| EngineError::Load("stderr alınamadı".into()))?;

        let (tx, lines) = mpsc::channel();
        thread::Builder::new()
            .name("sedjem-mlx-stdout".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|err| EngineError::Load(err.to_string()))?;
        thread::Builder::new()
            .name("sedjem-mlx-stderr".into())
            .spawn(move || {
                // Library diagnostics only; the worker never writes transcripts here.
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!(target: "sedjem::mlx_worker", "{line}");
                }
            })
            .map_err(|err| EngineError::Load(err.to_string()))?;

        let mut worker = Self {
            child,
            stdin,
            lines,
            next_id: 1,
        };
        let hello = worker.read_reply(STARTUP_TIMEOUT)?;
        if !hello.ok {
            worker.kill();
            return Err(EngineError::Load(
                hello
                    .error
                    .unwrap_or_else(|| "mlx-whisper içe aktarılamadı".into()),
            ));
        }
        let reply = worker.request(json!({ "op": "load", "model": model_dir }), LOAD_TIMEOUT)?;
        if !reply.ok {
            worker.kill();
            return Err(EngineError::Load(
                reply.error.unwrap_or_else(|| "model yüklenemedi".into()),
            ));
        }
        Ok(worker)
    }

    fn read_reply(&mut self, timeout: Duration) -> Result<WorkerReply, EngineError> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => parse_reply(&line),
            Err(RecvTimeoutError::Timeout) => Err(EngineError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(EngineError::Transcribe(
                "MLX worker beklenmedik şekilde kapandı".into(),
            )),
        }
    }

    fn request(
        &mut self,
        mut body: serde_json::Value,
        timeout: Duration,
    ) -> Result<WorkerReply, EngineError> {
        let id = self.next_id;
        self.next_id += 1;
        body["id"] = json!(id);
        let mut line = body.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|err| EngineError::Transcribe(format!("worker'a yazılamadı: {err}")))?;
        // Skip any stale reply left over from a request that timed out earlier.
        loop {
            let reply = self.read_reply(timeout)?;
            if reply.id == Some(id) {
                return Ok(reply);
            }
        }
    }

    fn kill(&mut self) {
        if let Err(err) = self.child.kill() {
            tracing::debug!(%err, "mlx worker already exited");
        }
        let _ = self.child.wait();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

pub struct MlxEngine {
    python: PathBuf,
    script: PathBuf,
    model_dir: String,
    worker: Mutex<Option<Worker>>,
}

impl MlxEngine {
    /// Starts the worker and loads the model; returns once it is warm.
    pub fn start(python: PathBuf, script: PathBuf, model_dir: String) -> Result<Self, EngineError> {
        if !python.is_file() {
            return Err(EngineError::NotConfigured(format!(
                "Python bulunamadı: {}. Ayarlardan MLX ortamını kurun.",
                python.display()
            )));
        }
        if !script.is_file() {
            return Err(EngineError::NotConfigured(format!(
                "worker betiği yok: {}",
                script.display()
            )));
        }
        if !Path::new(&model_dir).is_dir() {
            return Err(EngineError::NotConfigured(format!(
                "MLX model klasörü yok: {model_dir}"
            )));
        }
        let worker = Worker::spawn(&python, &script, &model_dir)?;
        tracing::info!(model = %model_dir, "mlx worker ready");
        Ok(Self {
            python,
            script,
            model_dir,
            worker: Mutex::new(Some(worker)),
        })
    }

    fn transcribe_once(
        worker: &mut Worker,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let seconds = audio.len() as f32 / crate::audio::resample::TARGET_SAMPLE_RATE as f32;
        let timeout = TRANSCRIBE_BASE_TIMEOUT
            + Duration::from_secs_f32(seconds * TRANSCRIBE_TIMEOUT_PER_AUDIO_SECOND);
        let body = json!({
            "op": "transcribe",
            "audio": encode_audio(audio),
            "language": options.language,
            "prompt": options.initial_prompt,
        });
        let reply = worker.request(body, timeout)?;
        if !reply.ok {
            return Err(EngineError::Transcribe(
                reply.error.unwrap_or_else(|| "bilinmeyen hata".into()),
            ));
        }
        Ok(Transcript {
            text: reply.text.unwrap_or_default(),
            language: reply.language,
        })
    }
}

impl Transcriber for MlxEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Mlx
    }

    fn describe(&self) -> String {
        let name = Path::new(&self.model_dir)
            .ancestors()
            .find_map(|p| {
                p.file_name()?
                    .to_str()?
                    .strip_prefix("models--")
                    .map(|n| n.replace("--", "/"))
            })
            .unwrap_or_else(|| self.model_dir.clone());
        format!("MLX · {name}")
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| EngineError::Transcribe("worker kilidi bozuk".into()))?;
        if guard.is_none() {
            *guard = Some(Worker::spawn(&self.python, &self.script, &self.model_dir)?);
        }
        let worker = guard
            .as_mut()
            .ok_or_else(|| EngineError::Transcribe("worker yok".into()))?;
        match Self::transcribe_once(worker, audio, options) {
            Ok(transcript) => Ok(transcript),
            Err(
                err
                @ (EngineError::Timeout | EngineError::Transcribe(_) | EngineError::Protocol(_)),
            ) => {
                // The worker may be wedged or dead; drop it so the next call respawns.
                tracing::warn!(%err, "mlx worker failed; it will be restarted on next use");
                *guard = None;
                Err(err)
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_float32_little_endian() {
        let encoded = encode_audio(&[1.0, -0.5]);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(
            bytes,
            [1.0f32.to_le_bytes(), (-0.5f32).to_le_bytes()].concat()
        );
    }

    #[test]
    fn parses_success_and_error_replies() {
        let ok = parse_reply(r#"{"id":3,"ok":true,"text":"merhaba","language":"tr"}"#).unwrap();
        assert_eq!(
            (ok.id, ok.ok, ok.text.as_deref()),
            (Some(3), true, Some("merhaba"))
        );
        let err = parse_reply(r#"{"id":4,"ok":false,"error":"boom"}"#).unwrap();
        assert_eq!(err.error.as_deref(), Some("boom"));
        assert!(parse_reply("not json").is_err());
    }
}
