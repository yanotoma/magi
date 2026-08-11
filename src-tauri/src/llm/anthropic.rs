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

use base64::Engine as _;

use crate::llm::provider::{
    Image, LlmError, Message, ProbeReply, ProbeRequest, Provider, StopReason, StreamEvent,
    ToolCall, TurnRequest,
};
use crate::llm::sse::{SseFrame, SseParser};

const BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

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
    let messages: Vec<_> = request.messages.iter().map(wire_message).collect();

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

    if !request.tools.is_empty() {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                // `input_schema`, not `parameters`, and unwrapped rather than nested
                // under a `function` key. Two differences from the OpenAI family in one
                // short object, which is why these are separate implementations.
                "input_schema": tool.parameters,
            }))
            .collect::<Vec<_>>());
    }

    body
}

/// One neutral message as Anthropic's content blocks.
///
/// Always an array, even for plain text. Anthropic accepts a bare string there, but
/// mixing the two forms across a conversation is how a subtle bug hides — and the
/// agentic path needs the array anyway, since an assistant turn carries prose and a
/// `tool_use` block together.
fn wire_message(message: &Message) -> serde_json::Value {
    match message {
        Message::User { text, images } => {
            let mut content = vec![json!({ "type": "text", "text": text })];
            for image in images {
                content.push(image_block(image));
            }
            json!({ "role": "user", "content": content })
        }

        Message::Assistant { text, calls } => {
            let mut content = Vec::with_capacity(calls.len() + 1);
            // Text first, then the calls, which is the order the model produced them and
            // the order Anthropic returns them in.
            if !text.is_empty() {
                content.push(json!({ "type": "text", "text": text }));
            }
            for call in calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    // Actual JSON here, unlike the OpenAI family's encoded string.
                    "input": call.arguments,
                }));
            }
            json!({ "role": "assistant", "content": content })
        }

        Message::ToolResult {
            call_id,
            text,
            images,
        } => {
            // A `user` message, which reads oddly and is what the API wants: the result
            // is the user's side of the exchange even though no person wrote it.
            let mut inner = Vec::with_capacity(images.len() + 1);
            if !text.is_empty() {
                inner.push(json!({ "type": "text", "text": text }));
            }
            // Inside the tool result, unlike the OpenAI family, where the images have to
            // follow as a separate user message.
            for image in images {
                inner.push(image_block(image));
            }

            json!({
                "role": "user",
                "content": [json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": inner,
                })],
            })
        }
    }
}

/// An image block. Base64 with no newlines — Anthropic rejects a wrapped payload.
fn image_block(image: &Image) -> serde_json::Value {
    use base64::Engine;
    json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": image.media_type,
            "data": base64::engine::general_purpose::STANDARD.encode(&image.bytes),
        }
    })
}

/// Translates a stop reason, keeping unknown ones intact.
fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        // Not the end of the turn: the loop answers the call and asks again.
        "tool_use" => StopReason::ToolUse,
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
        // A tool call beginning. `id` and `name` are both present here; `input` is an
        // empty object and must not be read — the arguments arrive as fragments after.
        "content_block_start" => {
            let block = parsed.get("content_block")?;
            if block.get("type")?.as_str()? != "tool_use" {
                return None;
            }
            Some(StreamEvent::ToolStart {
                index: parsed.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize,
                id: block.get("id")?.as_str()?.to_string(),
                name: block.get("name")?.as_str()?.to_string(),
            })
        }

        "content_block_delta" => {
            let delta = parsed.get("delta")?;
            // Text is the answer; thinking is the working. They travel as
            // different events so the panel can show them as different things —
            // or hide one — rather than running them together.
            match delta.get("type").and_then(|t| t.as_str())? {
                "text_delta" => delta
                    .get("text")
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty())
                    .map(|t| StreamEvent::Token(t.to_string())),
                "thinking_delta" => delta
                    .get("thinking")
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty())
                    .map(|t| StreamEvent::Thinking(t.to_string())),
                // Tool arguments: a fragment, never valid JSON alone. Anthropic's own
                // docs call these "partial JSON strings" and note that the final `input`
                // is an object. The first fragment is always `""`, which is why the empty
                // string is passed through rather than filtered — the accumulator treats
                // appending nothing as nothing.
                "input_json_delta" => {
                    delta
                        .get("partial_json")
                        .and_then(|j| j.as_str())
                        .map(|json| StreamEvent::ToolArguments {
                            index: parsed.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                as usize,
                            json: json.to_string(),
                        })
                }
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

