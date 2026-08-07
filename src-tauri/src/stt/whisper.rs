//! Transcription via `whisper-rs`, which is whisper.cpp behind Rust bindings.
//!
//! The only part of `stt` that needs a model file and a C++ toolchain. Everything
//! that decides whether a transcript is usable is in [`super`], which compiles and
//! tests with neither.
//!
//! Written against the 0.16 source rather than from examples: the segment API changed
//! from `full_get_segment_text(i)` to an iterator of [`whisper_rs::WhisperSegment`],
//! and the older shape appears in most documentation.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{SttError, Transcriber, Transcript};

/// Below this, whisper.cpp's own judgement that a segment is not speech is trusted.
///
/// The model reports a no-speech probability per segment, which is a far better
/// signal than matching its output against a list of known hallucinations: it is
/// numeric, it comes from the model, and it does not depend on the language. The
/// string list in [`super`] stays as a backstop for the cases that slip through with
/// a confident-looking probability.
///
/// 0.6 is whisper.cpp's own default for `no_speech_thold`.
const NO_SPEECH_THRESHOLD: f32 = 0.6;

/// How many threads inference may use.
///
/// Deliberately not every core. Magi transcribes while the user is waiting but also
/// while they are working, and saturating the machine to shave a second off a
/// transcription is the wrong trade for a background app. Half the cores, at least
/// two.
fn thread_count() -> i32 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    (cores / 2).max(2) as i32
}

pub struct WhisperTranscriber {
    /// `None` until a model has been loaded.
    ///
    /// Loading is deferred rather than done in `new` so that a missing model is a
    /// reportable state instead of a constructor failure — on first run there is no
    /// model, and the app still has to start and offer the download.
    context: Mutex<Option<WhisperContext>>,
    model_path: PathBuf,
}

impl WhisperTranscriber {
    /// Creates a transcriber for a model that may or may not exist yet.
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            context: Mutex::new(None),
            model_path: model_path.into(),
        }
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Loads the model, if it is not already loaded.
    ///
    /// Takes seconds and hundreds of megabytes of resident memory for the larger
    /// models, so it happens once and on first use rather than at startup — a tray
    /// app that spends four seconds loading a speech model before it can show a
    /// window is a tray app that feels broken.
    pub fn load(&self) -> Result<(), SttError> {
        let mut slot = self
            .context
            .lock()
            .map_err(|_| SttError::Failed("the model lock was poisoned".into()))?;

        if slot.is_some() {
            return Ok(());
        }

        if !self.model_path.exists() {
            return Err(SttError::ModelMissing);
        }

        let path = self.model_path.to_string_lossy().to_string();
        let context = WhisperContext::new_with_params(&path, WhisperContextParameters::default())
            .map_err(|e| SttError::ModelUnreadable {
            path: path.clone(),
            reason: e.to_string(),
        })?;

        tracing::info!(path = %path, threads = thread_count(), "speech model loaded");
        *slot = Some(context);
        Ok(())
    }

    /// Frees the model.
    ///
    /// Worth having because the model is the largest thing Magi holds — 141 MB
    /// resident for `base.en`, over a gigabyte for `medium.en` — and a background app
    /// that never gives it back is a background app people quit.
    pub fn unload(&self) {
        if let Ok(mut slot) = self.context.lock() {
            if slot.take().is_some() {
                tracing::info!("speech model unloaded");
            }
        }
    }
}

