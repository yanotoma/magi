//! Anthropic's native API. A different protocol, not a variant of the other one.
//!
//! Same shape as `openai.rs` — pure request mapping, pure frame parsing, pure
//! error classification, one thin networked wrapper — and a different wire
//! format at every one of those points. See
//! `.claude/skills/llm-providers/SKILL.md` for the full list of divergences.

use std::collections::HashMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::json;
use tokio::sync::mpsc;

use crate::llm::provider::{LlmError, Provider, Role, StopReason, StreamEvent, TurnRequest};
use crate::llm::sse::{SseFrame, SseParser};

/// Pinned, not tracked. The version header selects a frozen request/response
/// contract; following the latest would mean the wire format could change under
/// Magi without a single line of code changing.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

pub fn messages_url(base_url: &str) -> String {
    format!("{}/messages", base_url.trim_end_matches('/'))
}

/// The headers this API needs.
///
/// Returned as a map rather than applied to a request builder so the choice is
/// testable — the auth header is one of the five things that differ from the
/// OpenAI family, and asserting it is cheaper than discovering it against a 401.
pub fn auth_headers(api_key: Option<&str>) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    // A protocol header, not an auth header: sent with or without a key, since a
    // proxy in front of Anthropic may need no key but still require the version.
    headers.insert(
        "anthropic-version".to_string(),
        ANTHROPIC_VERSION.to_string(),
    );

    if let Some(key) = api_key {
        // Not `Authorization: Bearer`. That belongs to the other family.
        headers.insert("x-api-key".to_string(), key.to_string());
    }

    headers
}

/// Maps Magi's neutral turn onto Anthropic's request body.
pub fn build_request(request: &TurnRequest) -> serde_json::Value {
    let messages: Vec<_> = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": match message.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                "content": message.content,
            })
        })
        .collect();

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        // Required here, unlike the OpenAI family. Omitting it is a 400.
        "max_tokens": request.max_tokens,
    });

    // Top level, not a message. Sending it as a message would have the model
    // read Magi's instructions as the user's words.
    if let Some(system) = &request.system {
        body["system"] = json!(system);
    }

    body
}

/// Translates a stop reason, keeping unknown ones intact.
fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        // `refusal` lands here on purpose. A safety classifier can decline with
        // HTTP 200, and folding that into EndTurn would show the user an empty
        // answer with nothing to explain it.
        other => StopReason::Other(other.to_string()),
    }
}

/// Turns one SSE frame into an event, or `None` for frames with nothing to say.
///
/// Anthropic names its events, so the `event:` field could be used to dispatch.
/// The `type` inside the payload is used instead: it is present either way, and
/// relying on only one of the two means a server that omits the header still
/// parses.
pub fn parse_frame(frame: &SseFrame) -> Option<StreamEvent> {
    let data = frame.data.trim();
    if data.is_empty() {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(data).ok()?;

    match parsed.get("type").and_then(|t| t.as_str())? {
        "content_block_delta" => {
            let delta = parsed.get("delta")?;
            // Only text is the answer. `thinking_delta` is the model's working,
            // and streaming it into the panel would present reasoning as the reply.
            match delta.get("type").and_then(|t| t.as_str())? {
                "text_delta" => delta
                    .get("text")
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty())
                    .map(|t| StreamEvent::Token(t.to_string())),
                _ => None,
            }
        }

        "message_delta" => parsed
            .get("delta")?
            .get("stop_reason")?
            .as_str()
            .map(|raw| StreamEvent::Done(stop_reason(raw))),

        "message_stop" => Some(StreamEvent::Done(StopReason::EndTurn)),

        // A mid-stream failure, reported as an event because HTTP 200 has already
        // been sent. Ignoring it would truncate the answer with no explanation.
        "error" => {
            let message = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("the provider reported an error mid-stream");
            Some(StreamEvent::Done(StopReason::Other(format!(
                "error: {message}"
            ))))
        }

        // message_start, content_block_start, content_block_stop, ping.
        _ => None,
    }
}

