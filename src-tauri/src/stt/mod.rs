//! Speech to text. PCM in, words out.
//!
//! A leaf module, like `audio`: it knows nothing about the rest of Magi, and the
//! trait exists so that CI can exercise everything around transcription without
//! compiling whisper.cpp or owning a GPU.
//!
//! The input contract is inherited rather than restated. [`Transcriber`] takes 16 kHz
//! mono `f32` because that is what [`crate::audio::AudioSource`] produces and what
//! Whisper requires — `whisper-rs` does not resample, so there is exactly one place
//! that conversion can happen and it is `audio::format`.

pub mod fake;
pub mod model;
pub mod whisper;

pub use fake::FakeTranscriber;
pub use model::Model;
pub use whisper::WhisperTranscriber;

use crate::audio::format::TARGET_RATE;

#[derive(Debug, Clone, thiserror::Error)]
pub enum SttError {
    /// The model file is not on disk yet.
    ///
    /// Its own variant because it is the expected state on first run, not a failure:
    /// the UI's response is to offer the download, not to report an error.
    #[error("the speech model has not been downloaded yet")]
    ModelMissing,

    #[error("the speech model at {path} could not be loaded: {reason}")]
    ModelUnreadable { path: String, reason: String },

    #[error("transcription failed: {0}")]
    Failed(String),

    /// The audio was too short to be worth running the model over. See
    /// `audio::format::worth_transcribing` for why this is checked rather than left
    /// to the model.
    #[error("the recording was too short to transcribe")]
    TooShort,
}

/// What the model heard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transcript {
    /// The full text, with the segments joined and trimmed.
    pub text: String,
}

impl Transcript {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into().trim().to_string(),
        }
    }

    /// Whether the model produced anything usable.
    ///
    /// Whisper returns non-empty output for near-silence — it hallucinates rather
    /// than reporting nothing — so this is a last guard after
    /// `audio::format::worth_transcribing` has already filtered by length. Both are
    /// needed: the length check catches a tapped hotkey, and this catches a held one
    /// in a silent room.
    pub fn is_meaningful(&self) -> bool {
        !self.text.is_empty() && !is_hallucinated_silence(&self.text)
    }
}

/// Whether the text is one of Whisper's known outputs for silence.
///
/// These are not guesses. Whisper's training data includes a great deal of subtitled
/// video, so given silence it emits the things that appear over silent footage:
/// applause markers, music markers, and the sign-off from the end of a video. Sending
/// one of those to a model as a question produces a confident answer to something
/// the user never said.
///
/// Matched on the whole trimmed string rather than as a substring, so a genuine
/// question that happens to contain "thank you" is not thrown away.
fn is_hallucinated_silence(text: &str) -> bool {
    const KNOWN: [&str; 10] = [
        "thank you.",
        "thanks for watching!",
        "thank you for watching.",
        "thank you for watching!",
        "[blank_audio]",
        "(silence)",
        "[silence]",
        "[music]",
        "[applause]",
        "you",
    ];

    let normalised = text.trim().to_lowercase();
    KNOWN.contains(&normalised.as_str())
}

/// Turns recorded speech into text.
///
/// Synchronous, and called from `spawn_blocking`. Inference is CPU-bound and takes
/// seconds; making this `async` would suggest it yields, which it does not — it would
/// occupy a runtime thread for its whole duration and stall everything else the
/// runtime was going to poll. Keeping it blocking makes the caller's obligation
/// obvious.
pub trait Transcriber: Send + Sync {
    /// `samples` must be 16 kHz mono `f32`.
    fn transcribe(&self, samples: &[f32]) -> Result<Transcript, SttError>;

    /// Whether the model is loaded and ready.
    ///
    /// Separate from transcribing so Settings can report readiness without running
    /// inference over a dummy buffer.
    fn is_ready(&self) -> bool;
}

/// The sample rate every implementation of [`Transcriber`] expects.
///
/// Re-exported so a caller cannot get the contract from the wrong place, or hardcode
/// 16000 next to a constant that already says it.
pub const REQUIRED_RATE: u32 = TARGET_RATE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_is_trimmed_on_construction() {
        // Whisper pads its segments with leading spaces, and joining them without
        // trimming puts the panel's first character one space in.
        assert_eq!(Transcript::new("  hello there \n").text, "hello there");
    }

    #[test]
    fn real_speech_is_meaningful() {
        assert!(Transcript::new("what is on my screen").is_meaningful());
        assert!(Transcript::new("Thank you for the explanation, that helps.").is_meaningful());
    }

    #[test]
    fn empty_output_is_not_meaningful() {
        assert!(!Transcript::new("").is_meaningful());
        assert!(!Transcript::new("   \n ").is_meaningful());
    }

    #[test]
    fn whispers_silence_hallucinations_are_rejected() {
        // Not guesses. Whisper's training data is full of subtitled video, so given
        // silence it emits what appears over silent footage. Passing one of these on
        // as a question gets the user a confident answer to something they never
        // said.
        for hallucination in [
            "Thank you.",
            "thank you.",
            " Thanks for watching! ",
            "[BLANK_AUDIO]",
            "[Music]",
            "[Applause]",
            "You",
        ] {
            assert!(
                !Transcript::new(hallucination).is_meaningful(),
                "{hallucination:?} is a known silence artefact and must be rejected"
            );
        }
    }

    #[test]
    fn a_real_question_containing_thank_you_survives() {
        // The reason the match is on the whole string rather than a substring.
        // Throwing away a real question because it contains a polite phrase would be
        // a worse failure than passing one artefact through.
        assert!(Transcript::new("Thank you. Now, why is this build failing?").is_meaningful());
        assert!(Transcript::new("How do I thank you in Japanese?").is_meaningful());
    }

    #[test]
    fn the_required_rate_matches_the_audio_module() {
        // Two constants naming the same fact would drift. This asserts they are the
        // same fact.
        assert_eq!(REQUIRED_RATE, crate::audio::format::TARGET_RATE);
    }

    #[test]
    fn a_missing_model_is_its_own_error() {
        // The expected state on first run. The UI's response is to offer the
        // download, which it cannot do if this arrives as a generic failure.
        let message = SttError::ModelMissing.to_string();
        assert!(message.contains("downloaded"), "got: {message}");
    }
}
