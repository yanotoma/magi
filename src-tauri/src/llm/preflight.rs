//! Running the four probes and reading their answers.
//!
//! Split in two on purpose. The *verdict* functions — `saw_the_digit`,
//! `made_a_valid_call`, `returned_the_schema` — are pure and take the reply they
//! judge, so every way a model can almost-pass is a unit test. The orchestration
//! is a thin async shell over them, exercised against [`FakeProvider`].
//!
//! The verdicts are where this module earns its place. A probe that is too
//! generous is worse than no probe at all: it records a capability the model does
//! not have, which promotes it to a tier whose prompt tells it to do something it
//! will fail at. "Non-empty reply" is the version of this that everyone writes
//! first and that passes every model.

use crate::llm::capability::Capabilities;
use crate::llm::probe_image::{probe_image, PROBE_DIGIT};
use crate::llm::provider::{LlmError, ProbeReply, ProbeRequest, Provider, ToolSpec};

/// Written as words as well as digits, because a model reading `7` may answer
/// "seven" and both are correct.
const DIGIT_WORDS: [&str; 10] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];

/// The tool the tool-probe offers.
///
/// Deliberately not `capture_screen`. A model might refuse or hedge about reading
/// someone's screen for reasons that have nothing to do with whether it can call a
/// tool, and that refusal would be recorded as broken tool-calling. A weather
/// lookup is unambiguous, impossible to answer from training data, and
/// uncontroversial.
fn probe_tool() -> ToolSpec {
    ToolSpec {
        name: "get_weather".to_string(),
        description: "Get the current temperature for a city. \
                      The only way to obtain current weather."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        }),
    }
}

/// Whether the model read the digit in the probe image.
///
/// Accepts the digit or its English name, and nothing else. The temptation is to
/// accept any non-empty answer, or to check the model "described an image" — both
/// pass an endpoint that accepted the payload and ignored it, which is the exact
/// failure this probe exists to catch.
///
/// The digit must not merely appear somewhere: "I cannot see any image, but if it
/// were a 7…" contains it. So a negation anywhere in the reply fails the probe,
/// which is the right bias — a model wrongly marked as blind loses a feature it
/// could have had, while one wrongly marked as sighted silently produces wrong
/// answers about a screen it never saw.
pub fn saw_the_digit(reply: &ProbeReply) -> bool {
    let text = reply.text.to_lowercase();

    if text.is_empty() {
        return false;
    }

    // Any hedge about not seeing disqualifies, even alongside the right digit.
    const DENIALS: [&str; 7] = [
        "cannot see",
        "can't see",
        "unable to see",
        "no image",
        "not able to",
        "don't see",
        "do not see",
    ];
    if DENIALS.iter().any(|denial| text.contains(denial)) {
        return false;
    }

    let digit = char::from_digit(PROBE_DIGIT as u32, 10)
        .map(|c| c.to_string())
        .unwrap_or_default();
    let word = DIGIT_WORDS[(PROBE_DIGIT % 10) as usize];

    text.contains(&digit) || text.contains(word)
}

/// Whether the model made a structurally usable tool call.
///
/// Three conditions, and every one has been seen to fail on its own with small
/// local models: a call was parsed at all, it names the tool that was offered, and
/// its arguments contain the required field. A call to the right tool with an empty
/// object is not usable, and would break an agentic loop at the first turn.
pub fn made_a_valid_call(reply: &ProbeReply) -> bool {
    reply.tool_calls.iter().any(|call| {
        call.name == "get_weather"
            && call
                .arguments
                .get("city")
                .and_then(|c| c.as_str())
                .is_some_and(|c| !c.trim().is_empty())
    })
}

/// Whether the model returned JSON matching the requested schema.
///
/// Checked by parsing and inspecting the fields, never by looking for braces. The
/// reply can arrive two ways — as text for the OpenAI family, or as a forced tool
/// call for Anthropic, which has no `response_format` — so both are accepted.
///
/// Text is stripped of a markdown code fence first. A model that returns correct
/// JSON wrapped in ```json is doing the thing being tested; failing it for the
/// fence would report a capability it has as one it lacks.
pub fn returned_the_schema(reply: &ProbeReply) -> bool {
    let from_tool_call = reply
        .tool_calls
        .first()
        .is_some_and(|call| matches_probe_schema(&call.arguments));

    if from_tool_call {
        return true;
    }

    let text = strip_code_fence(reply.text.trim());
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .is_some_and(|value| matches_probe_schema(&value))
}

