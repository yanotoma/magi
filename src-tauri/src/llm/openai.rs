//! The OpenAI-compatible family: Ollama, LM Studio, OpenAI, OpenRouter, MiMo.
//!
//! Split into three pure functions and one thin wrapper that touches the network.
//! The tempting shape — a single sixty-line `turn()` that builds JSON, sends the
//! request, iterates the stream and parses it — can only be tested against a real
//! server, so in practice it does not get tested. Here the parts that actually
//! break in production are the parts under test, and the untested remainder is a
//! `post().send()`.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::json;
use tokio::sync::mpsc;

use crate::llm::provider::{LlmError, Provider, Role, StopReason, StreamEvent, TurnRequest};
use crate::llm::sse::{SseFrame, SseParser};

pub fn chat_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

/// Maps Magi's neutral turn onto this family's request body.
pub fn build_request(request: &TurnRequest) -> serde_json::Value {
    let mut messages = Vec::with_capacity(request.messages.len() + 1);

    // This family carries the system prompt as a message. Anthropic takes it as
    // a top-level field — the single clearest reason these are two
    // implementations rather than one with a flag.
    if let Some(system) = &request.system {
        messages.push(json!({ "role": "system", "content": system }));
    }

    for message in &request.messages {
        messages.push(json!({
            "role": match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            "content": message.content,
        }));
    }

    json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        // Optional here, required by Anthropic, so Magi always sends it.
        "max_tokens": request.max_tokens,
    })
}

/// Turns one SSE frame into an event, or `None` for frames with nothing to say.
///
/// `None` covers three genuinely uninteresting cases: keepalives, the opening
/// frame that carries only `role`, and malformed JSON. Losing one frame costs a
/// few characters; failing the turn costs the whole answer, and to the user that
/// is indistinguishable from a crash.
pub fn parse_frame(frame: &SseFrame) -> Option<StreamEvent> {
    let data = frame.data.trim();

    if data.is_empty() {
        return None;
    }

    // The OpenAI family's end-of-stream marker. Anthropic has no equivalent.
    if data == "[DONE]" {
        return Some(StreamEvent::Done(StopReason::EndTurn));
    }

    let parsed: serde_json::Value = serde_json::from_str(data).ok()?;
    let choice = parsed.get("choices")?.as_array()?.first()?;

    // Content first: a frame can carry both a token and a finish reason, and
    // dropping the token to report the finish would lose the last word.
    if let Some(content) = choice
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
    {
        return Some(StreamEvent::Token(content.to_string()));
    }

    match choice.get("finish_reason").and_then(|r| r.as_str()) {
        Some("stop") => Some(StreamEvent::Done(StopReason::EndTurn)),
        Some("length") => Some(StreamEvent::Done(StopReason::MaxTokens)),
        // Providers invent their own reasons. Folding an unknown one into
        // EndTurn would report a filtered or errored turn as a clean finish.
        Some(other) => Some(StreamEvent::Done(StopReason::Other(other.to_string()))),
        None => None,
    }
}

/// Turns an HTTP failure into something the user can act on.
pub fn classify_error(url: &str, model: &str, status: u16, body: &str) -> LlmError {
    // Providers explain themselves in the body. Replacing that with a bare status
    // code throws away the only genuinely useful part of the response.
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str().map(str::to_string))
        });

    match status {
        401 | 403 => LlmError::Unauthorized {
            url: url.to_string(),
        },
        // A 404 from this family is far more often a model that was never pulled
        // than a wrong URL, and the two need different fixes.
        404 => LlmError::ModelNotFound {
            url: url.to_string(),
            model: model.to_string(),
        },
        _ => LlmError::Http {
            url: url.to_string(),
            status,
            body: detail.unwrap_or_else(|| body.chars().take(200).collect()),
        },
    }
}

