//! Embedded whisper.cpp engine (Metal-accelerated via whisper-rs).
//!
//! The model is loaded once and kept warm; one decoding state is reused
//! across requests behind a mutex (whisper.cpp states are not re-entrant).

use std::path::Path;
use std::sync::Mutex;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use super::{EngineError, TranscribeOptions, Transcriber, Transcript};
use crate::audio::resample::TARGET_SAMPLE_RATE;
use crate::settings::EngineKind;

/// whisper.cpp refuses clips under 1 s; pad short utterances with silence.
const MIN_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize * 5) / 4;
/// Candidates sampled at each temperature fallback step.
const BEST_OF: i32 = 5;
/// Diminishing returns past 8 threads; the GPU does the heavy lifting.
const MAX_THREADS: usize = 8;

pub struct WhisperCppEngine {
    // Field order matters: the state must drop before its context.
    state: Mutex<WhisperState>,
    _context: WhisperContext,
    model_name: String,
}

impl WhisperCppEngine {
    pub fn load(
        model_path: &Path,
        use_gpu: bool,
        flash_attention: bool,
    ) -> Result<Self, EngineError> {
        if !model_path.is_file() {
            return Err(EngineError::NotConfigured(format!(
                "model dosyası yok: {}",
                model_path.display()
            )));
        }
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu).flash_attn(flash_attention);
        let context = WhisperContext::new_with_params(model_path, params)
            .map_err(|err| EngineError::Load(err.to_string()))?;
        let state = context
            .create_state()
            .map_err(|err| EngineError::Load(err.to_string()))?;
        let model_name = model_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        tracing::info!(model = %model_name, use_gpu, flash_attention, "whisper.cpp model loaded");
        Ok(Self {
            state: Mutex::new(state),
            _context: context,
            model_name,
        })
    }
}

fn thread_count() -> i32 {
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    i32::try_from(available.min(MAX_THREADS)).unwrap_or(4)
}

impl Transcriber for WhisperCppEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::WhisperCpp
    }

    fn describe(&self) -> String {
        format!("whisper.cpp · {}", self.model_name)
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let mut padded;
        let input: &[f32] = if audio.len() < MIN_SAMPLES {
            padded = audio.to_vec();
            padded.resize(MIN_SAMPLES, 0.0);
            &padded
        } else {
            audio
        };

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: BEST_OF });
        params.set_n_threads(thread_count());
        params.set_translate(false);
        params.set_language(options.language.as_deref());
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(prompt) = options.initial_prompt.as_deref() {
            params.set_initial_prompt(prompt);
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| EngineError::Transcribe("whisper durumu kilitlenemedi".into()))?;
        state
            .full(params, input)
            .map_err(|err| EngineError::Transcribe(err.to_string()))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            let piece = segment
                .to_str_lossy()
                .map_err(|err| EngineError::Transcribe(err.to_string()))?;
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
        let language = whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned);
        Ok(Transcript { text, language })
    }
}