/// Removes a surrounding markdown code fence, if there is one.
fn strip_code_fence(text: &str) -> &str {
    let text = text.trim();
    if !text.starts_with("```") {
        return text;
    }

    // Skip the opening fence and any language tag on the same line.
    let after_open = match text.find('\n') {
        Some(newline) => &text[newline + 1..],
        None => return text,
    };

    match after_open.rfind("```") {
        Some(close) => after_open[..close].trim(),
        None => after_open.trim(),
    }
}

/// The shape the structured-output probe asks for.
fn matches_probe_schema(value: &serde_json::Value) -> bool {
    value.get("city").and_then(|c| c.as_str()).is_some()
        && value.get("celsius").and_then(|c| c.as_i64()).is_some()
}

/// The JSON Schema sent with the structured-output probe.
fn probe_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "city": { "type": "string" },
            "celsius": { "type": "integer" }
        },
        "required": ["city", "celsius"],
        "additionalProperties": false
    })
}

/// Runs the four probes against a model and reports what it can do.
///
/// Sequential rather than concurrent, deliberately. Four simultaneous requests to
/// a local server running one model on one GPU queue behind each other anyway, and
/// against a metered API they can trip a rate limit that would be recorded as a
/// capability failure. Pre-flight is not on a path anyone is waiting on
/// keystroke-by-keystroke.
///
/// Reachability is checked first and short-circuits. Every later probe would fail
/// against an unreachable endpoint, and recording "no vision, no tools" for a
/// wrong URL would send the user to change models when the fix is a typo.
///
/// A capability probe that errors is recorded as `false`, not propagated. An
/// endpoint that returns 400 for an image payload has answered the question the
/// probe asked — it cannot do this — and failing the whole pre-flight would leave
/// the user with no information at all instead of a partial answer.
pub async fn run(provider: &dyn Provider, model: &str) -> Capabilities {
    let mut capabilities = Capabilities::default();

    // Probe 1 — reachability. A trivial completion rather than GET /v1/models,
    // because a provider can list a model it cannot serve, and because the
    // Anthropic-shaped endpoints have no equivalent listing route.
    let reach = ProbeRequest::new(model, "Reply with the single word: ok");
    match provider.probe(reach).await {
        Ok(_) => capabilities.reachable = true,
        Err(error) => {
            tracing::info!(summary = %error.log_summary(), model, "pre-flight: endpoint not reachable");
            return capabilities;
        }
    }

    // Probe 2 — vision.
    if let Some(image) = probe_image() {
        let mut request = ProbeRequest::new(
            model,
            "What single digit is shown in this image? \
             Reply with only the digit.",
        );
        request.image = Some(image);

        capabilities.vision = match provider.probe(request).await {
            Ok(reply) => {
                let saw = saw_the_digit(&reply);
                // A truncated failure is a Magi problem wearing a model
                // problem's clothes: the model may well have been about to name
                // the digit when the budget ran out. Logged apart so a bug report
                // says which one happened, instead of leaving both looking like a
                // model that cannot see.
                if !saw && reply.truncated {
                    tracing::warn!(
                        model,
                        "pre-flight: the vision reply hit the token limit before answering — \
                         this is a truncation, not necessarily a model without vision"
                    );
                }
                saw
            }
            Err(error) => {
                tracing::info!(summary = %error.log_summary(), model, "pre-flight: vision probe failed");
                false
            }
        };
    } else {
        // Magi could not build its own test image. That is a Magi bug and must not
        // be reported as a model limitation, so it is logged loudly and left false.
        tracing::error!("pre-flight: could not generate the probe image");
    }

    // Probe 3 — tool calling. The prompt cannot be answered from training data, so
    // a model that does not call the tool has genuinely failed rather than simply
    // preferred to answer directly.
    let mut request = ProbeRequest::new(
        model,
        "What is the current temperature in Kitchener, Ontario? \
         Use the available tool.",
    );
    request.tool = Some(probe_tool());
    capabilities.tools = match provider.probe(request).await {
        Ok(reply) => made_a_valid_call(&reply),
        Err(error) => {
            tracing::info!(summary = %error.log_summary(), model, "pre-flight: tool probe failed");
            false
        }
    };

    // Probe 4 — structured output.
    let mut request = ProbeRequest::new(
        model,
        "Return JSON with the city \"Kitchener\" and a celsius temperature of 21.",
    );
    request.json_schema = Some(probe_schema());
    capabilities.structured_output = match provider.probe(request).await {
        Ok(reply) => returned_the_schema(&reply),
        Err(error) => {
            tracing::info!(summary = %error.log_summary(), model, "pre-flight: structured output probe failed");
            false
        }
    };

    capabilities
}

