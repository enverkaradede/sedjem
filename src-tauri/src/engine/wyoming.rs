//! Wyoming protocol ASR client (e.g. faster-whisper on a LAN server).
//!
//! Wire format: each event is a JSON header line, optionally followed by
//! `data_length` bytes of JSON data and `payload_length` bytes of payload.
//! One TCP connection per utterance: transcribe → audio-start → audio-chunk*
//! → audio-stop, then read until a `transcript` event arrives.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::{EngineError, TranscribeOptions, Transcriber, Transcript};
use crate::audio::resample::TARGET_SAMPLE_RATE;
use crate::audio::wav::pcm16_le;
use crate::settings::EngineKind;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// Overall budget for one utterance including server-side decoding.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK_SAMPLES: usize = 1_600; // 100 ms at 16 kHz
const PROTOCOL_VERSION: &str = "1.5.2";
/// Upper bound on a single header/data/payload we accept from the server.
const MAX_FIELD_BYTES: usize = 4 * 1024 * 1024;
/// Events tolerated before the transcript before declaring a protocol error.
const MAX_EVENTS_BEFORE_TRANSCRIPT: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Map<String, Value>,
    pub payload: Vec<u8>,
}

/// Serialises one event in the data_length form current servers emit.
pub fn encode_event(kind: &str, data: &Value, payload: &[u8]) -> Vec<u8> {
    let data_bytes = if data.is_null() {
        Vec::new()
    } else {
        data.to_string().into_bytes()
    };
    let mut header = json!({ "type": kind, "version": PROTOCOL_VERSION });
    if !data_bytes.is_empty() {
        header["data_length"] = json!(data_bytes.len());
    }
    if !payload.is_empty() {
        header["payload_length"] = json!(payload.len());
    }
    let mut out = header.to_string().into_bytes();
    out.push(b'\n');
    out.extend_from_slice(&data_bytes);
    out.extend_from_slice(payload);
    out
}

fn read_exact_len(reader: &mut impl Read, len: usize) -> Result<Vec<u8>, EngineError> {
    if len > MAX_FIELD_BYTES {
        return Err(EngineError::Protocol(format!("çok büyük alan: {len} bayt")));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .map_err(|err| EngineError::Connection(err.to_string()))?;
    Ok(buf)
}

/// Reads one event; accepts both inline `data` and the `data_length` form.
pub fn read_event(reader: &mut impl BufRead) -> Result<Event, EngineError> {
    let mut line = String::new();
    let read = reader
        .by_ref()
        .take(MAX_FIELD_BYTES as u64)
        .read_line(&mut line)
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => EngineError::Timeout,
            _ => EngineError::Connection(err.to_string()),
        })?;
    if read == 0 {
        return Err(EngineError::Connection("sunucu bağlantıyı kapattı".into()));
    }
    let header: Value =
        serde_json::from_str(line.trim()).map_err(|err| EngineError::Protocol(err.to_string()))?;
    let kind = header
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::Protocol("olay türü yok".into()))?
        .to_owned();
    let mut data = header
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let as_len = |key: &str| {
        header
            .get(key)
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(0)
    };
    let data_len = as_len("data_length");
    if data_len > 0 {
        let extra: Value = serde_json::from_slice(&read_exact_len(reader, data_len)?)
            .map_err(|err| EngineError::Protocol(err.to_string()))?;
        if let Value::Object(map) = extra {
            data.extend(map);
        }
    }
    let payload = read_exact_len(reader, as_len("payload_length"))?;
    Ok(Event {
        kind,
        data,
        payload,
    })
}

pub struct WyomingEngine {
    host: String,
    port: u16,
}

impl WyomingEngine {
    pub fn new(host: String, port: u16) -> Result<Self, EngineError> {
        if host.trim().is_empty() {
            return Err(EngineError::NotConfigured(
                "Wyoming sunucu adresi boş".into(),
            ));
        }
        Ok(Self {
            host: host.trim().to_owned(),
            port,
        })
    }

    fn connect(&self) -> Result<TcpStream, EngineError> {
        let addrs = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|err| EngineError::Connection(err.to_string()))?;
        let mut last_error = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(IO_TIMEOUT))
                        .map_err(|e| EngineError::Connection(e.to_string()))?;
                    stream
                        .set_write_timeout(Some(IO_TIMEOUT))
                        .map_err(|e| EngineError::Connection(e.to_string()))?;
                    stream
                        .set_nodelay(true)
                        .map_err(|e| EngineError::Connection(e.to_string()))?;
                    return Ok(stream);
                }
                Err(err) => last_error = Some(err),
            }
        }
        Err(EngineError::Connection(last_error.map_or_else(
            || "adres çözümlenemedi".to_owned(),
            |e| e.to_string(),
        )))
    }

    /// Sends `describe` and returns the first ASR model name the server reports.
    pub fn probe(&self) -> Result<String, EngineError> {
        let stream = self.connect()?;
        let mut writer = stream
            .try_clone()
            .map_err(|e| EngineError::Connection(e.to_string()))?;
        writer
            .write_all(&encode_event("describe", &Value::Null, &[]))
            .map_err(|e| EngineError::Connection(e.to_string()))?;
        let mut reader = BufReader::new(stream);
        let event = read_event(&mut reader)?;
        if event.kind != "info" {
            return Err(EngineError::Protocol(format!(
                "beklenen info, gelen {}",
                event.kind
            )));
        }
        let model = event
            .data
            .get("asr")
            .and_then(Value::as_array)
            .and_then(|programs| programs.first())
            .and_then(|program| program.get("models"))
            .and_then(Value::as_array)
            .and_then(|models| models.first())
            .and_then(|model| model.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("ASR");
        Ok(model.to_owned())
    }
}

