//! Tap-vs-hold press controller.
//!
//! One state machine serves both the floating button and the global
//! shortcut. A press always starts recording immediately (no added latency);
//! the *release* decides the mode:
//!
//! * released before `tap_threshold` → **toggle**: keep recording until the
//!   next press (or silence auto-stop / max duration).
//! * released after `tap_threshold` → **push-to-talk**: the release ends it.
//!
//! The controller is pure: time is passed in, side effects are returned as
//! [`Command`]s for the caller to execute. This keeps it trivially testable.

use std::time::{Duration, Instant};

use serde::Serialize;

/// Where an input came from. A release only counts if it comes from the same
/// source that pressed, so mixing mouse and keyboard cannot cross-trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputSource {
    Panel,
    Hotkey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStopReason {
    /// Only honoured in toggle mode: while the user is holding, they decide.
    Silence,
    /// Honoured in every recording mode.
    MaxDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Press { source: InputSource, at: Instant },
    Release { source: InputSource, at: Instant },
    AutoStop(AutoStopReason),
    Cancel,
    ProcessingFinished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Latch {
    /// Still pressed; mode not decided yet.
    Pending,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording {
        source: InputSource,
        latch: Latch,
        pressed_at: Instant,
    },
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    StartRecording,
    StopAndTranscribe,
    DiscardRecording,
    /// Latch changed while recording; the UI should reflect it.
    LatchChanged(Latch),
    /// Input arrived while busy; the UI may give a "busy" hint.
    Busy,
    None,
}

#[derive(Debug, Clone)]
pub struct Controller {
    phase: Phase,
    tap_threshold: Duration,
}

impl Controller {
    pub fn new(tap_threshold: Duration) -> Self {
        Self {
            phase: Phase::Idle,
            tap_threshold,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn set_tap_threshold(&mut self, tap_threshold: Duration) {
        self.tap_threshold = tap_threshold;
    }

    /// Applies one input and returns the side effect the caller must perform.
    pub fn handle(&mut self, input: Input) -> Command {
        match (self.phase, input) {
            (Phase::Idle, Input::Press { source, at }) => {
                self.phase = Phase::Recording {
                    source,
                    latch: Latch::Pending,
                    pressed_at: at,
                };
                Command::StartRecording
            }
            (
                Phase::Recording {
                    source,
                    latch: Latch::Pending,
                    pressed_at,
                },
                Input::Release { source: from, at },
            ) if from == source => {
                if at.saturating_duration_since(pressed_at) < self.tap_threshold {
                    self.phase = Phase::Recording {
                        source,
                        latch: Latch::Toggle,
                        pressed_at,
                    };
                    Command::LatchChanged(Latch::Toggle)
                } else {
                    self.phase = Phase::Processing;
                    Command::StopAndTranscribe
                }
            }
            // Any press while toggled ends the recording, whatever the source.
            (
                Phase::Recording {
                    latch: Latch::Toggle,
                    ..
                },
                Input::Press { .. },
            ) => {
                self.phase = Phase::Processing;
                Command::StopAndTranscribe
            }
            (
                Phase::Recording {
                    latch: Latch::Toggle,
                    ..
                },
                Input::AutoStop(AutoStopReason::Silence),
            )
            | (Phase::Recording { .. }, Input::AutoStop(AutoStopReason::MaxDuration)) => {
                self.phase = Phase::Processing;
                Command::StopAndTranscribe
            }
            (Phase::Recording { .. }, Input::Cancel) => {
                self.phase = Phase::Idle;
                Command::DiscardRecording
            }
            (Phase::Processing, Input::ProcessingFinished) => {
                self.phase = Phase::Idle;
                Command::None
            }
            (Phase::Processing, Input::Press { .. }) => Command::Busy,
            // Key auto-repeat presses, stray releases, releases from the other
            // source and silence while holding are all deliberately ignored.
            _ => Command::None,
        }
    }

    /// Forces the controller back to idle, e.g. when recording failed to start.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: Duration = Duration::from_millis(300);

    fn press(source: InputSource, at: Instant) -> Input {
        Input::Press { source, at }
    }
    fn release(source: InputSource, at: Instant) -> Input {
        Input::Release { source, at }
    }

    #[test]
    fn press_starts_recording_immediately() {
        let mut c = Controller::new(THRESHOLD);
        assert_eq!(
            c.handle(press(InputSource::Panel, Instant::now())),
            Command::StartRecording
        );
    }

    #[test]
    fn quick_release_latches_toggle_and_next_press_stops() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Hotkey, t0));
        assert_eq!(
            c.handle(release(
                InputSource::Hotkey,
                t0 + Duration::from_millis(120)
            )),
            Command::LatchChanged(Latch::Toggle)
        );
        assert_eq!(
            c.handle(press(InputSource::Hotkey, t0 + Duration::from_secs(4))),
            Command::StopAndTranscribe
        );
        // The release that follows the stopping press must not do anything.
        assert_eq!(
            c.handle(release(InputSource::Hotkey, t0 + Duration::from_secs(5))),
            Command::None
        );
    }

    #[test]
    fn long_hold_is_push_to_talk() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, t0));
        assert_eq!(
            c.handle(release(InputSource::Panel, t0 + Duration::from_secs(2))),
            Command::StopAndTranscribe
        );
        assert_eq!(c.phase(), Phase::Processing);
    }

    #[test]
    fn release_exactly_at_threshold_counts_as_hold() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, t0));
        assert_eq!(
            c.handle(release(InputSource::Panel, t0 + THRESHOLD)),
            Command::StopAndTranscribe
        );
    }

    #[test]
    fn key_repeat_presses_while_holding_are_ignored() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Hotkey, t0));
        assert_eq!(
            c.handle(press(InputSource::Hotkey, t0 + Duration::from_millis(500))),
            Command::None
        );
        assert!(matches!(
            c.phase(),
            Phase::Recording {
                latch: Latch::Pending,
                ..
            }
        ));
    }

    #[test]
    fn release_from_other_source_is_ignored() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, t0));
        assert_eq!(
            c.handle(release(InputSource::Hotkey, t0 + Duration::from_secs(1))),
            Command::None
        );
        assert_eq!(
            c.handle(release(InputSource::Panel, t0 + Duration::from_secs(1))),
            Command::StopAndTranscribe
        );
    }

    #[test]
    fn silence_stops_only_in_toggle_mode() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, t0));
        assert_eq!(
            c.handle(Input::AutoStop(AutoStopReason::Silence)),
            Command::None
        );
        c.handle(release(InputSource::Panel, t0 + Duration::from_millis(100)));
        assert_eq!(
            c.handle(Input::AutoStop(AutoStopReason::Silence)),
            Command::StopAndTranscribe
        );
    }

    #[test]
    fn max_duration_stops_even_while_holding() {
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Hotkey, Instant::now()));
        assert_eq!(
            c.handle(Input::AutoStop(AutoStopReason::MaxDuration)),
            Command::StopAndTranscribe
        );
    }

    #[test]
    fn press_while_processing_reports_busy_and_finishing_returns_to_idle() {
        let t0 = Instant::now();
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, t0));
        c.handle(release(InputSource::Panel, t0 + Duration::from_secs(1)));
        assert_eq!(
            c.handle(press(InputSource::Panel, t0 + Duration::from_secs(2))),
            Command::Busy
        );
        c.handle(Input::ProcessingFinished);
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn cancel_discards_recording() {
        let mut c = Controller::new(THRESHOLD);
        c.handle(press(InputSource::Panel, Instant::now()));
        assert_eq!(c.handle(Input::Cancel), Command::DiscardRecording);
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn stray_release_in_idle_is_ignored() {
        let mut c = Controller::new(THRESHOLD);
        assert_eq!(
            c.handle(release(InputSource::Panel, Instant::now())),
            Command::None
        );
        assert_eq!(c.phase(), Phase::Idle);
    }
}
