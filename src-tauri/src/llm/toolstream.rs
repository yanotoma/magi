//! Reassembling tool calls that arrive in pieces.
//!
//! Both families stream a tool call the same way in outline and differently in detail:
//! the identity arrives once, the arguments arrive as a run of JSON fragments, and
//! **no individual fragment is valid JSON**. A parser that tries each piece as it
//! lands fails on every one of them; a parser that concatenates first succeeds. That
//! single fact is the reason this module exists rather than the work happening inline
//! in each provider.
//!
//! Pure and stateful, which is an unusual pair and deliberate here. The providers'
//! `parse_frame` functions stay pure functions of one frame — they are the most
//! heavily tested part of that code — and hand their findings to this, which is the
//! only place that remembers anything across frames.

use std::collections::BTreeMap;

use crate::llm::provider::ToolCall;

/// A tool call still arriving.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Partial {
    id: String,
    name: String,
    /// The concatenated argument JSON so far. Not parseable until the stream ends.
    arguments: String,
}

/// Collects streamed tool-call fragments into finished calls.
///
/// Keyed by the index each family stamps on its fragments — a `tool_calls[].index` in
/// the OpenAI family, a content block index in Anthropic's. Both can interleave more
/// than one call in a single response, so the index is what keeps two calls' arguments
/// from being concatenated into one unparseable string.
#[derive(Debug, Default)]
pub struct ToolCallStream {
    /// `BTreeMap` rather than `HashMap`: the calls come back in index order, which is
    /// the order the model asked for them. A model that captures and then reads a file
    /// means those in that order, and a hash map would return them arbitrarily.
    partial: BTreeMap<usize, Partial>,
}

