//! The microphone, behind a trait.
//!
//! The trait exists for the same reason every other one in Magi does: CI has no
//! microphone, and a module whose subject is hardware is exactly the module where
//! the logic around the hardware needs testing.
//!
//! [`AudioSource`] promises **16 kHz mono `f32`** rather than "whatever the device
//! gave us". That is deliberate: it means [`FakeSource`] and the real `cpal`
//! implementation return the same thing, so a fixture recording exercises the same
//! code path a microphone would rather than a parallel one.

use std::sync::{Arc, Mutex};

use crate::audio::format::{Recording, TARGET_RATE};

#[derive(Debug, Clone, thiserror::Error)]
pub enum AudioError {
    /// No input device at all. A Mac always has one, so in practice this means
    /// something is wrong with the audio system rather than with the hardware.
    #[error("no microphone is available")]
    NoDevice,

    /// The OS refused access.
    ///
    /// Its own variant rather than a general failure, because it is the one the user
    /// can fix and the fix is somewhere they would never guess: a System Settings
    /// pane, not anything in Magi. See `.claude/skills/macos-permissions/SKILL.md`.
    #[error(
        "microphone access was denied. Grant it in System Settings › Privacy & \
         Security › Microphone, then try again."
    )]
    PermissionDenied,

    /// The device vanished mid-recording — a USB microphone unplugged, or a headset
    /// disconnected. Whatever was captured before that point is still worth keeping.
    #[error("the microphone was disconnected while recording")]
    Disconnected,

    #[error("the microphone could not be opened: {0}")]
    Unavailable(String),

    #[error("nothing was recorded")]
    Empty,
}

/// Where a recording came from and how it ended.
///
/// Carried alongside the samples because the caller behaves differently: audio that
/// stopped because the cap was reached should still be transcribed, but the user
/// deserves to be told why the recording ended on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// The caller stopped it — the hotkey was released.
    Released,
    /// The length cap was reached. Not an error: see `format::Capacity`.
    CapReached,
    /// The device went away. The samples up to that point are still returned.
    Disconnected,
}

/// One captured utterance.
#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    /// 16 kHz mono `f32`, ready for [`crate::stt`] with no further conversion.
    pub samples: Vec<f32>,
    pub ending: Ending,
}

impl Captured {
    pub fn duration_seconds(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }
}

/// A microphone that can be recorded from.
///
/// Start and stop rather than a stream, because push-to-talk is exactly that shape:
/// the hotkey goes down, the hotkey comes up. Anything finer-grained would be an
/// abstraction built for a wake word that does not exist until M8.
pub trait AudioSource: Send + Sync {
    /// Begins capturing. Idempotent: starting an already-running capture is not an
    /// error, because a hotkey that repeats is a normal thing for an OS to do.
    fn start(&self) -> Result<(), AudioError>;

    /// Stops capturing and returns what was recorded.
    fn stop(&self) -> Result<Captured, AudioError>;

    /// Whether capture is currently running.
    fn is_recording(&self) -> bool;
}

/// A microphone that replays a fixed recording.
///
/// Used by tests, and by the UI before the real implementation exists. It holds a
/// buffer rather than generating tones so a fixture of real speech can be dropped in
/// and the whole pipeline exercised — including the transcriber — with no hardware.
pub struct FakeSource {
    /// What `stop` will return, already at 16 kHz mono.
    samples: Vec<f32>,
    ending: Ending,
    failure: Option<AudioError>,
    recording: Arc<Mutex<Option<Recording>>>,
}

impl FakeSource {
    /// Replays `samples`, treated as 16 kHz mono, ending as though the hotkey was
    /// released.
    pub fn replaying(samples: Vec<f32>) -> Self {
        Self {
            samples,
            ending: Ending::Released,
            failure: None,
            recording: Arc::new(Mutex::new(None)),
        }
    }

    /// Replays a recording that ended for a particular reason, so the caller's
    /// handling of a capped or disconnected capture can be exercised.
    pub fn ending_with(samples: Vec<f32>, ending: Ending) -> Self {
        Self {
            samples,
            ending,
            failure: None,
            recording: Arc::new(Mutex::new(None)),
        }
    }

    /// Fails on `start`, which is how a denied microphone permission reaches the UI.
    pub fn failing(error: AudioError) -> Self {
        Self {
            samples: Vec::new(),
            ending: Ending::Released,
            failure: Some(error),
            recording: Arc::new(Mutex::new(None)),
        }
    }
}

