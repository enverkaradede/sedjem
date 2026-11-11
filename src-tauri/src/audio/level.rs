//! Loudness tracking: UI level meter, speech gate and silence auto-stop.
//!
//! Works on 16 kHz mono frames of fixed length so durations are exact and
//! independent of how the device chunks its callbacks.

use std::time::Duration;

use super::resample::TARGET_SAMPLE_RATE;

/// 20 ms analysis frames.
pub const FRAME_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) / 50;
const FRAME_DURATION: Duration = Duration::from_millis(20);
/// RMS above this counts as speech (~ -40 dBFS). Deliberately low: the gate
/// only has to reject taps, coughs and dead air, not judge content.
const SPEECH_RMS_THRESHOLD: f32 = 0.01;
/// Below this the input is effectively digital silence (denied mic permission
/// on macOS delivers exact zeros rather than an error).
const DEAD_AIR_RMS: f32 = 1e-6;
const METER_FLOOR_DB: f32 = -60.0;

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Maps RMS to a 0..1 meter value on a dB scale (perceptually even).
pub fn meter_value(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// Summary of a finished recording used to decide whether to transcribe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechStats {
    pub total: Duration,
    pub speech: Duration,
    pub peak_rms: f32,
}

impl SpeechStats {
    /// True when the input was exact silence for the whole recording.
    pub fn is_dead_air(&self) -> bool {
        self.peak_rms < DEAD_AIR_RMS
    }
}

#[derive(Debug, Clone)]
pub struct LevelTracker {
    frame: Vec<f32>,
    total: Duration,
    speech: Duration,
    trailing_silence: Duration,
    heard_speech: bool,
    peak_rms: f32,
    last_rms: f32,
}

impl Default for LevelTracker {
    fn default() -> Self {
        Self {
            frame: Vec::with_capacity(FRAME_SAMPLES),
            total: Duration::ZERO,
            speech: Duration::ZERO,
            trailing_silence: Duration::ZERO,
            heard_speech: false,
            peak_rms: 0.0,
            last_rms: 0.0,
        }
    }
}

impl LevelTracker {
    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.frame.push(sample);
            if self.frame.len() == FRAME_SAMPLES {
                self.close_frame();
            }
        }
    }

    fn close_frame(&mut self) {
        let level = rms(&self.frame);
        self.frame.clear();
        self.last_rms = level;
        self.peak_rms = self.peak_rms.max(level);
        self.total += FRAME_DURATION;
        if level >= SPEECH_RMS_THRESHOLD {
            self.speech += FRAME_DURATION;
            self.heard_speech = true;
            self.trailing_silence = Duration::ZERO;
        } else {
            self.trailing_silence += FRAME_DURATION;
        }
    }

    pub fn last_rms(&self) -> f32 {
        self.last_rms
    }

    /// Silence after speech has been heard; zero before the first word so the
    /// user can take a breath before starting without being cut off.
    pub fn silence_after_speech(&self) -> Duration {
        if self.heard_speech {
            self.trailing_silence
        } else {
            Duration::ZERO
        }
    }

    pub fn stats(&self) -> SpeechStats {
        SpeechStats {
            total: self.total,
            speech: self.speech,
            peak_rms: self.peak_rms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(value: f32, count: usize) -> Vec<f32> {
        vec![value; FRAME_SAMPLES * count]
    }

    #[test]
    fn meter_maps_db_range_to_unit_interval() {
        assert_eq!(meter_value(0.0), 0.0);
        assert!((meter_value(1.0) - 1.0).abs() < 1e-6);
        assert!((meter_value(0.001) - 0.0).abs() < 1e-6); // -60 dB
        assert!((meter_value(0.0316) - 0.5).abs() < 0.01); // ~-30 dB
    }

    #[test]
    fn counts_speech_and_trailing_silence() {
        let mut t = LevelTracker::default();
        t.push(&frames(0.0, 10)); // 200 ms leading silence
        assert_eq!(t.silence_after_speech(), Duration::ZERO);
        t.push(&frames(0.2, 25)); // 500 ms speech
        t.push(&frames(0.0, 50)); // 1 s silence
        assert_eq!(t.stats().speech, Duration::from_millis(500));
        assert_eq!(t.silence_after_speech(), Duration::from_secs(1));
        assert_eq!(t.stats().total, Duration::from_millis(1_700));
    }

    #[test]
    fn partial_frames_accumulate_across_pushes() {
        let mut t = LevelTracker::default();
        let half = vec![0.2; FRAME_SAMPLES / 2];
        t.push(&half);
        assert_eq!(t.stats().total, Duration::ZERO);
        t.push(&half);
        assert_eq!(t.stats().speech, FRAME_DURATION);
    }

    #[test]
    fn exact_zero_input_is_dead_air() {
        let mut t = LevelTracker::default();
        t.push(&frames(0.0, 50));
        assert!(t.stats().is_dead_air());
        t.push(&frames(0.001, 1));
        assert!(!t.stats().is_dead_air());
    }
}