impl ToolCallStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the start of a call: its index, id and name.
    ///
    /// Tolerates being called more than once for an index, and never overwrites a
    /// known id or name with an empty one. The OpenAI family sends the id and name on
    /// the first fragment only and omits them afterwards, so a later fragment that
    /// carries the keys with empty values must not erase what is already known.
    pub fn begin(&mut self, index: usize, id: &str, name: &str) {
        let partial = self.partial.entry(index).or_default();
        if !id.is_empty() {
            partial.id = id.to_string();
        }
        if !name.is_empty() {
            partial.name = name.to_string();
        }
    }

    /// Appends a fragment of argument JSON.
    ///
    /// Creates the entry if a fragment somehow arrives before the start event. Being
    /// forgiving here is cheap and the alternative is discarding a call because two
    /// events came out of order.
    pub fn push_arguments(&mut self, index: usize, fragment: &str) {
        self.partial
            .entry(index)
            .or_default()
            .arguments
            .push_str(fragment);
    }

    /// Whether any call has been seen.
    pub fn is_empty(&self) -> bool {
        self.partial.is_empty()
    }

    /// The finished calls, in the order the model made them.
    ///
    /// Arguments that do not parse become an empty object rather than dropping the
    /// call. Losing the call entirely would be worse than losing its arguments: the
    /// model asked to look at the screen, and every argument `capture_screen` takes
    /// exists to annotate a log entry. A call with no arguments still captures; a call
    /// that vanished leaves the model waiting for a result that never comes, which
    /// both APIs treat as an error.
    ///
    /// A call with no name is dropped, because there is nothing to execute.
    pub fn finish(self) -> Vec<ToolCall> {
        self.partial
            .into_values()
            .filter(|partial| !partial.name.is_empty())
            .map(|partial| {
                let arguments = if partial.arguments.trim().is_empty() {
                    // A tool called with no arguments at all. Legal, and the model
                    // omitting an optional field is the common cause.
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&partial.arguments).unwrap_or_else(|error| {
                        tracing::warn!(
                            %error,
                            name = %partial.name,
                            arguments = %partial.arguments,
                            "tool-call arguments did not parse; treating as empty"
                        );
                        serde_json::json!({})
                    })
                };

                ToolCall {
                    id: partial.id,
                    name: partial.name,
                    arguments,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_are_concatenated_before_being_parsed() {
        // The whole point of the module. Each of these fragments is invalid JSON alone,
        // and a parser that tried them one at a time would fail three times and end up
        // with nothing.
        let mut stream = ToolCallStream::new();
        stream.begin(0, "call_1", "capture_screen");
        for fragment in [r#"{"rea"#, r#"son":"read the "#, r#"stack trace"}"#] {
            assert!(
                serde_json::from_str::<serde_json::Value>(fragment).is_err(),
                "{fragment:?} parses alone, so this test is not testing anything"
            );
            stream.push_arguments(0, fragment);
        }

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "capture_screen");
        assert_eq!(calls[0].arguments["reason"], "read the stack trace");
    }

    #[test]
    fn a_later_fragment_does_not_erase_the_id_or_name() {
        // The OpenAI family sends the id and name on the first fragment and omits them
        // after. A provider that forwards the keys regardless would arrive here with
        // empty strings, and overwriting on every call would leave the finished call
        // nameless and therefore dropped.
        let mut stream = ToolCallStream::new();
        stream.begin(0, "call_1", "capture_screen");
        stream.begin(0, "", "");
        stream.push_arguments(0, "{}");

        let calls = stream.finish();
        assert_eq!(calls.len(), 1, "the call was dropped: {calls:?}");
        assert_eq!(calls[0].name, "capture_screen");
    }

    #[test]
    fn two_calls_keep_their_arguments_apart() {
        // Both families can interleave calls in one response. Without the index their
        // fragments would concatenate into one string that parses as neither.
        let mut stream = ToolCallStream::new();
        stream.begin(0, "a", "capture_screen");
        stream.begin(1, "b", "capture_screen");
        stream.push_arguments(0, r#"{"reason":"first"#);
        stream.push_arguments(1, r#"{"reason":"second"#);
        stream.push_arguments(0, r#""}"#);
        stream.push_arguments(1, r#""}"#);

        let calls = stream.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["reason"], "first");
        assert_eq!(calls[1].arguments["reason"], "second");
    }

    #[test]
    fn calls_come_back_in_the_order_the_model_asked() {
        // Fed out of order on purpose. A hash map would return these arbitrarily, and a
        // model that means "look, then read" would have its steps swapped.
        let mut stream = ToolCallStream::new();
        stream.begin(2, "c", "third");
        stream.begin(0, "a", "first");
        stream.begin(1, "b", "second");

        let calls = stream.finish();
        let names: Vec<&str> = calls.iter().map(|call| call.name.as_str()).collect();
        assert_eq!(names, ["first", "second", "third"]);
    }

    #[test]
    fn a_call_with_no_arguments_still_survives() {
        // Legal, and the usual cause is a model omitting an optional field. Dropping the
        // call would leave the model waiting for a result that never arrives, which both
        // APIs treat as an error rather than as a missing detail.
        let mut stream = ToolCallStream::new();
        stream.begin(0, "call_1", "capture_screen");

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn whitespace_only_arguments_are_treated_as_none() {
        let mut stream = ToolCallStream::new();
        stream.begin(0, "call_1", "capture_screen");
        stream.push_arguments(0, "   \n");
        assert_eq!(stream.finish()[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn unparseable_arguments_keep_the_call() {
        // A truncated stream, or a model that emitted broken JSON. The call is what
        // matters — every argument `capture_screen` takes exists to annotate a log
        // entry, so losing them costs a log line and losing the call costs the answer.
        let mut stream = ToolCallStream::new();
        stream.begin(0, "call_1", "capture_screen");
        stream.push_arguments(0, r#"{"reason":"cut off"#);

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "capture_screen");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn a_nameless_call_is_dropped() {
        // Nothing to execute. Reachable if a stream ends between the start event and
        // the fragment that would have carried the name.
        let mut stream = ToolCallStream::new();
        stream.push_arguments(0, r#"{"reason":"orphan"}"#);
        assert!(stream.finish().is_empty());
    }

    #[test]
    fn a_fragment_arriving_before_the_start_is_still_kept() {
        // Forgiving on purpose: the alternative is discarding a call because two events
        // were delivered out of order, which costs the whole turn.
        let mut stream = ToolCallStream::new();
        stream.push_arguments(0, r#"{"reason":"early"}"#);
        stream.begin(0, "call_1", "capture_screen");

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["reason"], "early");
    }

    #[test]
    fn nothing_streamed_means_no_calls() {
        let stream = ToolCallStream::new();
        assert!(stream.is_empty());
        assert!(stream.finish().is_empty());
    }
}
