//! What was read, when, and why.
//!
//! The design doc's privacy claim is that "the screen is read at specific, logged,
//! model-initiated moments — not continuously". This module is the *logged* part, and
//! without it that sentence is an intention rather than a fact the user can check.
//!
//! ## In memory, not on disk
//!
//! Deliberate, and the opposite of what "audit log" usually implies. A persisted list of
//! which windows were open and when is a record of someone's working day — more sensitive
//! than most of what Magi handles, and produced as a side effect of a feature nobody asked
//! to be surveilled by. `config.toml` is meant to be safe to paste into a bug report, and a
//! sibling file full of window titles would quietly undo that.
//!
//! So the log answers "what has Magi looked at since it started", which is the question a
//! user actually asks, and it disappears on quit. If a persistent record is ever wanted it
//! should be an opt-in with its own consent, not a default that accumulated.
//!
//! ## Bounded
//!
//! A long session with a chatty model could otherwise grow this without limit. It keeps the
//! most recent [`MAX_ENTRIES`] and drops the oldest, because the recent ones are the ones
//! someone is asking about.

use std::sync::Mutex;

use crate::capture::source::Subject;
use crate::llm::tools::Reason;

/// How many captures are remembered.
///
/// Generous relative to how often a capture should happen — at three per turn, this is a
/// hundred turns of maximally chatty behaviour — and small enough that the memory is
/// irrelevant next to a single screenshot.
pub const MAX_ENTRIES: usize = 300;

/// One capture that happened.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    /// Milliseconds since the Unix epoch.
    ///
    /// Passed in rather than read from the clock here, so the log is testable without
    /// making time an input to assertions. The caller has a clock; this does not need one.
    pub at: u64,

    /// What was captured.
    pub subject: Subject,

    /// Why. Either the model asked, or the user's own words matched.
    #[serde(serialize_with = "serialise_reason")]
    pub reason: Reason,

    /// Encoded width in pixels, after downscaling.
    pub width: u32,

    /// Encoded height in pixels, after downscaling.
    pub height: u32,

    /// What the image cost a vision model.
    ///
    /// Stored rather than recomputed on read, so a later change to the token formula cannot
    /// silently rewrite history — a log that revises what it says happened is not a log.
    pub visual_tokens: u32,
}

/// Serialises a [`Reason`] as the sentence the user reads, plus its parts.
///
/// A custom serialiser rather than `#[derive(Serialize)]` on `Reason`, because the frontend
/// wants the rendered line and deriving would give it a tagged enum to re-render. Keeping
/// the phrasing in one place means the log and any notification agree.
fn serialise_reason<S>(reason: &Reason, serialiser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeStruct;

    let mut state = serialiser.serialize_struct("Reason", 2)?;
    state.serialize_field("text", &reason.describe())?;
    state.serialize_field(
        "asked_by",
        match reason {
            Reason::ModelAsked { .. } => "model",
            Reason::PhraseMatched { .. } | Reason::UserAsked => "you",
        },
    )?;
    state.end()
}

/// Every capture this run of Magi has made.
///
/// `Mutex` rather than `RwLock`: writes happen once per capture and reads once when a
/// settings pane opens, so there is no read contention to optimise for, and `Mutex` has one
/// poisoning story instead of two.
#[derive(Debug, Default)]
pub struct CaptureLog {
    entries: Mutex<std::collections::VecDeque<Entry>>,
}

