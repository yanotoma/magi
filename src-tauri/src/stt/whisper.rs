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

/// The threshold whisper.cpp uses internally to decide a segment is silence.
///
/// Passed to `set_no_speech_thold` and **not** applied again to the output, which is a
/// correction. The first version of this vetoed any segment whose
/// `no_speech_probability` exceeded this value, and that dropped real speech: whisper.cpp
/// uses the number *combined* with `logprob_thold`, discarding a segment only when both
/// conditions hold. Applying it alone is strictly more aggressive than whisper intends,
/// and a short utterance carries a high no-speech probability even when it is plainly
/// speech — two seconds of Spanish came back as zero segments.
///
/// The signal is good; second-guessing whisper's own use of it with a worse rule was not.
/// It decides, and [`super::Transcript::is_meaningful`] catches what gets through.
///
/// 0.6 is whisper.cpp's own default.
///
/// There is deliberately no test asserting the veto is absent. The first attempt scanned
/// this file for the method name and matched the phrase inside its own assertion message —
/// a self-referential check that passed or failed for reasons unrelated to the code. Some
/// things are a review's job rather than a test's.
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
    /// The language to transcribe, or `None` to detect it.
    ///
    /// Held rather than passed per call because it is a setting, and threading it through
    /// `Transcriber::transcribe` would put a whisper-specific concept in a trait that has
    /// no other opinion about languages.
    language: Option<String>,

    /// Whether the model can transcribe anything other than English.
    ///
    /// Kept so `params` can force English for an `.en` model regardless of the setting.
    /// Asking one of those for Spanish does not fail — it writes English words that sound
    /// similar, which is worse than failing.
    multilingual: bool,

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
    ///
    /// `language` is `None` to detect, or an ISO 639-1 code. It is ignored when the model
    /// is English-only.
    pub fn new(model: crate::stt::Model, dir: &Path, language: Option<String>) -> Self {
        Self {
            context: Mutex::new(None),
            model_path: model.path_in(dir),
            language,
            multilingual: model.is_multilingual(),
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
///
/// `language` borrows from the caller for the lifetime of the parameters, which is why
/// this takes a reference rather than owning one: `set_language` stores the pointer.
fn params<'a>(language: Option<&'a str>) -> FullParams<'a, 'a> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // whisper.cpp writes to stdout unless every one of these is off. In a background
    // tray app that is noise going nowhere, and it interleaves with `tracing` output
    // for anyone running with RUST_LOG set.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    params.set_n_threads(thread_count());

    // `None` means detect, and that is the whole of it.
    //
    // `set_detect_language(true)` was here as well, on reasoning I invented: that a `None`
    // language falls back to English unless detection is asked for separately. It does
    // not. whisper.cpp's own source handles `language == nullptr` as auto-detect, and
    // `detect_language` is a different mode entirely — it detects and then
    // `return 0`, transcribing nothing. The symptom was a confident language reading with
    // zero segments, which is exactly what that code path produces.
    //
    // whisper-rs's doc comment on the setter says it "has the same effect as setting the
    // language to auto or None", which is wrong. The C source is the authority.
    params.set_language(language);

    // Transcribe, never translate. Someone speaking Spanish wants Spanish text: silently
    // rendering it in English would be a different feature, and one they did not ask for.
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

        // An English-only model is forced to English whatever the setting says. Asking one
        // for Spanish does not fail — it writes English words that sound similar, which is
        // worse than failing, so the setting is overridden rather than honoured.
        let language: Option<&str> = if self.multilingual {
            self.language.as_deref()
        } else {
            Some("en")
        };

        state
            .full(params(language), samples)
            .map_err(|e| SttError::Failed(e.to_string()))?;

        // `as_iter` and `to_str_lossy`, not the `full_get_segment_text(i)` that most
        // examples show — the API changed. Lossy because the strict version fails the
        // whole transcription on one invalid byte, and losing a sentence to a
        // replacement character is the wrong trade.
        let mut text = String::new();
        let mut kept = 0usize;
        // Counted separately so a lost transcript is diagnosable. When these disagree, the
        // audio reached the model and something here threw the answer away — which is a
        // different problem from the model hearing nothing, and the two were
        // indistinguishable in the log while a filter was silently eating every segment.
        let raw_segments = state.full_n_segments();

        for segment in state.as_iter() {
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
            kept += 1;
        }

        tracing::info!(
            seconds = samples.len() as f32 / super::REQUIRED_RATE as f32,
            raw_segments,
            kept,
            characters = text.len(),
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
        let transcriber =
            WhisperTranscriber::new(crate::stt::Model::Base, Path::new("/nonexistent"), None);
        assert!(!transcriber.is_ready());
        assert!(matches!(transcriber.load(), Err(SttError::ModelMissing)));
    }

    #[test]
    fn audio_too_short_is_rejected_before_the_model_is_touched() {
        // The order matters: a tapped hotkey must not cost a model load. This passes
        // with no model on disk precisely because the length check comes first.
        let transcriber =
            WhisperTranscriber::new(crate::stt::Model::Base, Path::new("/nonexistent"), None);
        let tap = vec![0.0; 100];
        assert!(matches!(
            transcriber.transcribe(&tap),
            Err(SttError::TooShort)
        ));
    }

    #[test]
    fn unloading_a_transcriber_that_never_loaded_is_harmless() {
        let transcriber =
            WhisperTranscriber::new(crate::stt::Model::Base, Path::new("/nonexistent"), None);
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
        // Not a number picked here. Diverging from upstream's default would need a reason,
        // and there is none yet.
        assert!((NO_SPEECH_THRESHOLD - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn it_satisfies_the_trait_object() {
        let transcriber: Box<dyn Transcriber> = Box::new(WhisperTranscriber::new(
            crate::stt::Model::Base,
            Path::new("/nonexistent"),
            None,
        ));
        assert!(!transcriber.is_ready());
    }

    #[test]
    fn the_model_path_is_readable_back() {
        // Settings shows it, and the downloader writes to it.
        let transcriber =
            WhisperTranscriber::new(crate::stt::Model::BaseEn, Path::new("/tmp"), None);
        assert_eq!(
            transcriber.model_path().to_string_lossy(),
            "/tmp/ggml-base.en.bin"
        );
    }
}
