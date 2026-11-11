//! OpenAI-compatible `/audio/transcriptions` client. Works with any server
//! exposing that endpoint (whisper.cpp's server, mlx-audio, speaches, ...).
//! The optional API key comes from the macOS Keychain, never from settings.

use std::time::Duration;

use reqwest::blocking::{multipart, Client};
use serde::Deserialize;

use super::{EngineError, TranscribeOptions, Transcriber, Transcript};
use crate::audio::resample::TARGET_SAMPLE_RATE;
use crate::audio::wav::encode_wav_mono16;
use crate::settings::{validate_base_url, EngineKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Error bodies are truncated before being shown: they may echo request data.
const MAX_ERROR_BODY_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

pub struct OpenAiEngine {
    client: Client,
    endpoint: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

/// Joins the base URL and the endpoint path without doubling slashes.
fn endpoint_for(base_url: &str) -> String {
    format!(
        "{}/audio/transcriptions",
        base_url.trim().trim_end_matches('/')
    )
}

impl OpenAiEngine {
    pub fn new(base_url: &str, model: &str, api_key: Option<String>) -> Result<Self, EngineError> {
        validate_base_url(base_url).map_err(|err| EngineError::NotConfigured(err.message))?;
        if model.trim().is_empty() {
            return Err(EngineError::NotConfigured("model adı boş".into()));
        }
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| EngineError::Load(err.to_string()))?;
        Ok(Self {
            client,
            endpoint: endpoint_for(base_url),
            base_url: base_url.trim().to_owned(),
            model: model.trim().to_owned(),
            api_key: api_key.filter(|k| !k.is_empty()),
        })
    }
}

impl Transcriber for OpenAiEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Openai
    }

    fn describe(&self) -> String {
        format!("HTTP · {} · {}", self.base_url, self.model)
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let wav = encode_wav_mono16(audio, TARGET_SAMPLE_RATE);
        let file = multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|err| EngineError::Transcribe(err.to_string()))?;
        let mut form = multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");
        if let Some(language) = &options.language {
            form = form.text("language", language.clone());
        }
        if let Some(prompt) = &options.initial_prompt {
            form = form.text("prompt", prompt.clone());
        }
        let mut request = self.client.post(&self.endpoint).multipart(form);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().map_err(|err| {
            if err.is_timeout() {
                EngineError::Timeout
            } else {
                EngineError::Connection(err.without_url().to_string())
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            let body: String = response
                .text()
                .unwrap_or_default()
                .chars()
                .take(MAX_ERROR_BODY_CHARS)
                .collect();
            return Err(EngineError::Transcribe(format!("HTTP {status}: {body}")));
        }
        let parsed: TranscriptionResponse = response
            .json()
            .map_err(|err| EngineError::Protocol(err.without_url().to_string()))?;
        Ok(Transcript {
            text: parsed.text,
            language: parsed.language.or_else(|| options.language.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn endpoint_join_handles_trailing_slash() {
        assert_eq!(
            endpoint_for("http://localhost:8000/v1/"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
        assert_eq!(
            endpoint_for("http://localhost:8000/v1"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
    }

    #[test]
    fn rejects_plain_http_to_remote_host() {
        assert!(matches!(
            OpenAiEngine::new("http://api.example.com/v1", "whisper-1", None),
            Err(EngineError::NotConfigured(_))
        ));
    }

    /// Fake HTTP server that captures the request and returns a fixed JSON body.
    fn fake_server(
        status_line: &'static str,
        body: &'static str,
    ) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body_bytes = vec![0u8; content_length];
            reader.read_exact(&mut body_bytes).unwrap();
            let mut writer = stream;
            write!(
                writer,
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            head + &String::from_utf8_lossy(&body_bytes)
        });
        (port, handle)
    }

    #[test]
    fn posts_multipart_and_parses_text() {
        let (port, server) = fake_server("HTTP/1.1 200 OK", r#"{"text":"merhaba dünya"}"#);
        let engine = OpenAiEngine::new(
            &format!("http://127.0.0.1:{port}/v1"),
            "whisper-1",
            Some("k".into()),
        )
        .unwrap();
        let options = TranscribeOptions {
            language: Some("tr".into()),
            initial_prompt: None,
        };
        let transcript = engine.transcribe(&[0.0; 1_600], &options).unwrap();
        assert_eq!(transcript.text, "merhaba dünya");

        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/audio/transcriptions"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer k"));
        assert!(request.contains("name=\"language\""));
        assert!(request.contains("RIFF"));
    }

    #[test]
    fn surfaces_http_errors() {
        let (port, server) = fake_server("HTTP/1.1 500 Internal Server Error", r#"{"error":"x"}"#);
        let engine = OpenAiEngine::new(&format!("http://127.0.0.1:{port}/v1"), "m", None).unwrap();
        let result = engine.transcribe(&[0.0; 160], &TranscribeOptions::default());
        assert!(matches!(result, Err(EngineError::Transcribe(msg)) if msg.contains("500")));
        server.join().unwrap();
    }
}