impl Transcriber for WyomingEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Wyoming
    }

    fn describe(&self) -> String {
        format!("Wyoming · {}:{}", self.host, self.port)
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let started = Instant::now();
        let stream = self.connect()?;
        let mut writer = stream
            .try_clone()
            .map_err(|e| EngineError::Connection(e.to_string()))?;
        let format = json!({ "rate": TARGET_SAMPLE_RATE, "width": 2, "channels": 1 });
        let transcribe = match &options.language {
            Some(language) => json!({ "language": language }),
            None => json!({}),
        };

        let mut send = |bytes: Vec<u8>| {
            writer
                .write_all(&bytes)
                .map_err(|e| EngineError::Connection(e.to_string()))
        };
        send(encode_event("transcribe", &transcribe, &[]))?;
        send(encode_event("audio-start", &format, &[]))?;
        for chunk in audio.chunks(CHUNK_SAMPLES) {
            send(encode_event("audio-chunk", &format, &pcm16_le(chunk)))?;
        }
        send(encode_event("audio-stop", &json!({}), &[]))?;

        let mut reader = BufReader::new(stream);
        for _ in 0..MAX_EVENTS_BEFORE_TRANSCRIPT {
            if started.elapsed() > TOTAL_TIMEOUT {
                return Err(EngineError::Timeout);
            }
            let event = read_event(&mut reader)?;
            match event.kind.as_str() {
                "transcript" => {
                    let text = event
                        .data
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let language = event
                        .data
                        .get("language")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    return Ok(Transcript {
                        text,
                        language: language.or_else(|| options.language.clone()),
                    });
                }
                "error" => {
                    let message = event
                        .data
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("sunucu hatası");
                    return Err(EngineError::Transcribe(message.to_owned()));
                }
                // transcript-start / transcript-chunk and others: keep reading.
                _ => {}
            }
        }
        Err(EngineError::Protocol("transcript olayı gelmedi".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn encode_then_read_round_trips() {
        let bytes = encode_event("audio-chunk", &json!({"rate": 16000}), &[1, 2, 3]);
        let event = read_event(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(event.kind, "audio-chunk");
        assert_eq!(event.data.get("rate"), Some(&json!(16000)));
        assert_eq!(event.payload, vec![1, 2, 3]);
    }

    #[test]
    fn reads_inline_data_form() {
        let raw = b"{\"type\":\"transcript\",\"data\":{\"text\":\"merhaba\"}}\n".to_vec();
        let event = read_event(&mut Cursor::new(raw)).unwrap();
        assert_eq!(event.data.get("text"), Some(&json!("merhaba")));
    }

    #[test]
    fn rejects_oversized_payload_claims() {
        let raw = format!(
            "{{\"type\":\"x\",\"payload_length\":{}}}\n",
            MAX_FIELD_BYTES + 1
        )
        .into_bytes();
        assert!(matches!(
            read_event(&mut Cursor::new(raw)),
            Err(EngineError::Protocol(_))
        ));
    }

    /// Minimal fake server: reads events until audio-stop, checks the audio
    /// arrived, then answers with a transcript.
    #[test]
    fn transcribes_against_fake_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut kinds = Vec::new();
            let mut audio_bytes = 0;
            loop {
                let event = read_event(&mut reader).unwrap();
                audio_bytes += event.payload.len();
                kinds.push(event.kind.clone());
                if event.kind == "audio-stop" {
                    break;
                }
            }
            writer
                .write_all(&encode_event(
                    "transcript",
                    &json!({"text": "selam dünya"}),
                    &[],
                ))
                .unwrap();
            (kinds, audio_bytes)
        });

        let engine = WyomingEngine::new("127.0.0.1".into(), port).unwrap();
        let options = TranscribeOptions {
            language: Some("tr".into()),
            initial_prompt: None,
        };
        let transcript = engine.transcribe(&vec![0.1; 4_000], &options).unwrap();
        assert_eq!(transcript.text, "selam dünya");
        assert_eq!(transcript.language.as_deref(), Some("tr"));

        let (kinds, audio_bytes) = server.join().unwrap();
        assert_eq!(kinds.first().map(String::as_str), Some("transcribe"));
        assert_eq!(kinds[1], "audio-start");
        assert_eq!(kinds.last().map(String::as_str), Some("audio-stop"));
        assert_eq!(audio_bytes, 8_000);
    }

    #[test]
    fn empty_host_is_not_configured() {
        assert!(matches!(
            WyomingEngine::new("  ".into(), 10300),
            Err(EngineError::NotConfigured(_))
        ));
    }
}
