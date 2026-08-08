//! The one tool Magi offers a model.
//!
//! Only [`Tier::Agentic`] models ever see it. The tier below can see images but cannot be
//! trusted to call a tool, so it is never told a tool exists — introducing the concept to a
//! model that malforms tool syntax makes it leak that syntax into prose, and by then the
//! capture has already happened anyway.
//!
//! [`Tier::Agentic`]: crate::llm::Tier::Agentic

use crate::llm::provider::ToolSpec;

/// The tool name, as the model sees it and as tool calls come back named.
///
/// A constant rather than a literal because it is matched on in one place and constructed
/// in another, and a typo between them is a tool that is offered and never recognised —
/// which presents as a model that "ignores" the tool.
pub const CAPTURE_SCREEN: &str = "capture_screen";

/// How many captures one turn may perform.
///
/// A model that cannot read what it asked for can ask again, and a model in a bad state
/// will do that forever: each capture adds an image to the history, so an unbounded loop
/// costs money and context at an accelerating rate rather than merely hanging.
///
/// Three rather than one, because asking twice is legitimate — the first look can land
/// while a menu is open or a window is still animating. Beyond that it is not retrying, it
/// is looping.
pub const MAX_CAPTURES_PER_TURN: u8 = 3;

/// The `capture_screen` tool definition.
///
/// The description is written **for the model**, not for a reader of this file. It has to
/// answer one question at the moment of deciding: *do I need to call this?* So it states
/// the consequence of not calling — no sight at all — because a model that assumes it can
/// already see the screen will answer confidently about a screen it never looked at, and
/// that is the failure this whole design exists to avoid.
pub fn capture_screen() -> ToolSpec {
    ToolSpec {
        name: CAPTURE_SCREEN.to_string(),
        description: "Take a screenshot of the user's screen and look at it. \
                      You cannot see the user's screen unless you call this. \
                      Call it whenever answering depends on what is currently displayed — \
                      for example when the user refers to something as \"this\" or \"here\", \
                      asks about an error, a message, a window, or anything on screen, or \
                      when you would otherwise have to guess what they are looking at."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description":
                        "Briefly, what you are looking for. Shown to the user in the \
                         capture log so they can see why their screen was read."
                }
            },
            "required": ["reason"],
            // Both families accept this and some validate against it. Set explicitly
            // rather than left out, so a model cannot invent a second argument that then
            // has to be ignored somewhere downstream.
            "additionalProperties": false
        }),
    }
}

/// Why the screen was read, for the audit log.
///
/// The two tiers arrive at a capture by different routes and both owe the user an answer.
/// A single type so the log has one shape: "captured because you said 'this error'" and
/// "captured because the model asked to read the stack trace" are the same kind of fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A [`Tier::Agentic`] model called the tool and said why.
    ///
    /// [`Tier::Agentic`]: crate::llm::Tier::Agentic
    ModelAsked { reason: String },

    /// The Tier 2 heuristic matched a phrase in what the user said.
    PhraseMatched { phrase: String, language: String },
}

impl Reason {
    /// Reads a reason out of a tool call's arguments.
    ///
    /// Tolerant on purpose. `reason` is required by the schema so that models supply it,
    /// but a missing or non-string value must not turn a capture the model asked for into
    /// an error — the user would get no answer because of a field that exists only to
    /// annotate a log entry. A blank reason is honest; a failed turn is not.
    pub fn from_tool_arguments(arguments: &serde_json::Value) -> Self {
        let reason = arguments
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .unwrap_or("no reason given")
            .to_string();

        Reason::ModelAsked { reason }
    }

    /// One line for the capture log.
    pub fn describe(&self) -> String {
        match self {
            Reason::ModelAsked { reason } => format!("the model asked: {reason}"),
            Reason::PhraseMatched { phrase, .. } => format!("you said \"{phrase}\""),
        }
    }
}

/// How many captures a turn has left.
///
/// Counted rather than trusted to a loop bound, because the loop that would need bounding
/// lives across an async boundary and a `for` there is easy to turn into a `while` by
/// accident. This is the guard, and it is a value the turn owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureBudget {
    spent: u8,
}

impl Default for CaptureBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureBudget {
    /// A fresh budget for one turn.
    pub fn new() -> Self {
        Self { spent: 0 }
    }

    /// Whether another capture is allowed.
    pub fn has_room(self) -> bool {
        self.spent < MAX_CAPTURES_PER_TURN
    }

    /// Records a capture. Saturates rather than wrapping.
    ///
    /// Wrapping would be catastrophic in exactly the situation this type exists for: at
    /// 256 captures the counter would return to zero and the loop it was guarding would
    /// be granted a fresh budget.
    pub fn spend(&mut self) {
        self.spent = self.spent.saturating_add(1);
    }

