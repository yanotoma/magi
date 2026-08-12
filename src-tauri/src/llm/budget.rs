//! Turning a context window into a history budget.
//!
//! [`history::fit`] takes a number and trims a conversation to it. This module decides what
//! that number should be, which is the harder half: until now it was one constant, chosen to
//! bound growth rather than to match anything, and the comment above it admitted as much.
//!
//! ## A context window is not a budget
//!
//! The window has to hold four things, and history is only one of them:
//!
//! ```text
//! [ system prompt ][ tool definitions ][ history ][ the model's reply ]
//! `------------------ overhead ------------------'            `- reserved -'
//! ```
//!
//! Setting the history budget to the window overflows precisely on the long conversations
//! truncation exists to handle — the request is accepted right up to the turn where the reply
//! has nowhere to go. So the budget is the window *minus* everything else in the request,
//! which is why [`plan`] returns both numbers: they are computed from one subtraction and
//! disagreeing about it is the bug.
//!
//! ## Why the reply is capped twice
//!
//! Magi asks for [`MAX_REPLY`] tokens of answer. On a 200k window that is nothing worth
//! thinking about. On an 8k local model it is half the window, reserved for a reply that is
//! usually a few hundred tokens — and paid for by the conversation, which gets truncated to
//! protect space the model will not use.
//!
//! So a reply may claim at most a quarter of a small window. The quarter is not tuned against
//! anything; it is a statement that on a small model the conversation matters more than room
//! for an answer nobody asked to be long. [`MIN_REPLY`] keeps it from reaching zero.
//!
//! ## An unknown window changes nothing
//!
//! Most providers cannot be asked how big their window is, and Magi does not guess. With no
//! number configured the result is [`DEFAULT_BUDGET`] and the reply cap Magi asked for, which
//! is exactly the behaviour that shipped before this module existed. That equivalence is a
//! test, not a comment — it is what makes the feature additive.

use crate::llm::history;
use crate::llm::provider::ToolSpec;

/// The history budget used when no context window is known.
///
/// Deliberately the same figure the single constant carried before this module existed, so
/// that a user who configures nothing sees no change. It bounds growth and matches no
/// particular model, which was always its honest description.
pub const DEFAULT_BUDGET: u32 = 16_000;

/// The most answer Magi ever asks for.
///
/// Enough for a long explanation with code in it. Raising this costs history on every model
/// small enough to notice, which is the trade the quarter-window rule below exists to make
/// visible.
pub const MAX_REPLY: u32 = 4_096;

/// The least answer worth asking for.
///
/// A window too small to hold overhead plus this is a window Magi cannot serve, and the right
/// outcome is the provider's own error rather than a request for nothing. Asking for zero
/// tokens of reply would answer a question nobody asked — the same reasoning that makes
/// [`history::fit`] keep the newest exchange even when it alone is too big.
pub const MIN_REPLY: u32 = 512;

/// What the JSON around a tool definition costs, per tool.
///
/// The keys and braces go over the wire with the values inside them: Anthropic wraps a tool
/// in `{"name":…,"description":…,"input_schema":…}` and the OpenAI family adds a
/// `{"type":"function","function":{…}}` layer on top of that. Twenty tokens is small against
/// a 16k budget, and it is counted anyway because the direction is not symmetric — this sum
/// is subtracted from the window, so anything missing from it gets spent twice.
///
/// The larger of the two families, since one number serves both and erring high is the safe
/// side of a subtraction.
const TOOL_FRAMING: u32 = 20;

/// The largest share of the window a reply may claim.
///
/// Expressed as a divisor: a quarter. Only binds when the window is small enough that
/// [`MAX_REPLY`] would be a meaningful fraction of it.
const REPLY_SHARE_DIVISOR: u32 = 4;

/// How a turn divides the context window it has.
///
/// Both fields come out of one subtraction on purpose. Computing them separately is how a
/// request ends up asking for more reply than the space it left itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// What [`history::fit`] may spend on the conversation.
    pub history_budget: u32,
    /// What to send as `max_tokens`.
    pub max_tokens: u32,
}