/// Maps a probe onto Anthropic's request body.
///
/// Every attachment differs from the OpenAI family's form, which is why this is a
/// separate function rather than a shared one with a flag:
///
/// - the image is a `source` object with the media type in its own field, not a
///   `data:` URL crammed into `image_url`
/// - the tool is `input_schema`, not `function.parameters`
/// - there is no `response_format`, so structured output is requested by handing
///   the model a tool whose schema *is* the schema and requiring it
pub fn build_probe(request: &ProbeRequest) -> serde_json::Value {
    // Content is an array of blocks whenever there is more than text. Anthropic
    // accepts a bare string too, and the text-only probes use it so that a
    // reachability check stays as close to a minimal request as possible.
    let content = match &request.image {
        Some(image) => json!([
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.media_type,
                    "data": BASE64.encode(&image.bytes),
                }
            },
            { "type": "text", "text": request.prompt }
        ]),
        None => json!(request.prompt),
    };

    let mut body = json!({
        "model": request.model,
        "messages": [{ "role": "user", "content": content }],
        "stream": false,
        // Required here, unlike the other family.
        "max_tokens": request.max_tokens,
    });

    // Top-level, and omitted rather than sent as null. Some proxies in front of
    // this API reject an explicit null.
    if let Some(system) = &request.system {
        body["system"] = json!(system);
    }

    if let Some(tool) = &request.tool {
        body["tools"] = json!([{
            "name": tool.name,
            "description": tool.description,
            // `input_schema`, not `parameters`, and not wrapped in a `function`.
            "input_schema": tool.parameters,
        }]);
        body["tool_choice"] = json!({ "type": "auto" });
    }

    // Anthropic has no `response_format`. A schema is requested by offering a tool
    // that takes it as input and requiring that tool — so the reply arrives as a
    // tool call rather than as JSON in the text, which the probe accounts for.
    if let Some(schema) = &request.json_schema {
        body["tools"] = json!([{
            "name": "respond",
            "description": "Return the answer in the required structure.",
            "input_schema": schema,
        }]);
        body["tool_choice"] = json!({ "type": "tool", "name": "respond" });
    }

    body
}