/// How many probes [`run`] issues at most, for progress reporting in Settings.
pub const PROBE_COUNT: usize = 4;

/// Whether an error means "do not bother with the remaining probes".
pub fn is_fatal(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Unreachable { .. }
            | LlmError::Unauthorized { .. }
            | LlmError::ModelNotFound { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::capability::{assign, Tier};
    use crate::llm::provider::{FakeProvider, ToolCall};
    use serde_json::json;

    /// The digit the probe actually asks about, and its English name.
    ///
    /// Derived rather than written out. These tests hardcoded `"7"` and `"seven"`,
    /// so changing `PROBE_DIGIT` to a more legible glyph left five of them asserting
    /// against a digit the probe no longer sends — the same mistake as the test that
    /// pinned `max_tokens` to a literal: an assertion about a value rather than about
    /// the behaviour.
    fn digit() -> String {
        PROBE_DIGIT.to_string()
    }

    fn word() -> &'static str {
        DIGIT_WORDS[(PROBE_DIGIT % 10) as usize]
    }

    fn text(body: &str) -> ProbeReply {
        ProbeReply {
            text: body.to_string(),
            ..Default::default()
        }
    }

    fn truncated(body: &str) -> ProbeReply {
        ProbeReply {
            text: body.to_string(),
            truncated: true,
            ..Default::default()
        }
    }

    fn call(name: &str, arguments: serde_json::Value) -> ProbeReply {
        ProbeReply {
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: name.to_string(),
                arguments,
            }],
            ..Default::default()
        }
    }

    // ---- the vision verdict ------------------------------------------------

    #[test]
    fn the_digit_alone_passes() {
        assert!(saw_the_digit(&text(&digit())));
        assert!(saw_the_digit(&text(&format!("The digit is {}.", digit()))));
    }

    #[test]
    fn the_digit_as_a_word_passes() {
        // A model reading a numeral may well answer in words, and both are right.
        assert!(saw_the_digit(&text(word())));
        assert!(saw_the_digit(&text(&format!(
            "It shows the number {}",
            word().to_uppercase()
        ))));
    }

    #[test]
    fn a_wrong_digit_fails() {
        // Any digit other than the one the probe sends.
        let wrong = (PROBE_DIGIT + 1) % 10;
        assert!(!saw_the_digit(&text(&wrong.to_string())));
        assert!(!saw_the_digit(&text("It looks like a different shape.")));
    }

    #[test]
    fn an_empty_reply_fails() {
        assert!(!saw_the_digit(&text("")));
    }

    #[test]
    fn describing_an_image_without_reading_it_fails() {
        // What an endpoint that accepted the payload and ignored it produces: a
        // confident description with no digit in it. "Non-empty reply" would pass
        // this, which is why that is not the test.
        assert!(!saw_the_digit(&text(
            "The image shows a black square with a white shape in the centre."
        )));
    }

    #[test]
    fn a_denial_fails_even_when_it_names_the_right_digit() {
        // The subtle one. A hedge that happens to guess correctly must not pass, or
        // a blind model gets promoted to a vision tier on the strength of a guess.
        assert!(!saw_the_digit(&text(&format!(
            "I cannot see any image, but if I had to guess it would be a {}.",
            digit()
        ))));
        assert!(!saw_the_digit(&text(&format!(
            "I don't see an image. Perhaps {}?",
            digit()
        ))));
        assert!(!saw_the_digit(&text(&format!(
            "I'm unable to see images. Is it {}?",
            word()
        ))));
    }

    #[test]
    fn a_truncated_reply_is_flagged_separately_from_a_wrong_one() {
        // Both fail the verdict, and only one of them is the model's fault. A
        // reasoning model can spend the entire budget thinking before it writes a
        // character, so the empty answer says nothing about whether it saw the
        // image.
        let cut_off = truncated("");
        assert!(!saw_the_digit(&cut_off));
        assert!(cut_off.truncated, "the flag is what tells the two apart");

        let wrong = text("It shows a black square.");
        assert!(!saw_the_digit(&wrong));
        assert!(!wrong.truncated);
    }

    #[test]
    fn truncation_does_not_override_a_correct_answer() {
        // A model that named the digit and then ran out of room while elaborating
        // has passed. The flag is diagnostic, not a veto.
        assert!(saw_the_digit(&truncated(&format!(
            "The digit shown is {}, and the image is",
            digit()
        ))));
    }

    #[test]
    fn every_probe_uses_the_configured_budget() {
        // The floor itself is a compile-time assertion next to the constant, so it
        // cannot be lowered without a build error. What is left to check here is
        // that probes actually use it rather than setting their own.
        assert_eq!(
            ProbeRequest::new("m", "p").max_tokens,
            crate::llm::provider::PROBE_MAX_TOKENS
        );
    }

    // ---- the tool verdict --------------------------------------------------

    #[test]
    fn a_well_formed_call_passes() {
        assert!(made_a_valid_call(&call(
            "get_weather",
            json!({"city": "Kitchener"})
        )));
    }

    #[test]
    fn prose_describing_the_call_fails() {
        // The failure this probe exists for. Small local models narrate the call
        // instead of emitting it, and the narration parses to no calls at all.
        assert!(!made_a_valid_call(&text(
            "I'll use get_weather with city=Kitchener to find out."
        )));
    }

    #[test]
    fn a_call_with_no_arguments_fails() {
        // Structurally a call, and useless. An agentic loop would break on the
        // first turn with nothing to act on.
        assert!(!made_a_valid_call(&call("get_weather", json!({}))));
    }

    #[test]
    fn a_call_with_a_blank_city_fails() {
        assert!(!made_a_valid_call(&call(
            "get_weather",
            json!({"city": ""})
        )));
        assert!(!made_a_valid_call(&call(
            "get_weather",
            json!({"city": "   "})
        )));
    }

    #[test]
    fn a_call_to_a_tool_that_was_never_offered_fails() {
        // Models invent tool names. Accepting any call at all would pass a model
        // that cannot follow the definition it was given.
        assert!(!made_a_valid_call(&call(
            "search_web",
            json!({"city": "Kitchener"})
        )));
    }

    #[test]
    fn the_probe_tool_is_not_capture_screen() {
        // A model might decline to read someone's screen for reasons unrelated to
        // tool-calling, and that refusal would be recorded as broken tool support.
        assert_ne!(probe_tool().name, "capture_screen");
    }

    // ---- the structured output verdict -------------------------------------

    #[test]
    fn matching_json_as_text_passes() {
        assert!(returned_the_schema(&text(
            r#"{"city":"Kitchener","celsius":21}"#
        )));
    }

    #[test]
    fn matching_json_inside_a_code_fence_passes() {
        // A model that wraps correct JSON in a fence has done the thing being
        // tested. Failing it for the fence would report a capability it has as one
        // it lacks.
        assert!(returned_the_schema(&text(
            "```json\n{\"city\":\"Kitchener\",\"celsius\":21}\n```"
        )));
        assert!(returned_the_schema(&text(
            "```\n{\"city\":\"Kitchener\",\"celsius\":21}\n```"
        )));
    }

    #[test]
    fn matching_json_from_a_forced_tool_call_passes() {
        // Anthropic has no response_format, so its structured output arrives as a
        // tool call. Both routes must count.
        assert!(returned_the_schema(&call(
            "respond",
            json!({"city": "Kitchener", "celsius": 21})
        )));
    }

    #[test]
    fn json_missing_a_required_field_fails() {
        assert!(!returned_the_schema(&text(r#"{"city":"Kitchener"}"#)));
        assert!(!returned_the_schema(&text(r#"{"celsius":21}"#)));
    }

    #[test]
    fn json_with_the_wrong_type_fails() {
        // "21" is not an integer. A schema the model half-followed is not schema
        // support, and later milestones would parse this and break.
        assert!(!returned_the_schema(&text(
            r#"{"city":"Kitchener","celsius":"21"}"#
        )));
    }

    #[test]
    fn prose_around_the_json_fails() {
        // Best-effort JSON, which is what a model without real schema support
        // produces. Accepting it would record a capability the caller cannot rely
        // on.
        assert!(!returned_the_schema(&text(
            "Sure! Here you go: {\"city\":\"Kitchener\",\"celsius\":21}"
        )));
    }

    #[test]
    fn prose_alone_fails() {
        assert!(!returned_the_schema(&text(
            "It is 21 degrees in Kitchener."
        )));
    }

    // ---- orchestration -----------------------------------------------------

    #[tokio::test]
    async fn an_unreachable_endpoint_stops_after_the_first_probe() {
        // One queued failure, and no replies for the other three. If `run` tried
        // them anyway the fake would report "no reply queued" — so this asserts the
        // short-circuit rather than trusting it.
        let provider = FakeProvider::answering_probes(vec![Err(LlmError::Unreachable {
            url: "http://localhost:11434/v1".into(),
            reason: "connection refused".into(),
        })]);

        let capabilities = run(&provider, "qwen2.5").await;

        assert!(!capabilities.reachable);
        assert_eq!(assign(&capabilities), Tier::Unreachable);
    }

    #[tokio::test]
    async fn a_fully_capable_model_reaches_the_agentic_tier() {
        let provider = FakeProvider::answering_probes(vec![
            Ok(text("ok")),
            Ok(text(&digit())),
            Ok(call("get_weather", json!({"city": "Kitchener"}))),
            Ok(text(r#"{"city":"Kitchener","celsius":21}"#)),
        ]);

        let capabilities = run(&provider, "gpt-5").await;

        assert_eq!(
            capabilities,
            Capabilities {
                reachable: true,
                vision: true,
                tools: true,
                structured_output: true
            }
        );
        assert_eq!(assign(&capabilities), Tier::Agentic);
    }

    #[tokio::test]
    async fn a_model_that_sees_but_cannot_call_tools_reaches_the_heuristic_tier() {
        // The case tier 2 exists for, and the reason the fake queues replies rather
        // than returning one canned answer.
        let provider = FakeProvider::answering_probes(vec![
            Ok(text("ok")),
            Ok(text(word())),
            Ok(text("I would call get_weather('Kitchener') to check.")),
            Ok(text("It's about 21 degrees.")),
        ]);

        let capabilities = run(&provider, "llava").await;

        assert!(capabilities.vision);
        assert!(!capabilities.tools);
        assert_eq!(assign(&capabilities), Tier::Heuristic);
    }

    #[tokio::test]
    async fn a_blind_model_reaches_the_text_only_tier_even_with_good_tools() {
        let provider = FakeProvider::answering_probes(vec![
            Ok(text("ok")),
            Ok(text("I cannot see images.")),
            Ok(call("get_weather", json!({"city": "Kitchener"}))),
            Ok(text(r#"{"city":"Kitchener","celsius":21}"#)),
        ]);

        let capabilities = run(&provider, "llama3.2").await;

        assert!(!capabilities.vision);
        assert!(capabilities.tools);
        assert_eq!(assign(&capabilities), Tier::TextOnly);
    }

    #[tokio::test]
    async fn a_failing_capability_probe_is_recorded_as_false_not_propagated() {
        // An endpoint that 400s on an image payload has answered the question. The
        // remaining probes must still run, or one rejection would leave the user
        // with no information rather than partial information.
        let provider = FakeProvider::answering_probes(vec![
            Ok(text("ok")),
            Err(LlmError::Http {
                url: "http://localhost:11434/v1".into(),
                status: 400,
                body: "this model does not support images".into(),
            }),
            Ok(call("get_weather", json!({"city": "Kitchener"}))),
            Ok(text(r#"{"city":"Kitchener","celsius":21}"#)),
        ]);

        let capabilities = run(&provider, "llama3.2").await;

        assert!(capabilities.reachable, "reachability already succeeded");
        assert!(!capabilities.vision);
        assert!(
            capabilities.tools,
            "a rejected image must not stop the tool probe"
        );
        assert_eq!(assign(&capabilities), Tier::TextOnly);
    }

    #[test]
    fn only_setup_errors_are_fatal() {
        assert!(is_fatal(&LlmError::Unreachable {
            url: "u".into(),
            reason: "r".into()
        }));
        assert!(is_fatal(&LlmError::Unauthorized { url: "u".into() }));
        assert!(is_fatal(&LlmError::ModelNotFound {
            url: "u".into(),
            model: "m".into()
        }));

        // A 400 is the model declining a capability, which is a result rather than
        // a reason to stop asking.
        assert!(!is_fatal(&LlmError::Http {
            url: "u".into(),
            status: 400,
            body: "b".into()
        }));
    }
}
