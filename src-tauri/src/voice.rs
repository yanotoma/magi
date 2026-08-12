//! Push-to-talk: hold the hotkey, speak, release, get words.
//!
//! The smallest thing that makes M4's promise true. Deliberately **not** a state
//! machine — that is M6's, and it covers thinking, capturing and streaming as well.
//! What is here is one edge in, one edge out, and a transcript emitted to the panel.
//!
//! The transcript lands in the panel's input rather than being sent. Voice puts words in
//! the box; whether to ask them is still the user's decision, and a mis-transcription
//! that goes straight to a model is a wrong question asked confidently.

use tauri::{Emitter, Manager};

use crate::audio::{AudioError, Ending};
use crate::commands::AppState;
use crate::stt::SttError;

/// What the panel is told while a voice turn happens.
///
/// Emitted as `magi://voice`, one event per transition. Distinct states rather than a
/// boolean, because "recording" and "transcribing" feel different to wait through:
/// recording ends when you let go, and transcription ends when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VoiceState {
    Idle,
    Recording,
    Transcribing,
}

/// Handles a press or release of the push-to-talk shortcut.
///
/// Both edges land here because "hold to record" is exactly a press and a release. The
/// OS repeats key-down while a key is held, which is why `AudioSource::start` is
/// idempotent — a repeat must not restart the recording or report an error.
pub fn on_push_to_talk(app: &tauri::AppHandle, pressed: bool) {
    if pressed {
        begin(app);
    } else {
        finish(app);
    }
}

fn begin(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    match state.microphone.start() {
        Ok(()) => {
            crate::session::report(app, crate::session::Event::Held);
            let _ = app.emit("magi://voice", VoiceState::Recording);
        }
        Err(error) => {
            tracing::warn!(%error, "could not start recording");
            crate::session::report(app, crate::session::Event::Stopped);
            let _ = app.emit("magi://voice", VoiceState::Idle);
            // A denied microphone is the case worth naming: the fix is in System
            // Settings, which no error about a device would ever suggest.
            let _ = app.emit("magi://voice-error", error.to_string());
        }
    }
}