/// Turns an HTTP failure into something the user can act on.
pub fn classify_error(url: &str, model: &str, status: u16, body: &str) -> LlmError {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let error = parsed.as_ref().and_then(|v| v.get("error"));
    let kind = error
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    let detail = error
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_string);

    match status {
        401 | 403 => LlmError::Unauthorized {
            url: url.to_string(),
        },
        // Unlike the OpenAI family, a bad model name here comes back as a 400
        // with `not_found_error`; a 404 means the route itself is wrong. Reading
        // the body rather than the status is what tells them apart.
        _ if kind == "not_found_error" => LlmError::ModelNotFound {
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

pub struct Anthropic {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl Anthropic {
    pub fn new(client: reqwest::Client, base_url: String, api_key: Option<String>) -> Self {
        Self {
            client,
            base_url,
            api_key,
        }
    }
}

#[async_trait]
impl Provider for Anthropic {
    async fn turn(
        &self,
        request: TurnRequest,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        let url = messages_url(&self.base_url);
        let mut http = self.client.post(&url).json(&build_request(&request));

        for (name, value) in auth_headers(self.api_key.as_deref()) {
            http = http.header(name, value);
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
                    // A closed receiver is the user dismissing the panel. Stop and
                    // let the request drop: cancellation, not failure.
                    if events.send(event).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }

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
            model: "claude-opus-5".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
        }
    }

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame {
            event: Some(event.to_string()),
            data: data.to_string(),
        }
    }

    #[test]
    fn the_system_prompt_is_a_top_level_field_not_a_message() {
        // The divergence that makes this a separate implementation. Sending it as
        // a message here would have it read as the user's words.
        let body = build_request(&a_request());
        assert_eq!(body["system"], "be brief");

        let messages = body["messages"]
            .as_array()
            .expect("messages must be an array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn system_is_omitted_entirely_when_there_is_none() {
        let body = build_request(&TurnRequest {
            system: None,
            ..a_request()
        });
        assert!(body.get("system").is_none(), "no empty system field");
    }

    #[test]
    fn max_tokens_is_always_present_because_this_api_requires_it() {
        // Optional in the OpenAI family, mandatory here. Omitting it is a 400.
        let body = build_request(&a_request());
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn headers_use_x_api_key_and_a_version_not_bearer() {
        let headers = auth_headers(Some("sk-ant-test"));
        assert_eq!(headers.get("x-api-key"), Some(&"sk-ant-test".to_string()));
        assert_eq!(
            headers.get("anthropic-version"),
            Some(&ANTHROPIC_VERSION.to_string())
        );
        assert!(
            !headers.contains_key("authorization"),
            "Bearer belongs to the other family"
        );
    }

    #[test]
    fn the_version_header_is_sent_even_without_a_key() {
        // It is a protocol header, not an auth header. A proxy in front of
        // Anthropic may need no key but still requires the version.
        assert!(auth_headers(None).contains_key("anthropic-version"));
    }

    #[test]
    fn a_text_delta_becomes_a_token() {
        let event = parse_frame(&frame(
            "content_block_delta",
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}"#,
        ));
        assert_eq!(event, Some(StreamEvent::Token("Hi".into())));
    }

    #[test]
    fn a_thinking_delta_is_not_emitted_as_an_answer() {
        // Reasoning is not the reply. Streaming it into the panel would show the
        // user the model's working as though it were the answer.
        let event = parse_frame(&frame(
            "content_block_delta",
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
        ));
        assert_eq!(event, None);
    }

    #[test]
    fn message_delta_carries_the_stop_reason() {
        assert_eq!(
            parse_frame(&frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#
            )),
            Some(StreamEvent::Done(StopReason::EndTurn))
        );
        assert_eq!(
            parse_frame(&frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#
            )),
            Some(StreamEvent::Done(StopReason::MaxTokens))
        );
    }

    #[test]
    fn a_refusal_is_reported_as_itself_not_as_a_clean_finish() {
        // Safety classifiers can decline with HTTP 200. Mapping this onto EndTurn
        // would show the user an empty answer with no explanation.
        assert_eq!(
            parse_frame(&frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#
            )),
            Some(StreamEvent::Done(StopReason::Other("refusal".into())))
        );
    }

    #[test]
    fn message_stop_ends_the_stream() {
        assert_eq!(
            parse_frame(&frame("message_stop", r#"{"type":"message_stop"}"#)),
            Some(StreamEvent::Done(StopReason::EndTurn))
        );
    }

    #[test]
    fn structural_events_are_ignored() {
        // message_start, content_block_start and ping carry no answer text.
        for (name, data) in [
            ("message_start", r#"{"type":"message_start","message":{}}"#),
            (
                "content_block_start",
                r#"{"type":"content_block_start","content_block":{"type":"text","text":""}}"#,
            ),
            ("ping", r#"{"type":"ping"}"#),
        ] {
            assert_eq!(
                parse_frame(&frame(name, data)),
                None,
                "{name} should be ignored"
            );
        }
    }

    #[test]
    fn an_error_event_becomes_an_error_not_a_silent_stop() {
        // Anthropic can report a mid-stream failure as an SSE event with HTTP 200
        // already sent. Ignoring it would truncate the answer with no explanation.
        let event = parse_frame(&frame(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ));
        assert_eq!(
            event,
            Some(StreamEvent::Done(StopReason::Other(
                "error: Overloaded".into()
            )))
        );
    }

    #[test]
    fn a_malformed_frame_is_skipped_rather_than_failing_the_turn() {
        assert_eq!(
            parse_frame(&frame("content_block_delta", "{not json")),
            None
        );
    }

    #[test]
    fn http_errors_are_classified_from_anthropics_error_shape() {
        let unauthorized = classify_error("https://api.anthropic.com/v1", "m", 401, "");
        assert!(matches!(unauthorized, LlmError::Unauthorized { .. }));

        // Unlike the OpenAI family, a 404 here means the *route* is wrong; a bad
        // model name comes back as a 400 naming the model.
        let bad_model = classify_error(
            "https://api.anthropic.com/v1",
            "claude-nope",
            400,
            r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-nope"}}"#,
        );
        assert!(matches!(bad_model, LlmError::ModelNotFound { .. }));

        let overloaded = classify_error("https://api.anthropic.com/v1", "m", 529, "");
        assert!(matches!(overloaded, LlmError::Http { status: 529, .. }));
    }

    #[test]
    fn the_messages_url_is_built_without_doubling_the_slash() {
        assert_eq!(
            messages_url("https://api.anthropic.com/v1/"),
            "https://api.anthropic.com/v1/messages"
        );
    }
}
