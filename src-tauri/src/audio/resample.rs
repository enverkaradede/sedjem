//! Streaming mono resampler from the device rate to Whisper's 16 kHz.
//!
//! Wraps rubato's synchronous FFT resampler (high quality, fixed ratio) and
//! handles the two things callers otherwise get wrong: the resampler's
//! startup delay (dropped from the front) and the tail still inside the
//! filter when input ends (flushed with silence, then trimmed so the output
//! length is exactly `input_len * out_rate / in_rate`).

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
/// Input frames per FFT chunk: ~21 ms at 48 kHz keeps latency low for
/// future streaming engines while staying efficient.
const CHUNK_FRAMES: usize = 1_024;

#[derive(Debug, thiserror::Error)]
pub enum ResampleError {
    #[error("resampler construction failed: {0}")]
    Construction(String),
    #[error("resampling failed: {0}")]
    Process(String),
}

pub struct MonoResampler {
    inner: Option<Fft<f32>>,
    in_rate: u32,
    pending: Vec<f32>,
    delay_left: usize,
    input_total: u64,
    output_total: u64,
}

impl MonoResampler {
    pub fn new(in_rate: u32) -> Result<Self, ResampleError> {
        let inner = if in_rate == TARGET_SAMPLE_RATE {
            None
        } else {
            let fft = Fft::<f32>::new(
                in_rate as usize,
                TARGET_SAMPLE_RATE as usize,
                CHUNK_FRAMES,
                1,
                FixedSync::Input,
            )
            .map_err(|err| ResampleError::Construction(err.to_string()))?;
            Some(fft)
        };
        let delay_left = inner.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            inner,
            in_rate,
            pending: Vec::with_capacity(CHUNK_FRAMES * 2),
            delay_left,
            input_total: 0,
            output_total: 0,
        })
    }

    /// Number of output samples the whole input so far should map to.
    fn expected_output(&self) -> u64 {
        (self.input_total * u64::from(TARGET_SAMPLE_RATE) + u64::from(self.in_rate) / 2)
            / u64::from(self.in_rate)
    }

    fn emit(&mut self, produced: &[f32], out: &mut Vec<f32>) {
        let skip = self.delay_left.min(produced.len());
        self.delay_left -= skip;
        let useful = &produced[skip..];
        out.extend_from_slice(useful);
        self.output_total += useful.len() as u64;
    }

    fn run_chunk(&mut self, frames: usize, out: &mut Vec<f32>) -> Result<(), ResampleError> {
        let Some(resampler) = self.inner.as_mut() else {
            return Ok(());
        };
        let partial_len = (frames < CHUNK_FRAMES).then_some(frames);
        let mut padded;
        let input: &[f32] = if frames < CHUNK_FRAMES {
            padded = self.pending[..frames].to_vec();
            padded.resize(CHUNK_FRAMES, 0.0);
            &padded
        } else {
            &self.pending[..CHUNK_FRAMES]
        };
        let adapter = InterleavedSlice::new(input, 1, CHUNK_FRAMES)
            .map_err(|err| ResampleError::Process(err.to_string()))?;
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len,
            active_channels_mask: None,
        };
        let produced = resampler
            .process(&adapter, Some(&indexing))
            .map_err(|err| ResampleError::Process(err.to_string()))?
            .take_data();
        self.emit(&produced, out);
        Ok(())
    }

    /// Feeds device-rate mono samples; returns whatever 16 kHz output is ready.
    pub fn push(&mut self, input: &[f32]) -> Result<Vec<f32>, ResampleError> {
        self.input_total += input.len() as u64;
        if self.inner.is_none() {
            self.output_total += input.len() as u64;
            return Ok(input.to_vec());
        }
        self.pending.extend_from_slice(input);
        let mut out = Vec::new();
        while self.pending.len() >= CHUNK_FRAMES {
            self.run_chunk(CHUNK_FRAMES, &mut out)?;
            self.pending.drain(..CHUNK_FRAMES);
        }
        Ok(out)
    }

    /// Flushes the remaining input and filter tail; output is trimmed to the
    /// exact expected length. Call once, after the last `push`.
    pub fn finish(&mut self) -> Result<Vec<f32>, ResampleError> {
        let mut out = Vec::new();
        if self.inner.is_none() {
            return Ok(out);
        }
        let remaining = self.pending.len();
        if remaining > 0 {
            self.run_chunk(remaining, &mut out)?;
            self.pending.clear();
        }
        let expected = self.expected_output();
        // Bounded: each silent chunk yields output, the delay is < 2 chunks.
        let mut guard = 0;
        while self.output_total < expected && guard < 4 {
            self.pending.resize(CHUNK_FRAMES, 0.0);
            self.run_chunk(CHUNK_FRAMES, &mut out)?;
            self.pending.clear();
            guard += 1;
        }
        let excess = self.output_total.saturating_sub(expected) as usize;
        out.truncate(out.len().saturating_sub(excess));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn resample_all(rate: u32, input: &[f32], push_size: usize) -> Vec<f32> {
        let mut r = MonoResampler::new(rate).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(push_size) {
            out.extend(r.push(chunk).unwrap());
        }
        out.extend(r.finish().unwrap());
        out
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn output_length_matches_ratio_for_48k() {
        let input = sine(48_000, 440.0, 1.37);
        let out = resample_all(48_000, &input, 480);
        assert_eq!(out.len(), (input.len() as f64 / 3.0).round() as usize);
    }

    #[test]
    fn output_length_matches_ratio_for_44k1_with_odd_pushes() {
        let input = sine(44_100, 440.0, 0.9);
        let out = resample_all(44_100, &input, 333);
        let expected = (input.len() as f64 * 16_000.0 / 44_100.0).round() as usize;
        assert_eq!(out.len(), expected);
    }

    #[test]
    fn frequency_is_preserved() {
        let input = sine(48_000, 440.0, 1.0);
        let out = resample_all(48_000, &input, 512);
        // 440 Hz over 1 s has ~880 zero crossings; allow edge effects.
        let crossings = zero_crossings(&out) as i64;
        assert!((crossings - 880).abs() <= 6, "crossings = {crossings}");
    }

    #[test]
    fn startup_delay_is_removed() {
        let input = sine(48_000, 440.0, 0.5);
        let out = resample_all(48_000, &input, 4_800);
        // The first 2 ms of output must already carry signal, not filter delay.
        let early_energy: f32 = out[8..32].iter().map(|s| s.abs()).sum();
        assert!(early_energy > 1.0, "early energy = {early_energy}");
    }

    #[test]
    fn passthrough_at_16k() {
        let input = sine(16_000, 440.0, 0.25);
        let out = resample_all(16_000, &input, 100);
        assert_eq!(out, input);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(resample_all(48_000, &[], 10).is_empty());
    }
}