pub struct OpenAiCompatible {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl OpenAiCompatible {
    pub fn new(client: reqwest::Client, base_url: String, api_key: Option<String>) -> Self {
        Self {
            client,
            base_url,
            api_key,
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    async fn turn(
        &self,
        request: TurnRequest,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        let url = chat_url(&self.base_url);
        let mut http = self.client.post(&url).json(&build_request(&request));

        if let Some(key) = &self.api_key {
            http = http.bearer_auth(key);
        }

        let response = http.send().await.map_err(|e| LlmError::Unreachable {
            url: url.clone(),
            reason: e.to_string(),
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(classify_error(&url, &request.model, status.as_u16(), &body));
        }

        let mut parser = SseParser::default();
        let mut body = response.bytes_stream();

        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| LlmError::Unreachable {
                url: url.clone(),
                reason: format!("the stream ended early: {e}"),
            })?;

            for frame in parser.push(&chunk) {
                if let Some(event) = parse_frame(&frame) {
                    // A closed receiver means the user dismissed the panel. Stop,
                    // and let the request drop — this is cancellation, not failure.
                    if events.send(event).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }

        // Servers that close without [DONE] still owe us their last frame.
        for frame in parser.finish() {
            if let Some(event) = parse_frame(&frame) {
                if events.send(event).await.is_err() {
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{Message, TurnRequest};

    fn a_request() -> TurnRequest {
        TurnRequest {
            model: "qwen2.5".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
        }
    }

    fn frame(data: &str) -> SseFrame {
        SseFrame {
            event: None,
            data: data.to_string(),
        }
    }

    #[test]
    fn the_system_prompt_becomes_a_message_in_this_family() {
        // The divergence from Anthropic, which takes it as a top-level field.
        let body = build_request(&a_request());
        let messages = body["messages"]
            .as_array()
            .expect("messages must be an array");

        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "be brief");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hello");
    }

    #[test]
    fn no_system_message_is_sent_when_there_is_no_system_prompt() {
        // An empty system message is not the same as none: some backends treat it
        // as an instruction to say nothing.
        let request = TurnRequest {
            system: None,
            ..a_request()
        };
        let body = build_request(&request);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn streaming_is_requested_and_max_tokens_is_always_sent() {
        let body = build_request(&a_request());
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "qwen2.5");
        assert_eq!(body["max_tokens"], 1024);
    }

    #[test]
    fn a_content_delta_becomes_a_token() {
        let event = parse_frame(&frame(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#));
        assert_eq!(event, Some(StreamEvent::Token("Hi".into())));
    }

    #[test]
    fn the_done_sentinel_ends_the_stream() {
        assert_eq!(
            parse_frame(&frame("[DONE]")),
            Some(StreamEvent::Done(StopReason::EndTurn))
        );
    }

    #[test]
    fn a_finish_reason_ends_the_stream_and_is_translated() {
        // Not every server sends [DONE]; several close after a finish_reason.
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#
            )),
            Some(StreamEvent::Done(StopReason::EndTurn))
        );
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#
            )),
            Some(StreamEvent::Done(StopReason::MaxTokens))
        );
    }

    #[test]
    fn an_unknown_finish_reason_is_carried_through_rather_than_guessed() {
        // Providers invent their own. Mapping one we do not know onto EndTurn
        // would report a filtered or errored turn as a clean finish.
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{},"finish_reason":"content_filter"}]}"#
            )),
            Some(StreamEvent::Done(StopReason::Other(
                "content_filter".into()
            )))
        );
    }

    #[test]
    fn the_first_frame_carrying_only_a_role_is_ignored() {
        // OpenAI opens every stream with {"delta":{"role":"assistant"}}. Emitting
        // an empty token for it would put a leading blank in every answer.
        assert_eq!(
            parse_frame(&frame(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)),
            None
        );
    }

    #[test]
    fn an_empty_content_string_is_not_emitted() {
        assert_eq!(
            parse_frame(&frame(r#"{"choices":[{"delta":{"content":""}}]}"#)),
            None
        );
    }

    #[test]
    fn a_malformed_frame_is_skipped_rather_than_failing_the_turn() {
        // Losing one frame costs a few characters. Failing the turn costs the
        // whole answer, and the user cannot tell the difference from a crash.
        assert_eq!(parse_frame(&frame("{not json")), None);
        assert_eq!(parse_frame(&frame(r#"{"choices":[]}"#)), None);
    }

    #[test]
    fn keepalive_frames_are_ignored() {
        // Ollama sends these during a long prefill.
        assert_eq!(parse_frame(&frame("")), None);
    }

    #[test]
    fn http_errors_are_classified_into_something_actionable() {
        let unauthorized = classify_error("http://x/v1", "m", 401, "");
        assert!(matches!(unauthorized, LlmError::Unauthorized { .. }));

        // 404 from this family almost always means the model is not pulled,
        // which is a different fix from a wrong URL.
        let missing = classify_error(
            "http://x/v1",
            "qwen2.5",
            404,
            r#"{"error":{"message":"model not found"}}"#,
        );
        assert!(matches!(missing, LlmError::ModelNotFound { .. }));

        let other = classify_error("http://x/v1", "m", 500, "boom");
        assert!(matches!(other, LlmError::Http { status: 500, .. }));
    }

    #[test]
    fn the_error_message_from_the_body_is_surfaced_when_there_is_one() {
        // Providers explain themselves in the body. Replacing that with a status
        // code throws away the only useful part.
        let error = classify_error(
            "http://x/v1",
            "m",
            400,
            r#"{"error":{"message":"max_tokens exceeds the model's limit"}}"#,
        );
        assert!(
            error.to_string().contains("max_tokens exceeds"),
            "got: {error}"
        );
    }

    #[test]
    fn the_chat_url_is_built_without_doubling_the_slash() {
        assert_eq!(
            chat_url("http://localhost:11434/v1/"),
            "http://localhost:11434/v1/chat/completions"
        );
    }
}