impl CaptureLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a capture, dropping the oldest entry if the log is full.
    ///
    /// Infallible on purpose. This is called on the capture path, and a failure to write an
    /// audit entry must never be a reason the user does not get their answer — a poisoned
    /// lock loses the entry silently rather than propagating.
    pub fn record(&self, entry: Entry) {
        let Ok(mut entries) = self.entries.lock() else {
            tracing::warn!("the capture log is poisoned; this capture will not be listed");
            return;
        };

        if entries.len() >= MAX_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    /// Every capture, **most recent first**.
    ///
    /// Reversed here rather than in the UI: "what did you just look at" is the question, and
    /// a list that answers it needs no scrolling.
    pub fn entries(&self) -> Vec<Entry> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries.iter().rev().cloned().collect()
    }

    /// How many captures have been made and are still remembered.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// Whether nothing has been captured yet.
    ///
    /// The state a first-time user is in, and the one worth saying out loud in Settings:
    /// "Magi has not read your screen" is reassuring in a way an empty table is not.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets everything.
    ///
    /// Offered because a user who has just shown Magi something private should be able to
    /// remove the record of it without quitting.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: u64) -> Entry {
        Entry {
            at,
            subject: Subject::Display {
                id: 1,
                label: "Built-in Retina Display".to_string(),
            },
            reason: Reason::ModelAsked {
                reason: "read the stack trace".to_string(),
            },
            width: 1372,
            height: 882,
            visual_tokens: 1568,
        }
    }

    #[test]
    fn a_new_log_says_nothing_has_been_read() {
        // The first-run state, and one Settings should state rather than render as an
        // empty table.
        let log = CaptureLog::new();
        assert!(log.is_empty());
        assert_eq!(log.entries(), Vec::new());
    }

    #[test]
    fn the_most_recent_capture_is_first() {
        let log = CaptureLog::new();
        log.record(entry(1_000));
        log.record(entry(2_000));
        log.record(entry(3_000));

        let listed: Vec<u64> = log.entries().iter().map(|entry| entry.at).collect();
        assert_eq!(listed, vec![3_000, 2_000, 1_000]);
    }

    #[test]
    fn the_log_is_bounded_and_drops_the_oldest() {
        // A long session with a chatty model would otherwise grow this without limit.
        let log = CaptureLog::new();
        for at in 0..(MAX_ENTRIES as u64 + 50) {
            log.record(entry(at));
        }

        assert_eq!(log.len(), MAX_ENTRIES);

        let listed = log.entries();
        assert_eq!(
            listed.first().map(|entry| entry.at),
            Some(MAX_ENTRIES as u64 + 49),
            "the newest entry was dropped instead of the oldest"
        );
        assert_eq!(
            listed.last().map(|entry| entry.at),
            Some(50),
            "the oldest entries were not the ones dropped"
        );
    }

    #[test]
    fn clearing_forgets_everything() {
        let log = CaptureLog::new();
        log.record(entry(1));
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn both_reasons_serialise_with_a_sentence_and_who_asked() {
        // The frontend renders `text` directly, so the phrasing lives in one place and the
        // log cannot disagree with a notification about the same capture.
        let log = CaptureLog::new();
        log.record(entry(1_000));
        log.record(Entry {
            reason: Reason::PhraseMatched {
                phrase: "this error".to_string(),
                language: "en".to_string(),
            },
            ..entry(2_000)
        });

        let json = serde_json::to_value(log.entries()).expect("serialisable");

        assert_eq!(json[0]["reason"]["asked_by"], "you");
        assert_eq!(json[0]["reason"]["text"], "you said \"this error\"");
        assert_eq!(json[1]["reason"]["asked_by"], "model");
        assert_eq!(
            json[1]["reason"]["text"],
            "the model asked: read the stack trace"
        );
    }

    #[test]
    fn an_entry_carries_what_the_capture_cost() {
        // So Settings can show the price of a capture rather than only that one happened.
        let json = serde_json::to_value(entry(1)).expect("serialisable");
        assert_eq!(json["visual_tokens"], 1568);
        assert_eq!(json["width"], 1372);
        assert_eq!(json["height"], 882);
    }

    #[test]
    fn the_subject_survives_into_the_log() {
        // By the time a user reads the log the window may be gone, so the entry has to
        // carry its own description rather than a handle to look up.
        let log = CaptureLog::new();
        log.record(Entry {
            subject: Subject::Window {
                id: 10,
                title: "src/main.rs".to_string(),
                app: "Zed".to_string(),
            },
            ..entry(1)
        });

        let json = serde_json::to_value(log.entries()).expect("serialisable");
        assert_eq!(json[0]["subject"]["kind"], "window");
        assert_eq!(json[0]["subject"]["app"], "Zed");
    }

    #[test]
    fn recording_from_several_threads_loses_nothing() {
        // `AppState` is shared, and a capture can be recorded from any `spawn_blocking`
        // worker. Losing entries under contention would make the log quietly incomplete,
        // which is worse than not having one.
        let log = std::sync::Arc::new(CaptureLog::new());
        let mut handles = Vec::new();

        for thread in 0..8u64 {
            let log = std::sync::Arc::clone(&log);
            handles.push(std::thread::spawn(move || {
                for step in 0..20 {
                    log.record(entry(thread * 100 + step));
                }
            }));
        }

        for handle in handles {
            handle.join().expect("no thread panicked");
        }

        assert_eq!(log.len(), 160);
    }
}
