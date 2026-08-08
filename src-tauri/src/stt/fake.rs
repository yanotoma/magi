//! A transcriber that returns what it was told to.
//!
//! Every test of the session loop, the panel, and the error paths goes through this.
//! The real transcriber needs a 141 MB model and a C++ toolchain; neither belongs in
//! a test of Magi's own logic.

use super::{SttError, Transcriber, Transcript};

use std::sync::Mutex;

pub struct FakeTranscriber {
    /// Replies handed out in order, one per call.
    ///
    /// A queue rather than one canned answer, so a test can script a sequence — a
    /// silence artefact followed by real speech is exactly how a session behaves when
    /// the user taps the hotkey and then uses it properly.
    replies: Mutex<std::collections::VecDeque<Result<Transcript, SttError>>>,

    ready: bool,

    /// Every sample slice this was asked to transcribe.
    ///
    /// Recorded so a test can assert what reached the transcriber rather than only
    /// what came back — which is how the 16 kHz contract gets checked end to end
    /// instead of trusted.
    received: Mutex<Vec<Vec<f32>>>,
}

impl FakeTranscriber {
    /// Returns `text` for every call.
    pub fn saying(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            replies: Mutex::new(std::collections::VecDeque::new()),
            ready: true,
            received: Mutex::new(Vec::new()),
        }
        .always(Ok(Transcript::new(text)))
    }

    /// Returns the queued results in order.
    pub fn scripted(replies: Vec<Result<Transcript, SttError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            ready: true,
            received: Mutex::new(Vec::new()),
        }
    }

    /// Fails every call, which is how the UI's error paths get exercised.
    pub fn failing(error: SttError) -> Self {
        Self::scripted(Vec::new()).always(Err(error))
    }

    /// Reports itself as not ready, the first-run state before the model exists.
    pub fn unready() -> Self {
        Self {
            replies: Mutex::new(std::collections::VecDeque::new()),
            ready: false,
            received: Mutex::new(Vec::new()),
        }
        .always(Err(SttError::ModelMissing))
    }

    /// Repeats one result indefinitely.
    ///
    /// Implemented by refilling rather than by a separate mode, so `scripted` and
    /// this share one code path in `transcribe`.
    fn always(self, reply: Result<Transcript, SttError>) -> Self {
        if let Ok(mut queue) = self.replies.lock() {
            // A generous fill rather than an unbounded loop: a test that calls more
            // than this many times has a different problem.
            for _ in 0..64 {
                queue.push_back(reply.clone());
            }
        }
        self
    }

    /// The sample slices this was asked to transcribe, in order.
    pub fn received(&self) -> Vec<Vec<f32>> {
        self.received.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<Transcript, SttError> {
        if let Ok(mut received) = self.received.lock() {
            received.push(samples.to_vec());
        }

        let queued = self
            .replies
            .lock()
            .map_err(|_| SttError::Failed("the fake's queue was poisoned".into()))?
            .pop_front();

        // Running out means a test asked for more transcriptions than it scripted.
        // Saying so beats returning something plausible that makes the test pass for
        // the wrong reason.
        queued.unwrap_or(Err(SttError::Failed(
            "no transcript was queued for this call".into(),
        )))
    }

    fn is_ready(&self) -> bool {
        self.ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_what_it_was_told_to() {
        let transcriber = FakeTranscriber::saying("what is on my screen");
        let transcript = transcriber.transcribe(&[0.1; 16_000]).expect("ok");
        assert_eq!(transcript.text, "what is on my screen");
        assert!(transcript.is_meaningful());
    }

    #[test]
    fn scripted_replies_come_back_in_order() {
        // The sequence that matters: a tapped hotkey producing a silence artefact,
        // then the same hotkey used properly.
        let transcriber = FakeTranscriber::scripted(vec![
            Ok(Transcript::new("Thank you.")),
            Ok(Transcript::new("why is this failing")),
        ]);

        let first = transcriber.transcribe(&[0.0; 100]).expect("ok");
        assert!(
            !first.is_meaningful(),
            "a silence artefact must be rejected"
        );

        let second = transcriber.transcribe(&[0.1; 16_000]).expect("ok");
        assert!(second.is_meaningful());
    }

    #[test]
    fn it_records_what_it_was_asked_to_transcribe() {
        // How the 16 kHz contract gets checked end to end: a session test can assert
        // the sample count that reached here, not just the text that came back.
        let transcriber = FakeTranscriber::saying("hello");
        transcriber.transcribe(&[0.5; 8_000]).expect("ok");
        transcriber.transcribe(&[0.5; 16_000]).expect("ok");

        let received = transcriber.received();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].len(), 8_000);
        assert_eq!(received[1].len(), 16_000);
    }

    #[test]
    fn a_failing_transcriber_keeps_failing() {
        let transcriber = FakeTranscriber::failing(SttError::Failed("model crashed".into()));
        for _ in 0..3 {
            assert!(transcriber.transcribe(&[0.1; 16_000]).is_err());
        }
    }

    #[test]
    fn an_unready_transcriber_reports_the_missing_model() {
        // The first-run state. Settings needs to distinguish it from a broken model
        // so it can offer the download instead of an error.
        let transcriber = FakeTranscriber::unready();
        assert!(!transcriber.is_ready());
        assert!(matches!(
            transcriber.transcribe(&[0.1; 16_000]),
            Err(SttError::ModelMissing)
        ));
    }

    #[test]
    fn running_out_of_script_is_an_error_rather_than_a_plausible_answer() {
        let transcriber = FakeTranscriber::scripted(vec![Ok(Transcript::new("one"))]);
        transcriber
            .transcribe(&[0.1; 100])
            .expect("the scripted call");
        assert!(
            transcriber.transcribe(&[0.1; 100]).is_err(),
            "an unscripted call must fail loudly"
        );
    }

    #[test]
    fn the_trait_is_object_safe() {
        // `session` will hold a `Box<dyn Transcriber>`.
        let transcriber: Box<dyn Transcriber> = Box::new(FakeTranscriber::saying("hi"));
        assert!(transcriber.is_ready());
        assert!(transcriber.transcribe(&[0.1; 16_000]).is_ok());
    }
}