impl AudioSource for FakeSource {
    fn start(&self) -> Result<(), AudioError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Ok(mut slot) = self.recording.lock() {
            *slot = Some(Recording::new());
        }
        Ok(())
    }

    fn stop(&self) -> Result<Captured, AudioError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }

        let mut slot = self
            .recording
            .lock()
            .map_err(|_| AudioError::Unavailable("the fake's lock was poisoned".into()))?;

        // Stopping without starting is a caller bug worth surfacing, not something to
        // paper over with an empty result that reads as "the user said nothing".
        if slot.take().is_none() {
            return Err(AudioError::Empty);
        }

        Ok(Captured {
            samples: self.samples.clone(),
            ending: self.ending,
        })
    }

    fn is_recording(&self) -> bool {
        self.recording
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_second_of_speech() -> Vec<f32> {
        vec![0.1; TARGET_RATE as usize]
    }

    #[test]
    fn the_fake_replays_what_it_was_given() {
        let source = FakeSource::replaying(a_second_of_speech());
        source.start().expect("starting must succeed");
        let captured = source.stop().expect("stopping must succeed");

        assert_eq!(captured.samples.len(), TARGET_RATE as usize);
        assert_eq!(captured.duration_seconds(), 1.0);
        assert_eq!(captured.ending, Ending::Released);
    }

    #[test]
    fn recording_state_tracks_start_and_stop() {
        let source = FakeSource::replaying(a_second_of_speech());
        assert!(!source.is_recording());
        source.start().expect("start");
        assert!(source.is_recording());
        source.stop().expect("stop");
        assert!(!source.is_recording());
    }

    #[test]
    fn starting_twice_is_not_an_error() {
        // An OS can deliver a repeated key-down for a held hotkey, and refusing the
        // second one would turn ordinary key repeat into a visible failure.
        let source = FakeSource::replaying(a_second_of_speech());
        source.start().expect("first start");
        source.start().expect("a repeated start must be accepted");
        assert!(source.is_recording());
    }

    #[test]
    fn stopping_without_starting_is_an_error_rather_than_an_empty_result() {
        // Returning empty samples would reach the transcriber as "the user said
        // nothing", which is a different claim from "nothing was recording".
        let source = FakeSource::replaying(a_second_of_speech());
        assert!(matches!(source.stop(), Err(AudioError::Empty)));
    }

    #[test]
    fn a_denied_permission_surfaces_as_its_own_error() {
        // Distinct from a general failure because it is the one the user can fix,
        // and the fix is in System Settings rather than anywhere in Magi.
        let source = FakeSource::failing(AudioError::PermissionDenied);
        assert!(matches!(source.start(), Err(AudioError::PermissionDenied)));

        let message = AudioError::PermissionDenied.to_string();
        assert!(
            message.contains("System Settings"),
            "the message must say where to go: {message}"
        );
    }

    #[test]
    fn a_capped_recording_reports_why_it_ended() {
        // The caller still transcribes it; the ending is what lets the UI explain
        // that the recording stopped on its own.
        let source = FakeSource::ending_with(a_second_of_speech(), Ending::CapReached);
        source.start().expect("start");
        let captured = source.stop().expect("a capped recording is still returned");

        assert_eq!(captured.ending, Ending::CapReached);
        assert!(!captured.samples.is_empty(), "capped audio must be kept");
    }

    #[test]
    fn a_disconnected_device_still_returns_what_it_captured() {
        // Unplugging a USB microphone mid-sentence should not lose the sentence.
        let source = FakeSource::ending_with(a_second_of_speech(), Ending::Disconnected);
        source.start().expect("start");
        let captured = source.stop().expect("partial audio is still returned");

        assert_eq!(captured.ending, Ending::Disconnected);
        assert_eq!(captured.samples.len(), TARGET_RATE as usize);
    }

    #[test]
    fn every_error_says_something_a_user_could_act_on() {
        for error in [
            AudioError::NoDevice,
            AudioError::PermissionDenied,
            AudioError::Disconnected,
            AudioError::Unavailable("device is in use".into()),
            AudioError::Empty,
        ] {
            let message = error.to_string();
            assert!(!message.is_empty());
            assert!(
                message
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_lowercase() || c.is_uppercase()),
                "{error:?} produced a message that does not read as prose"
            );
        }
    }

    #[test]
    fn the_trait_is_usable_behind_a_reference() {
        // `session` will hold a `Box<dyn AudioSource>`, so the trait has to be
        // object-safe. This is a compile-time assertion written as a test.
        fn record(source: &dyn AudioSource) -> Result<Captured, AudioError> {
            source.start()?;
            source.stop()
        }

        let source = FakeSource::replaying(a_second_of_speech());
        assert!(record(&source).is_ok());
    }
}