    /// How many have been used.
    pub fn spent(self) -> u8 {
        self.spent
    }

    /// What to hand back to the model instead of an image, once the budget is gone.
    ///
    /// A tool **result**, not an error. The model asked a reasonable question and should
    /// still get to answer with what it already has; a failed tool call invites a retry,
    /// which is the loop being guarded against. Phrased so that trying again is obviously
    /// pointless rather than merely discouraged.
    pub fn exhausted_message(self) -> String {
        format!(
            "No more screenshots are available this turn ({} of {} used). \
             Answer using the screenshots you have already seen.",
            self.spent, MAX_CAPTURES_PER_TURN
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_is_named_exactly_what_the_matcher_looks_for() {
        // Offered in one place and matched in another. A typo between them is a tool that
        // is never recognised, which looks like a model ignoring it.
        assert_eq!(capture_screen().name, CAPTURE_SCREEN);
        assert_eq!(CAPTURE_SCREEN, "capture_screen");
    }

    #[test]
    fn the_description_tells_the_model_it_is_otherwise_blind() {
        // The one thing the description must convey. A model that assumes it can already
        // see will answer confidently about a screen it never looked at.
        let description = capture_screen().description;
        assert!(
            description.contains("cannot see"),
            "the description does not say the model is blind without it: {description}"
        );
    }

    #[test]
    fn the_schema_is_a_closed_object_with_one_required_string() {
        // Closed on purpose: an open schema lets a model invent an argument that then has
        // to be ignored somewhere, and "ignored somewhere" is where behaviour goes to
        // become unexplainable.
        let parameters = capture_screen().parameters;
        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["additionalProperties"], false);
        assert_eq!(parameters["required"], serde_json::json!(["reason"]));
        assert_eq!(parameters["properties"]["reason"]["type"], "string");
    }

    #[test]
    fn a_missing_reason_still_produces_a_capture() {
        // Tolerance that matters. `reason` exists to annotate a log entry; refusing the
        // capture over it would cost the user their answer for the sake of the log.
        let reason = Reason::from_tool_arguments(&serde_json::json!({}));
        assert_eq!(
            reason,
            Reason::ModelAsked {
                reason: "no reason given".to_string()
            }
        );

        // Same for a wrong type and for whitespace, both of which small models produce.
        assert_eq!(
            Reason::from_tool_arguments(&serde_json::json!({ "reason": 42 })),
            Reason::ModelAsked {
                reason: "no reason given".to_string()
            }
        );
        assert_eq!(
            Reason::from_tool_arguments(&serde_json::json!({ "reason": "   " })),
            Reason::ModelAsked {
                reason: "no reason given".to_string()
            }
        );
    }

    #[test]
    fn a_given_reason_survives_to_the_log() {
        let reason = Reason::from_tool_arguments(
            &serde_json::json!({ "reason": "  read the stack trace  " }),
        );
        assert_eq!(
            reason,
            Reason::ModelAsked {
                reason: "read the stack trace".to_string()
            }
        );
        assert_eq!(reason.describe(), "the model asked: read the stack trace");
    }

    #[test]
    fn both_tiers_explain_themselves_in_the_same_log() {
        // The point of one type for two routes: the user reads one list, not two.
        assert_eq!(
            Reason::PhraseMatched {
                phrase: "this error".to_string(),
                language: "en".to_string()
            }
            .describe(),
            "you said \"this error\""
        );
    }

    #[test]
    fn the_budget_allows_a_retry_and_then_stops() {
        let mut budget = CaptureBudget::new();
        assert!(budget.has_room());

        for _ in 0..MAX_CAPTURES_PER_TURN {
            assert!(budget.has_room());
            budget.spend();
        }

        assert!(!budget.has_room(), "the budget did not stop at the cap");
        assert_eq!(budget.spent(), MAX_CAPTURES_PER_TURN);
    }

    #[test]
    fn spending_past_the_cap_never_wraps_back_into_room() {
        // The catastrophic case this guards. A `u8` that wrapped would hand a runaway
        // loop a fresh budget at 256 captures — the exact situation the cap exists for.
        let mut budget = CaptureBudget::new();
        for _ in 0..1000 {
            budget.spend();
        }
        assert!(!budget.has_room());
        assert_eq!(budget.spent(), u8::MAX);
    }

    #[test]
    fn the_exhausted_message_tells_the_model_to_answer_anyway() {
        // Handed back as a tool result rather than an error, so the model finishes the
        // turn instead of retrying. It has to read as final.
        let mut budget = CaptureBudget::new();
        budget.spend();
        budget.spend();
        budget.spend();

        let message = budget.exhausted_message();
        assert!(message.contains("No more screenshots"), "{message}");
        assert!(message.contains("already seen"), "{message}");
        assert!(message.contains("3 of 3"), "{message}");
    }
}