/// The parameters every transcription uses.
fn params<'a>() -> FullParams<'a, 'a> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // whisper.cpp writes to stdout unless every one of these is off. In a background
    // tray app that is noise going nowhere, and it interleaves with `tracing` output
    // for anyone running with RUST_LOG set.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    params.set_n_threads(thread_count());

    // English, and stated rather than detected. The default models are `.en`
    // variants, which cannot detect a language they were not trained on, and asking
    // them to try costs a pass over the audio to reach the same answer.
    params.set_language(Some("en"));
    params.set_translate(false);

    // Suppresses the blank and non-speech tokens whisper.cpp emits over silence,
    // which is the same problem `Transcript::is_meaningful` guards against — better
    // to stop it at the source as well.
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);

    params.set_no_speech_thold(NO_SPEECH_THRESHOLD);

    params
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<Transcript, SttError> {
        // Checked here as well as by the caller, because this is the boundary the
        // model sits behind: given a fraction of a second of room tone Whisper does
        // not return nothing, it invents something.
        if !crate::audio::format::worth_transcribing(samples) {
            return Err(SttError::TooShort);
        }

        self.load()?;

        let slot = self
            .context
            .lock()
            .map_err(|_| SttError::Failed("the model lock was poisoned".into()))?;
        let context = slot.as_ref().ok_or(SttError::ModelMissing)?;

        let mut state = context
            .create_state()
            .map_err(|e| SttError::Failed(format!("the model would not start: {e}")))?;

        state
            .full(params(), samples)
            .map_err(|e| SttError::Failed(e.to_string()))?;

        // `as_iter` and `to_str_lossy`, not the `full_get_segment_text(i)` that most
        // examples show — the API changed. Lossy because the strict version fails the
        // whole transcription on one invalid byte, and losing a sentence to a
        // replacement character is the wrong trade.
        let mut text = String::new();
        let mut spoken_segments = 0usize;

        for segment in state.as_iter() {
            // The model's own judgement, per segment, that this is not speech. A
            // better signal than matching the output against known hallucinations:
            // numeric, from the model, and language-independent.
            if segment.no_speech_probability() > NO_SPEECH_THRESHOLD {
                tracing::debug!(
                    probability = segment.no_speech_probability(),
                    "dropping a segment the model considers silence"
                );
                continue;
            }

            let Ok(chunk) = segment.to_str_lossy() else {
                continue;
            };

            let chunk = chunk.trim();
            if chunk.is_empty() {
                continue;
            }

            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(chunk);
            spoken_segments += 1;
        }

        tracing::info!(
            seconds = samples.len() as f32 / super::REQUIRED_RATE as f32,
            segments = spoken_segments,
            "transcribed"
        );

        Ok(Transcript::new(text))
    }

    fn is_ready(&self) -> bool {
        // Loaded, or loadable: the model file existing is what the UI needs to know,
        // and forcing a load to answer would make a readiness check cost seconds.
        self.context
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
            || self.model_path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here loads a model. CI has neither the 141 MB file nor a reason to run
    // inference; what is testable is the behaviour around it.

    #[test]
    fn a_missing_model_reports_itself_rather_than_failing_to_construct() {
        // The first-run state. If `new` failed, the app could not start to offer the
        // download.
        let transcriber = WhisperTranscriber::new("/nonexistent/ggml-base.en.bin");
        assert!(!transcriber.is_ready());
        assert!(matches!(transcriber.load(), Err(SttError::ModelMissing)));
    }

    #[test]
    fn audio_too_short_is_rejected_before_the_model_is_touched() {
        // The order matters: a tapped hotkey must not cost a model load. This passes
        // with no model on disk precisely because the length check comes first.
        let transcriber = WhisperTranscriber::new("/nonexistent/ggml-base.en.bin");
        let tap = vec![0.0; 100];
        assert!(matches!(
            transcriber.transcribe(&tap),
            Err(SttError::TooShort)
        ));
    }

    #[test]
    fn unloading_a_transcriber_that_never_loaded_is_harmless() {
        let transcriber = WhisperTranscriber::new("/nonexistent/model.bin");
        transcriber.unload();
        transcriber.unload();
        assert!(!transcriber.is_ready());
    }

    #[test]
    fn the_thread_count_leaves_the_machine_usable() {
        // Magi transcribes while the user is working. Saturating every core to shave
        // a second off is the wrong trade for a background app.
        let threads = thread_count();
        assert!(threads >= 2, "at least two, or inference crawls");

        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2) as i32;
        assert!(
            threads <= cores.max(2),
            "asked for {threads} threads on {cores} cores"
        );
    }

    #[test]
    fn the_no_speech_threshold_matches_whisper_cpps_own_default() {
        // Not a number picked here. Diverging from upstream's default would need a
        // reason, and there is none yet.
        assert!((NO_SPEECH_THRESHOLD - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn it_satisfies_the_trait_object() {
        let transcriber: Box<dyn Transcriber> =
            Box::new(WhisperTranscriber::new("/nonexistent/model.bin"));
        assert!(!transcriber.is_ready());
    }

    #[test]
    fn the_model_path_is_readable_back() {
        // Settings shows it, and the downloader writes to it.
        let transcriber = WhisperTranscriber::new("/tmp/ggml-base.en.bin");
        assert_eq!(
            transcriber.model_path().to_string_lossy(),
            "/tmp/ggml-base.en.bin"
        );
    }
}