/// Divide a context window between the conversation and the reply.
///
/// `overhead` is everything in the request that is not history and not the reply — the system
/// prompt and the tool definitions, as measured by [`request_overhead`]. `context_tokens` is
/// `None` when Magi has not been told the window, which is the common case.
pub fn plan(context_tokens: Option<u32>, overhead: u32) -> Plan {
    let Some(context) = context_tokens else {
        return Plan {
            history_budget: DEFAULT_BUDGET,
            max_tokens: MAX_REPLY,
        };
    };

    let share = (context / REPLY_SHARE_DIVISOR).max(MIN_REPLY);
    let max_tokens = MAX_REPLY.min(share);

    Plan {
        history_budget: context.saturating_sub(overhead.saturating_add(max_tokens)),
        max_tokens,
    }
}

/// What the fixed part of a request costs — the system prompt and the tools.
///
/// Estimated with [`history::estimate_text`], the same rule the conversation is measured by.
/// A second estimator here would be a second answer to "how long is this string", and the two
/// would drift apart in the direction that matters: this figure is subtracted from the
/// window, so under-counting it over-spends on history.
///
/// Tools are measured from their serialised JSON rather than their fields, because the schema
/// braces and keys go over the wire too — a 1.7 KB tool definition is 400-odd tokens whose
/// absence from this sum would be spent twice.
pub fn request_overhead(system: Option<&str>, tools: &[ToolSpec]) -> u32 {
    let mut total = system.map_or(0, history::estimate_text);

    for tool in tools {
        let serialised = serde_json::to_string(&tool.parameters).unwrap_or_default();
        total = total
            .saturating_add(TOOL_FRAMING)
            .saturating_add(history::estimate_text(&tool.name))
            .saturating_add(history::estimate_text(&tool.description))
            .saturating_add(history::estimate_text(&serialised));
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, description: &str, parameters: serde_json::Value) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        }
    }

    #[test]
    fn an_unknown_window_reproduces_the_behaviour_that_shipped_before() {
        // The whole feature is additive on this assertion: a user who configures nothing gets
        // the constant that was there before, and the reply cap Magi always asked for.
        let plan = plan(None, 1_400);

        assert_eq!(plan.history_budget, DEFAULT_BUDGET);
        assert_eq!(plan.max_tokens, MAX_REPLY);
    }

    #[test]
    fn an_unknown_window_ignores_its_overhead() {
        // Nothing to subtract from, so overhead cannot change the answer. If it ever does,
        // the None path has started pretending to know a window.
        assert_eq!(plan(None, 0), plan(None, 100_000));
    }

    #[test]
    fn a_large_window_leaves_almost_all_of_itself_to_the_conversation() {
        let plan = plan(Some(200_000), 1_400);

        assert_eq!(plan.max_tokens, MAX_REPLY);
        assert_eq!(plan.history_budget, 200_000 - 1_400 - MAX_REPLY);
    }

    #[test]
    fn a_large_window_beats_the_default_rather_than_being_capped_by_it() {
        // The point of the task: a 200k model should not be trimmed to 16k.
        let plan = plan(Some(200_000), 1_400);

        assert!(plan.history_budget > DEFAULT_BUDGET);
    }

    #[test]
    fn a_small_window_gives_the_conversation_more_than_the_reply() {
        // 8k with a 4096 reply reserved leaves 2696 for history — less than the reply, for a
        // reply that is usually a few hundred tokens. The quarter rule exists for this case.
        let plan = plan(Some(8_192), 1_400);

        assert_eq!(plan.max_tokens, 8_192 / REPLY_SHARE_DIVISOR);
        assert!(
            plan.history_budget > plan.max_tokens,
            "history {} should beat reply {}",
            plan.history_budget,
            plan.max_tokens
        );
    }

    #[test]
    fn the_reply_never_exceeds_what_magi_asks_for() {
        // A quarter of a big window is far more than MAX_REPLY, and Magi does not want it.
        for context in [16_384, 32_768, 131_072, 1_000_000] {
            assert_eq!(
                plan(Some(context), 1_400).max_tokens,
                MAX_REPLY,
                "context {context}"
            );
        }
    }

    #[test]
    fn the_reply_never_falls_below_the_floor() {
        // A quarter of 1024 is 256, which is not an answer.
        assert_eq!(plan(Some(1_024), 100).max_tokens, MIN_REPLY);
    }

    #[test]
    fn a_window_too_small_for_its_own_overhead_yields_no_history_rather_than_underflowing() {
        // 512 of window against 1400 of system prompt. The subtraction must not wrap: `fit`
        // keeps the newest exchange regardless, so the request still goes out and the
        // provider says what is wrong with it.
        let plan = plan(Some(512), 1_400);

        assert_eq!(plan.history_budget, 0);
        assert_eq!(plan.max_tokens, MIN_REPLY);
    }

    #[test]
    fn a_zero_window_is_survived() {
        let plan = plan(Some(0), 0);

        assert_eq!(plan.history_budget, 0);
        assert_eq!(plan.max_tokens, MIN_REPLY);
    }

    #[test]
    fn the_three_parts_never_claim_more_than_the_window() {
        // The invariant the struct exists to hold, over a range wide enough to cross both
        // caps and the saturating floor.
        for context in [
            0, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 128_000, 1_000_000,
        ] {
            for overhead in [0, 500, 1_400, 5_000, 50_000] {
                let plan = plan(Some(context), overhead);
                let claimed = plan
                    .history_budget
                    .saturating_add(plan.max_tokens)
                    .saturating_add(overhead);

                // Only a window too small to hold overhead plus the reply floor may exceed
                // itself, and then by no more than that floor plus the overhead it cannot fit.
                if plan.history_budget > 0 {
                    assert!(
                        claimed <= context,
                        "context {context}, overhead {overhead}, claimed {claimed}"
                    );
                }
            }
        }
    }

    #[test]
    fn more_overhead_never_buys_more_history() {
        let mut previous = u32::MAX;

        for overhead in [0, 100, 1_000, 4_000, 10_000] {
            let budget = plan(Some(32_768), overhead).history_budget;
            assert!(budget <= previous, "overhead {overhead} raised the budget");
            previous = budget;
        }
    }

    #[test]
    fn a_bigger_window_never_buys_less_history() {
        let mut previous = 0;

        for context in [1_024, 4_096, 8_192, 16_384, 65_536, 200_000] {
            let budget = plan(Some(context), 1_400).history_budget;
            assert!(budget >= previous, "context {context} lowered the budget");
            previous = budget;
        }
    }

    #[test]
    fn nothing_in_the_request_costs_nothing() {
        assert_eq!(request_overhead(None, &[]), 0);
    }

    #[test]
    fn the_system_prompt_is_measured_by_the_same_rule_as_a_message() {
        let system = "x".repeat(800);

        assert_eq!(
            request_overhead(Some(&system), &[]),
            history::estimate_text(&system)
        );
    }

    #[test]
    fn a_tool_costs_its_schema_and_not_only_its_name() {
        // The failure this guards: measuring name + description and forgetting that the JSON
        // schema is the larger half of what goes over the wire.
        let parameters = json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "x".repeat(600) }
            },
            "required": ["target"]
        });

        let with_schema = request_overhead(None, &[tool("capture_screen", "look", parameters)]);
        let without = request_overhead(None, &[tool("capture_screen", "look", json!({}))]);

        assert!(
            with_schema > without + 100,
            "schema of 600-odd chars was not counted: {with_schema} vs {without}"
        );
    }

    #[test]
    fn a_tool_costs_more_than_the_strings_inside_it() {
        // The wrapping JSON travels too. Counting only the values would under-count, and
        // this sum is subtracted from the window — so what is missing here is spent twice.
        let name = "capture_screen";
        let description = "look at the screen";
        let parameters = json!({ "type": "object" });
        let serialised = serde_json::to_string(&parameters).expect("serialises");

        let values = history::estimate_text(name)
            + history::estimate_text(description)
            + history::estimate_text(&serialised);

        assert_eq!(
            request_overhead(None, &[tool(name, description, parameters)]),
            values + TOOL_FRAMING
        );
    }

    #[test]
    fn tools_accumulate() {
        let one = tool("a", "does a thing", json!({ "type": "object" }));
        let two = tool("b", "does another", json!({ "type": "object" }));

        let both = request_overhead(None, std::slice::from_ref(&one));
        let all = request_overhead(None, &[one, two.clone()]);

        assert!(all > both);
        assert_eq!(all, both + request_overhead(None, &[two]));
    }

    #[test]
    fn overhead_is_never_under_counted_into_a_wrap() {
        // Saturating arithmetic, checked rather than assumed: under-counting here is spent
        // twice, once by the reply and once by history.
        let huge = "x".repeat(4_000);
        let tools: Vec<ToolSpec> = (0..64)
            .map(|i| tool(&format!("t{i}"), &huge, json!({ "type": "object" })))
            .collect();

        assert!(request_overhead(Some(&huge), &tools) > 60_000);
    }
}