/// Reads a non-streaming message response.
///
/// The reply is a list of content blocks, so text and tool calls arrive
/// interleaved in one array rather than in separate fields as the other family
/// sends them. Arguments are already a JSON object here — no second parse, unlike
/// the OpenAI family's stringified `arguments`.
pub fn parse_probe_reply(url: &str, body: &str) -> Result<ProbeReply, LlmError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|e| LlmError::MalformedResponse {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let blocks = parsed
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| LlmError::MalformedResponse {
            url: url.to_string(),
            reason: "no content array in the response".to_string(),
        })?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(chunk) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(chunk);
                }
            }
            Some("tool_use") => {
                if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                    tool_calls.push(ToolCall {
                        // Read if present, not required: the probe never answers a call.
                        id: block
                            .get("id")
                            .and_then(|id| id.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        name: name.to_string(),
                        arguments: block.get("input").cloned().unwrap_or(json!({})),
                    });
                }
            }
            // Thinking blocks and anything added to the format later. A probe
            // cares about text and calls; ignoring the rest is what keeps a new
            // block type from turning into a parse failure.
            _ => {}
        }
    }

    // `max_tokens` here is what the OpenAI family calls `finish_reason: "length"`.
    let truncated = parsed.get("stop_reason").and_then(|r| r.as_str()) == Some("max_tokens");

    Ok(ProbeReply {
        text,
        tool_calls,
        truncated,
    })
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

    async fn probe(&self, request: ProbeRequest) -> Result<ProbeReply, LlmError> {
        let url = messages_url(&self.base_url);
        let mut http = self.client.post(&url).json(&build_probe(&request));

        for (name, value) in auth_headers(self.api_key.as_deref()) {
            http = http.header(name, value);
        }

        let response = http.send().await.map_err(|e| LlmError::Unreachable {
            url: url.clone(),
            reason: e.to_string(),
        })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(classify_error(&url, &request.model, status.as_u16(), &body));
        }

        parse_probe_reply(&url, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_with_a_screenshot_carries_an_image_block() {
        let body = build_request(&with_messages(vec![Message::user_seeing(
            "what is this error",
            vec![a_screenshot()],
        )]));

        let content = body["messages"][0]["content"].as_array().expect("blocks");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
    }

    #[test]
    fn a_streamed_tool_call_reassembles_from_the_documented_events() {
        // Anthropic's own example sequence, abbreviated: the block start carries `id` and
        // `name` with `input` as an empty object, then `input_json_delta` fragments whose
        // first is always the empty string.
        let events = [
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01T","name":"capture_screen","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"reason\":"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"the error\"}"}}"#,
            ),
        ];

        let mut stream = crate::llm::toolstream::ToolCallStream::new();
        for (name, data) in events {
            match parse_frame(&frame(name, data)) {
                Some(StreamEvent::ToolStart { index, id, name }) => stream.begin(index, &id, &name),
                Some(StreamEvent::ToolArguments { index, json }) => {
                    stream.push_arguments(index, &json)
                }
                other => panic!("unexpected {other:?}"),
            }
        }

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_01T");
        assert_eq!(calls[0].name, "capture_screen");
        assert_eq!(calls[0].arguments["reason"], "the error");
    }

    #[test]
    fn a_text_block_start_is_not_a_tool_call() {
        // Every response opens a text block. Treating that as a tool start would invent a
        // nameless call on every single turn.
        assert_eq!(
            parse_frame(&frame(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#
            )),
            None
        );
    }

    #[test]
    fn the_tool_use_stop_reason_is_not_the_end_of_the_turn() {
        assert_eq!(
            parse_frame(&frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#
            )),
            Some(StreamEvent::Done(StopReason::ToolUse))
        );
    }

    #[test]
    fn the_block_index_separates_parallel_calls() {
        // Anthropic streams parallel calls as separate blocks, distinguished only by the
        // index that also positions them in the final content array.
        let second = parse_frame(&frame(
            "content_block_start",
            r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_02","name":"capture_screen","input":{}}}"#,
        ));
        assert!(matches!(
            second,
            Some(StreamEvent::ToolStart { index: 2, .. })
        ));
    }

    fn a_screenshot() -> Image {
        Image {
            media_type: "image/png",
            bytes: vec![0x89, 0x50, 0x4E, 0x47],
        }
    }

    fn a_call() -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: "capture_screen".to_string(),
            arguments: serde_json::json!({ "reason": "read the error" }),
        }
    }

    fn with_messages(messages: Vec<Message>) -> TurnRequest {
        TurnRequest {
            model: "m".to_string(),
            system: None,
            messages,
            max_tokens: 100,
            tools: vec![crate::llm::tools::capture_screen(1)],
        }
    }

    #[test]
    fn a_tool_definition_uses_input_schema_and_no_wrapper() {
        // Two differences from the OpenAI family in one short object: the key is
        // `input_schema`, and there is no `function` wrapper.
        let body = build_request(&with_messages(vec![Message::user("hi")]));
        let tool = &body["tools"][0];
        assert_eq!(tool["name"], "capture_screen");
        assert_eq!(tool["input_schema"]["type"], "object");
        assert!(tool.get("function").is_none(), "{tool}");
        assert!(tool.get("type").is_none(), "{tool}");
    }

    #[test]
    fn tool_call_input_goes_out_as_an_object() {
        // An object, not the encoded string the OpenAI family wants.
        let body = build_request(&with_messages(vec![Message::Assistant {
            text: "Let me look.".to_string(),
            calls: vec![a_call()],
        }]));

        let content = body["messages"][0]["content"].as_array().expect("array");
        assert_eq!(content[0]["type"], "text", "text comes first");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "call_1");
        assert!(
            content[1]["input"].is_object(),
            "input must be JSON, not a string: {}",
            content[1]["input"]
        );
    }

    #[test]
    fn a_screenshot_rides_inside_the_tool_result() {
        // Unlike the OpenAI family. `tool_result.content` accepts nested blocks, so the
        // image belongs to the result rather than to a message after it — which keeps
        // the association between the call and its picture explicit.
        let body = build_request(&with_messages(vec![Message::ToolResult {
            call_id: "call_1".to_string(),
            text: "Screenshot captured.".to_string(),
            images: vec![a_screenshot()],
        }]));

        let messages = body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 1, "no extra message is needed: {body}");
        assert_eq!(messages[0]["role"], "user", "a result is the user's turn");

        let result = &messages[0]["content"][0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "call_1");

        let inner = result["content"].as_array().expect("nested blocks");
        assert_eq!(inner[1]["type"], "image");
        assert_eq!(inner[1]["source"]["type"], "base64");
        assert_eq!(inner[1]["source"]["media_type"], "image/png");
        let data = inner[1]["source"]["data"].as_str().expect("base64");
        assert!(!data.contains('\n'), "Anthropic rejects a wrapped payload");
    }

    #[test]
    fn content_is_always_an_array_even_for_plain_text() {
        // A bare string is accepted, but mixing the two forms across a conversation is
        // where a subtle bug hides.
        let body = build_request(&with_messages(vec![Message::user("hello")]));
        assert!(body["messages"][0]["content"].is_array(), "{body}");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    }
    use crate::llm::provider::{Message, TurnRequest};

    fn a_request() -> TurnRequest {
        TurnRequest {
            model: "claude-opus-5".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
            tools: Vec::new(),
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
    fn a_thinking_delta_is_a_thinking_event_not_a_token() {
        // Reasoning is not the reply, but it is worth showing separately. As a
        // Token it would read as the answer; as its own event the panel can put
        // it behind a disclosure or drop it.
        let event = parse_frame(&frame(
            "content_block_delta",
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
        ));
        assert_eq!(event, Some(StreamEvent::Thinking("hmm".into())));
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

/// Probe mapping and reply reading. Every assertion here has a counterpart in
/// `openai.rs` asserting the opposite shape — which is the clearest available
/// evidence that these two really are separate protocols rather than one with a
/// flag.
#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::llm::provider::{Image, ToolSpec};

    fn probe() -> ProbeRequest {
        ProbeRequest::new("claude-test", "what digit is shown?")
    }

    fn a_tool() -> ToolSpec {
        ToolSpec {
            name: "get_weather".into(),
            description: "Look up the weather".into(),
            parameters: json!({
                "type": "object",
                "properties": { "city": { "type": "string" } }
            }),
        }
    }

    #[test]
    fn max_tokens_is_always_present() {
        // Required by this API. Omitting it is an error, not a default. The
        // assertion is that the field is present and carries the configured budget,
        // not that the budget is any particular number.
        let body = build_probe(&probe());
        assert!(body.get("max_tokens").is_some());
        assert_eq!(
            body["max_tokens"],
            json!(crate::llm::provider::PROBE_MAX_TOKENS)
        );
    }

    #[test]
    fn the_system_prompt_is_top_level_and_omitted_when_absent() {
        let body = build_probe(&probe());
        assert!(
            body.get("system").is_none(),
            "an explicit null is rejected by some proxies in front of this API"
        );

        let mut with_system = probe();
        with_system.system = Some("be brief".into());
        let body = build_probe(&with_system);
        assert_eq!(body["system"], json!("be brief"));
        // Never as a message. That is the other family's shape.
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn an_image_is_a_source_block_not_a_data_url() {
        let mut request = probe();
        request.image = Some(Image::png(vec![1, 2, 3]));
        let body = build_probe(&request);

        let blocks = body["messages"][0]["content"]
            .as_array()
            .expect("an array once an image is attached");
        assert_eq!(blocks[0]["type"], json!("image"));
        assert_eq!(blocks[0]["source"]["type"], json!("base64"));
        // The media type lives in its own field here, rather than inside a URL.
        assert_eq!(blocks[0]["source"]["media_type"], json!("image/png"));
        assert_eq!(blocks[0]["source"]["data"], json!("AQID"));

        let data = blocks[0]["source"]["data"].as_str().expect("a string");
        assert!(
            !data.starts_with("data:"),
            "the data: prefix belongs to the OpenAI family, not here"
        );
    }

    #[test]
    fn a_text_probe_sends_content_as_a_bare_string() {
        assert!(build_probe(&probe())["messages"][0]["content"].is_string());
    }

    #[test]
    fn a_tool_uses_input_schema_and_no_function_wrapper() {
        let mut request = probe();
        request.tool = Some(a_tool());
        let body = build_probe(&request);

        assert_eq!(body["tools"][0]["name"], json!("get_weather"));
        assert!(body["tools"][0]["input_schema"].is_object());
        // Sending `function.parameters` here gets the tool silently ignored rather
        // than rejected, which would look exactly like a model that cannot call
        // tools.
        assert!(body["tools"][0]["function"].is_null());
        assert!(body["tools"][0]["parameters"].is_null());
    }

    #[test]
    fn tool_choice_is_an_object_not_a_string() {
        let mut request = probe();
        request.tool = Some(a_tool());
        assert_eq!(
            build_probe(&request)["tool_choice"],
            json!({"type": "auto"})
        );
    }

    #[test]
    fn a_schema_probe_forces_a_tool_because_there_is_no_response_format() {
        // This API has no `response_format`. Structured output is requested by
        // handing the model a tool whose input schema *is* the schema, and
        // requiring it — so the answer arrives as a tool call, not as JSON text.
        let mut request = probe();
        request.json_schema = Some(json!({ "type": "object" }));
        let body = build_probe(&request);

        assert!(body["response_format"].is_null());
        assert_eq!(body["tools"][0]["name"], json!("respond"));
        assert_eq!(
            body["tool_choice"],
            json!({"type": "tool", "name": "respond"})
        );
    }

    #[test]
    fn text_blocks_are_concatenated() {
        // A reply is a list of blocks, so text can arrive in pieces even without
        // streaming. Reading only the first would truncate the answer.
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"content":[{"type":"text","text":"the digit is "},
                           {"type":"text","text":"seven"}]}"#,
        )
        .expect("valid");
        assert_eq!(reply.text, "the digit is seven");
    }

    #[test]
    fn a_tool_use_block_carries_arguments_already_parsed() {
        // No second parse here, unlike the OpenAI family's stringified `arguments`.
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"content":[{"type":"tool_use","id":"t1","name":"get_weather",
                            "input":{"city":"Kitchener"}}]}"#,
        )
        .expect("valid");

        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "get_weather");
        assert_eq!(reply.tool_calls[0].arguments["city"], json!("Kitchener"));
    }

    #[test]
    fn text_and_tool_use_can_arrive_together() {
        // Models routinely narrate before calling. Both must survive: the text for
        // the vision probe, the call for the tool probe.
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"content":[{"type":"text","text":"Looking that up."},
                           {"type":"tool_use","name":"get_weather","input":{}}]}"#,
        )
        .expect("valid");

        assert_eq!(reply.text, "Looking that up.");
        assert_eq!(reply.tool_calls.len(), 1);
    }

    #[test]
    fn unknown_block_types_are_ignored_rather_than_fatal() {
        // Thinking blocks exist today and more will be added. A new block type must
        // not turn a working probe into a parse failure.
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"content":[{"type":"thinking","thinking":"hmm"},
                           {"type":"something_new","payload":1},
                           {"type":"text","text":"seven"}]}"#,
        )
        .expect("valid");
        assert_eq!(reply.text, "seven");
    }

    #[test]
    fn prose_about_a_tool_is_not_a_tool_call() {
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"content":[{"type":"text","text":"I would use get_weather here."}]}"#,
        )
        .expect("valid");
        assert!(reply.tool_calls.is_empty());
    }

    #[test]
    fn a_max_tokens_stop_reason_marks_the_reply_truncated() {
        // What the OpenAI family calls `finish_reason: "length"`. Different key,
        // different value, same meaning — and the reason the flag is set in each
        // implementation rather than inferred by a shared helper.
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"The dig"}]}"#,
        )
        .expect("valid");
        assert!(reply.truncated);
    }

    #[test]
    fn an_end_turn_stop_reason_is_not_truncated() {
        let reply = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"seven"}]}"#,
        )
        .expect("valid");
        assert!(!reply.truncated);
    }

    #[test]
    fn a_reply_with_no_content_array_is_malformed() {
        let error = parse_probe_reply(
            "https://api.anthropic.com/v1/messages",
            r#"{"type":"message","role":"assistant"}"#,
        )
        .expect_err("must not read as an empty success");
        assert!(matches!(error, LlmError::MalformedResponse { .. }));
    }
}