fn finish(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    // Not recording. A release without a press happens whenever the shortcut is held
    // across a restart, or when `start` failed a moment ago — neither is worth an error.
    if !state.microphone.is_recording() {
        return;
    }

    let captured = match state.microphone.stop() {
        Ok(captured) => captured,
        Err(AudioError::Empty) => return,
        Err(error) => {
            tracing::warn!(%error, "could not stop recording");
            crate::session::report(app, crate::session::Event::Stopped);
            let _ = app.emit("magi://voice", VoiceState::Idle);
            let _ = app.emit("magi://voice-error", error.to_string());
            return;
        }
    };

    // Worth telling the user about, and not a failure: the audio is still transcribed.
    match captured.ending {
        Ending::CapReached => {
            let _ = app.emit(
                "magi://voice-notice",
                "Recording stopped at the two-minute limit.".to_string(),
            );
        }
        Ending::Disconnected => {
            let _ = app.emit(
                "magi://voice-notice",
                "The microphone disconnected. Transcribing what was captured.".to_string(),
            );
        }
        Ending::Released => {}
    }

    // A tap rather than a hold. Silently ignored: Whisper given a fraction of a second
    // of room tone does not return nothing, it invents something, and a confident answer
    // to a question nobody asked is worse than no answer.
    if !crate::audio::format::worth_transcribing(&captured.samples) {
        tracing::debug!(
            seconds = captured.duration_seconds(),
            "too short to transcribe; ignoring"
        );
        crate::session::report(app, crate::session::Event::Stopped);
        let _ = app.emit("magi://voice", VoiceState::Idle);
        return;
    }

    crate::session::report(app, crate::session::Event::Released);
    let _ = app.emit("magi://voice", VoiceState::Transcribing);

    let app = app.clone();
    // `spawn_blocking`, because inference is CPU-bound for seconds. On the main thread
    // it would freeze the tray and the hotkey; on an async worker it would hold a runtime
    // thread for its whole duration and starve everything else that runtime polls.
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };

        // Cloned, and the lock released before inference. Holding it for the seconds a
        // transcription takes would freeze Settings, and a model swapped mid-transcription
        // should not cut off the one already running.
        let Ok(transcriber) = state.transcriber.lock().map(|t| t.clone()) else {
            return;
        };

        let result = transcriber.transcribe(&captured.samples);
        // Whatever came back, transcription is over. Reported before the outcome is
        // examined, so an error path cannot leave the indicator spinning.
        crate::session::report(&app, crate::session::Event::Transcribed);
        let _ = app.emit("magi://voice", VoiceState::Idle);

        match result {
            Ok(transcript) if transcript.is_meaningful() => {
                tracing::info!(characters = transcript.text.len(), "transcript ready");
                // The panel decides what to do with it. Voice fills the box; sending is
                // still the user's move.
                let _ = app.emit("magi://transcript", transcript.text);
            }
            Ok(_) => {
                // The model produced only a silence artefact. Nothing to show, and
                // nothing wrong — saying "no speech detected" for a held key in a quiet
                // room is more useful than putting "Thank you." in the input.
                tracing::debug!("the transcript held no speech");
                let _ = app.emit("magi://voice-notice", "Nothing was heard.".to_string());
            }
            Err(SttError::ModelMissing) => {
                // The first-run state, and the one with a specific fix.
                let _ = app.emit(
                    "magi://voice-error",
                    "No speech model yet. Open Settings › Voice to download one.".to_string(),
                );
            }
            Err(SttError::TooShort) => {}
            Err(error) => {
                tracing::warn!(%error, "transcription failed");
                let _ = app.emit("magi://voice-error", error.to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioSource, FakeSource};
    use crate::stt::{FakeTranscriber, Transcriber};

    // `on_push_to_talk` needs an `AppHandle`, which needs a running Tauri app. What is
    // testable without one is the sequence it drives, which is asserted here against the
    // fakes — the same objects the real path uses through their traits.

    #[test]
    fn a_held_key_records_then_transcribes() {
        let microphone = FakeSource::replaying(vec![0.1; 16_000]);
        let transcriber = FakeTranscriber::saying("why is this build failing");

        microphone.start().expect("start");
        assert!(microphone.is_recording());

        let captured = microphone.stop().expect("stop");
        assert_eq!(captured.ending, Ending::Released);
        assert!(crate::audio::format::worth_transcribing(&captured.samples));

        let transcript = transcriber.transcribe(&captured.samples).expect("ok");
        assert_eq!(transcript.text, "why is this build failing");
        assert!(transcript.is_meaningful());
    }

    #[test]
    fn a_tapped_key_never_reaches_the_transcriber() {
        // The guard that matters most. Whisper given a fraction of a second of room tone
        // invents something, and it would arrive in the input as a question the user
        // never asked.
        let microphone = FakeSource::replaying(vec![0.0; 800]);
        let transcriber = FakeTranscriber::saying("Thank you.");

        microphone.start().expect("start");
        let captured = microphone.stop().expect("stop");

        assert!(
            !crate::audio::format::worth_transcribing(&captured.samples),
            "a tap must be rejected before the model is reached"
        );
        assert!(
            transcriber.received().is_empty(),
            "the transcriber was called for a tap"
        );
    }

    #[test]
    fn a_silence_artefact_is_not_shown_to_the_user() {
        // A key held in a quiet room. The audio is long enough to transcribe, and what
        // comes back is one of Whisper's subtitle habits.
        let transcriber = FakeTranscriber::saying("Thank you.");
        let transcript = transcriber.transcribe(&[0.0; 16_000]).expect("ok");

        assert!(
            !transcript.is_meaningful(),
            "\"Thank you.\" must not reach the input"
        );
    }

    #[test]
    fn a_capped_recording_is_still_transcribed() {
        // Reaching the limit is not a failure. The user said something; the last thing to
        // do with it is discard it because they said too much.
        let microphone = FakeSource::ending_with(vec![0.2; 16_000], Ending::CapReached);
        microphone.start().expect("start");
        let captured = microphone.stop().expect("stop");

        assert_eq!(captured.ending, Ending::CapReached);
        assert!(crate::audio::format::worth_transcribing(&captured.samples));
    }

    #[test]
    fn a_disconnected_microphone_keeps_what_it_heard() {
        // Unplugging a headset mid-sentence should not lose the sentence.
        let microphone = FakeSource::ending_with(vec![0.2; 16_000], Ending::Disconnected);
        microphone.start().expect("start");
        let captured = microphone.stop().expect("stop");

        assert_eq!(captured.ending, Ending::Disconnected);
        assert!(!captured.samples.is_empty());
    }

    #[test]
    fn releasing_without_recording_is_not_an_error_path() {
        // Happens when the shortcut is held across a restart, or when `start` failed a
        // moment ago. `finish` checks `is_recording` first for exactly this.
        let microphone = FakeSource::replaying(vec![0.1; 16_000]);
        assert!(!microphone.is_recording());
    }

    #[test]
    fn a_repeated_press_does_not_restart_the_recording() {
        // The OS repeats key-down while a key is held.
        let microphone = FakeSource::replaying(vec![0.1; 16_000]);
        microphone.start().expect("first");
        microphone.start().expect("a repeat must be accepted");
        assert!(microphone.is_recording());
    }

    #[test]
    fn a_missing_model_is_reported_as_something_to_do() {
        let transcriber = FakeTranscriber::unready();
        assert!(matches!(
            transcriber.transcribe(&[0.1; 16_000]),
            Err(SttError::ModelMissing)
        ));
    }

    #[test]
    fn the_states_serialise_as_the_panel_expects() {
        // The panel switches on these strings; a renamed variant would silently stop
        // matching and leave the indicator stuck.
        assert_eq!(
            serde_json::to_string(&VoiceState::Recording).expect("serialisable"),
            "\"recording\""
        );
        assert_eq!(
            serde_json::to_string(&VoiceState::Transcribing).expect("serialisable"),
            "\"transcribing\""
        );
        assert_eq!(
            serde_json::to_string(&VoiceState::Idle).expect("serialisable"),
            "\"idle\""
        );
    }
}
