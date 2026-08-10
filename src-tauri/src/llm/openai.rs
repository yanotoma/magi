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

use base64::Engine as _;

use crate::llm::provider::{
    Image, LlmError, Message, ProbeReply, ProbeRequest, Provider, StopReason, StreamEvent,
    ToolCall, TurnRequest,
};
use crate::llm::sse::{SseFrame, SseParser};

const BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

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
        push_message(&mut messages, message);
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        // Optional here, required by Anthropic, so Magi always sends it.
        "max_tokens": request.max_tokens,
    });

    if !request.tools.is_empty() {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            }))
            .collect::<Vec<_>>());
        // "auto" rather than forcing the call. The model deciding *when* to look is the
        // whole design; forcing it would make every turn pay for an image.
        body["tool_choice"] = json!("auto");
    }

    body
}

/// Appends one neutral message to this family's `messages` array.
///
/// Appends rather than returns, because one neutral message can become **two** wire
/// messages. A tool result carrying a screenshot is the case: this family's
/// `role: "tool"` message takes text only, so the image cannot ride along inside it
/// and follows as a `user` message with an `image_url` part. Anthropic has no such
/// split — it puts the image inside the tool result — which is precisely the kind of
/// divergence the neutral types exist to absorb.
fn push_message(messages: &mut Vec<serde_json::Value>, message: &Message) {
    match message {
        Message::User { text } => {
            messages.push(json!({ "role": "user", "content": text }));
        }

        Message::Assistant { text, calls } if calls.is_empty() => {
            messages.push(json!({ "role": "assistant", "content": text }));
        }

        Message::Assistant { text, calls } => {
            messages.push(json!({
                "role": "assistant",
                // Null rather than an empty string when the model said nothing but
                // called a tool. Some endpoints in this family reject `""` alongside
                // `tool_calls`, and null is what OpenAI's own responses contain.
                "content": if text.is_empty() { serde_json::Value::Null } else { json!(text) },
                "tool_calls": calls
                    .iter()
                    .map(|call| json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            // A *string* of JSON, not JSON. This family nests the
                            // arguments as an encoded string on the way out as well as
                            // on the way in.
                            "arguments": call.arguments.to_string(),
                        }
                    }))
                    .collect::<Vec<_>>(),
            }));
        }

        Message::ToolResult {
            call_id,
            text,
            image,
        } => {
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": text,
            }));

            if let Some(image) = image {
                messages.push(json!({
                    "role": "user",
                    "content": [json!({
                        "type": "image_url",
                        "image_url": { "url": data_url(image) },
                    })],
                }));
            }
        }
    }
}

/// An image as this family wants it: a `data:` URL with base64 payload.
fn data_url(image: &Image) -> String {
    use base64::Engine;
    format!(
        "data:{};base64,{}",
        image.media_type,
        base64::engine::general_purpose::STANDARD.encode(&image.bytes)
    )
}

/// Everything one frame means, in order.
///
/// **The only function the stream loop calls.** It exists because the alternative did
/// not work: `parse_tool_calls` was written, tested directly, and never wired into the
/// loop, so every tool call was parsed perfectly in the test suite and dropped in
/// production. Four hundred tests passed and the feature was dead. One entry point makes
/// that particular mistake unavailable.
///
/// Tool fragments come after the text of the same frame, which is the order they occur in
/// — a chunk carrying both is a model that said something and then reached for a tool.
pub fn parse_events(frame: &SseFrame) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if let Some(event) = parse_frame(frame) {
        events.push(event);
    }
    events.extend(parse_tool_calls(frame));
    events
}

/// Tool-call fragments in one frame, if any.
///
/// Separate from [`parse_frame`] because the arities differ, not for tidiness: a frame
/// carries at most one piece of text and may carry fragments for **several** tool calls
/// at once — `delta.tool_calls` is an array and its entries can have different indices
/// in a single chunk, which is how this family streams parallel calls. Folding both into
/// one function would mean returning a `Vec` for the text case too, where it is always
/// zero or one.
pub fn parse_tool_calls(frame: &SseFrame) -> Vec<StreamEvent> {
    let data = frame.data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Vec::new();
    }

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else {
        return Vec::new();
    };

    let Some(calls) = parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("tool_calls"))
        .and_then(|t| t.as_array())
    else {
        return Vec::new();
    };

    let mut events = Vec::new();
    for call in calls {
        // Defaults to 0 rather than skipping. A single call with the index omitted is
        // the shape some OpenAI-compatible servers produce, and dropping it would lose
        // the call entirely.
        let index = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

        // `id` and `name` arrive on the first fragment and are `null` afterwards, which
        // is why the accumulator is told never to overwrite a known value with an empty
        // one. Read as empty strings here so the same call can carry both a start and
        // an argument fragment.
        let id = call.get("id").and_then(|i| i.as_str()).unwrap_or_default();
        let name = call
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_default();

        if !id.is_empty() || !name.is_empty() {
            events.push(StreamEvent::ToolStart {
                index,
                id: id.to_string(),
                name: name.to_string(),
            });
        }

        if let Some(arguments) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
            .filter(|a| !a.is_empty())
        {
            events.push(StreamEvent::ToolArguments {
                index,
                json: arguments.to_string(),
            });
        }
    }

    events
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

    // Reasoning models in this family put their working in `reasoning_content`
    // (the DeepSeek convention, adopted by MiMo and others) or `reasoning`.
    // Neither is in OpenAI's own spec, which is why both are checked.
    if let Some(reasoning) = choice.get("delta").and_then(|d| {
        d.get("reasoning_content")
            .or_else(|| d.get("reasoning"))
            .and_then(|r| r.as_str())
            .filter(|r| !r.is_empty())
    }) {
        return Some(StreamEvent::Thinking(reasoning.to_string()));
    }

    match choice.get("finish_reason").and_then(|r| r.as_str()) {
        Some("stop") => Some(StreamEvent::Done(StopReason::EndTurn)),
        Some("length") => Some(StreamEvent::Done(StopReason::MaxTokens)),
        // Not the end of the turn: the loop answers the call and asks again.
        Some("tool_calls") => Some(StreamEvent::Done(StopReason::ToolUse)),
        // Providers invent their own reasons. Folding an unknown one into
        // EndTurn would report a filtered or errored turn as a clean finish.
        Some(other) => Some(StreamEvent::Done(StopReason::Other(other.to_string()))),
        None => None,
    }
}

/// Maps a probe onto this family's request body.
///
/// Non-streaming, and it attaches whichever one thing is being tested. Kept
/// separate from [`build_request`] rather than growing that function optional
/// fields: a turn never sends tools or images in v1, and merging the two would put
/// four `if let Some` branches in the path that every real answer goes through.
pub fn build_probe(request: &ProbeRequest) -> serde_json::Value {
    let mut messages = Vec::with_capacity(2);

    if let Some(system) = &request.system {
        messages.push(json!({ "role": "system", "content": system }));
    }

    // With an image, `content` becomes an array of parts. Without one it stays a
    // bare string — some local servers reject the array form outright, and the
    // text-only probes must work against those or reachability would fail for a
    // server that is in fact reachable.
    let content = match &request.image {
        Some(image) => json!([
            { "type": "text", "text": request.prompt },
            {
                "type": "image_url",
                "image_url": {
                    // This family wants a data URL. Anthropic wants the bare
                    // payload with the media type in its own field.
                    "url": format!(
                        "data:{};base64,{}",
                        image.media_type,
                        BASE64.encode(&image.bytes)
                    )
                }
            }
        ]),
        None => json!(request.prompt),
    };

    messages.push(json!({ "role": "user", "content": content }));

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": false,
        "max_tokens": request.max_tokens,
    });

    if let Some(tool) = &request.tool {
        body["tools"] = json!([{
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            }
        }]);
        // "auto" rather than forcing the call. Forcing it would make every model
        // that can emit the syntax at all look reliable, and the probe exists to
        // find out whether the model *chooses* the tool when it needs it.
        body["tool_choice"] = json!("auto");
    }

    if let Some(schema) = &request.json_schema {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "probe",
                "strict": true,
                "schema": schema,
            }
        });
    }

    body
}

/// Reads a non-streaming completion.
///
/// Tool calls are parsed as JSON, never string-matched. That distinction is the
/// tool probe's entire purpose: a model that writes `I'll call get_weather(...)`
/// into its prose has failed, and substring matching would score it as a pass.
pub fn parse_probe_reply(url: &str, body: &str) -> Result<ProbeReply, LlmError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|e| LlmError::MalformedResponse {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let message = parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| LlmError::MalformedResponse {
            url: url.to_string(),
            reason: "no choices[0].message in the response".to_string(),
        })?;

    let text = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();

    // "length" means the token limit cut the answer off. Worth carrying, because a
    // truncated probe and a wrong answer are indistinguishable in the verdict — and
    // a reasoning model can spend the whole budget thinking before it writes
    // anything at all.
    let truncated = parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("finish_reason"))
        .and_then(|r| r.as_str())
        == Some("length");

    let tool_calls = message
        .get("tool_calls")
        .and_then(|c| c.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();

                    // Arguments arrive as a JSON *string*, so they need a second
                    // parse. A model that emits something unparseable here has
                    // produced a malformed call, which is a failure rather than a
                    // call with no arguments — so this yields nothing rather than
                    // an empty object.
                    let raw = function.get("arguments").and_then(|a| a.as_str())?;
                    let arguments = if raw.trim().is_empty() {
                        serde_json::Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str(raw).ok()?
                    };

                    Some(ToolCall {
                        // The probe only checks that a call is well formed and never
                        // answers one, so the id is read if present and not required.
                        id: call
                            .get("id")
                            .and_then(|id| id.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ProbeReply {
        text,
        tool_calls,
        truncated,
    })
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
                for event in parse_events(&frame) {
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
            for event in parse_events(&frame) {
                if events.send(event).await.is_err() {
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    async fn probe(&self, request: ProbeRequest) -> Result<ProbeReply, LlmError> {
        let url = chat_url(&self.base_url);
        let mut http = self.client.post(&url).json(&build_probe(&request));

        if let Some(key) = &self.api_key {
            http = http.bearer_auth(key);
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
    fn the_stream_entry_point_covers_tool_calls_as_well_as_text() {
        // The guard for the mistake that shipped: a tool parser written, tested, and
        // never called by the loop. Anything the loop reads must come through here.
        let events = parse_events(&frame(
            r#"{"choices":[{"delta":{"content":"Looking.","tool_calls":[
                {"index":0,"id":"call_1","function":{"name":"capture_screen","arguments":"{}"}}
            ]}}]}"#,
        ));

        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Token(_))),
            "text was lost: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::ToolStart { .. })),
            "the tool call was lost: {events:?}"
        );
    }

    #[test]
    fn a_streamed_tool_call_reassembles_from_the_documented_chunks() {
        // The fragments are OpenAI's own, from the function-calling guide: `id` and
        // `name` on the first only, `null` after, and arguments in pieces that are each
        // invalid JSON.
        let chunks = [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_Ddm","type":"function","function":{"name":"capture_screen","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":null,"function":{"arguments":"{\"","name":null}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"reason"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"the error"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]}}]}"#,
        ];

        let mut stream = crate::llm::toolstream::ToolCallStream::new();
        for chunk in chunks {
            for event in parse_tool_calls(&frame(chunk)) {
                match event {
                    StreamEvent::ToolStart { index, id, name } => stream.begin(index, &id, &name),
                    StreamEvent::ToolArguments { index, json } => {
                        stream.push_arguments(index, &json)
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }

        let calls = stream.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_Ddm");
        assert_eq!(calls[0].name, "capture_screen");
        assert_eq!(calls[0].arguments["reason"], "the error");
    }

    #[test]
    fn a_null_name_does_not_start_a_second_call() {
        // The later chunks carry `"name": null`. Read as an empty string, they must not
        // produce a start event that could overwrite the real name with nothing.
        let events = parse_tool_calls(&frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":null,"function":{"arguments":"x","name":null}}]}}]}"#,
        ));
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(matches!(events[0], StreamEvent::ToolArguments { .. }));
    }

    #[test]
    fn two_calls_in_one_chunk_are_kept_apart() {
        // This family streams parallel calls as several entries in one array, and their
        // indices are the only thing separating the two argument strings.
        let events = parse_tool_calls(&frame(
            r#"{"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"a","function":{"name":"capture_screen","arguments":"{}"}},
                {"index":1,"id":"b","function":{"name":"capture_screen","arguments":"{}"}}
            ]}}]}"#,
        ));
        let indices: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ToolStart { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(indices, [0, 1]);
    }

    #[test]
    fn a_tool_finish_reason_is_not_the_end_of_the_turn() {
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#
            )),
            Some(StreamEvent::Done(StopReason::ToolUse))
        );
    }

    #[test]
    fn frames_without_tool_calls_produce_nothing() {
        assert!(parse_tool_calls(&frame(r#"{"choices":[{"delta":{"content":"hi"}}]}"#)).is_empty());
        assert!(parse_tool_calls(&frame("[DONE]")).is_empty());
        assert!(parse_tool_calls(&frame("{not json")).is_empty());
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
            tools: vec![crate::llm::tools::capture_screen()],
        }
    }

    #[test]
    fn a_tool_definition_is_nested_under_a_function_key() {
        // This family wraps the schema; Anthropic does not. Getting it wrong is a 400
        // that reads as though the tool itself is malformed.
        let body = build_request(&with_messages(vec![Message::user("hi")]));
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "capture_screen");
        assert_eq!(tool["function"]["parameters"]["type"], "object");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn no_tools_means_no_tools_key_at_all() {
        // An empty array is not the same as absence for every endpoint in this family,
        // and a tier that must not see tools must not see the key either.
        let mut request = with_messages(vec![Message::user("hi")]);
        request.tools.clear();
        let body = build_request(&request);
        assert!(body.get("tools").is_none(), "{body}");
        assert!(body.get("tool_choice").is_none(), "{body}");
    }

    #[test]
    fn tool_call_arguments_go_out_as_an_encoded_string() {
        // The one shape most likely to be got wrong: this family nests the arguments as
        // a JSON *string* in both directions, while Anthropic sends an object.
        let body = build_request(&with_messages(vec![Message::Assistant {
            text: String::new(),
            calls: vec![a_call()],
        }]));
        let call = &body["messages"][0]["tool_calls"][0];
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["type"], "function");
        let arguments = call["function"]["arguments"]
            .as_str()
            .expect("arguments must be a string, not an object");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(arguments).expect("valid JSON inside"),
            serde_json::json!({ "reason": "read the error" })
        );
    }

    #[test]
    fn an_assistant_that_only_called_a_tool_sends_null_content() {
        // Some endpoints in this family reject `""` alongside `tool_calls`, and null is
        // what OpenAI's own responses contain.
        let body = build_request(&with_messages(vec![Message::Assistant {
            text: String::new(),
            calls: vec![a_call()],
        }]));
        assert!(body["messages"][0]["content"].is_null(), "{body}");
    }

    #[test]
    fn a_screenshot_follows_the_tool_result_as_a_user_message() {
        // The divergence that shapes the neutral type. This family's tool messages
        // support text only — "For tool messages, only type `text` is supported" — so an
        // image cannot ride inside the result and follows it instead. One neutral
        // message therefore becomes two wire messages.
        let body = build_request(&with_messages(vec![Message::ToolResult {
            call_id: "call_1".to_string(),
            text: "Screenshot captured.".to_string(),
            image: Some(a_screenshot()),
        }]));

        let messages = body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 2, "expected a result then an image: {body}");

        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[0]["tool_call_id"], "call_1");
        assert!(
            messages[0]["content"].is_string(),
            "a tool message may only carry text"
        );

        assert_eq!(messages[1]["role"], "user");
        let url = messages[1]["content"][0]["image_url"]["url"]
            .as_str()
            .expect("a data URL");
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert!(!url.contains('\n'), "the payload must not be wrapped");
    }

    #[test]
    fn a_tool_result_without_an_image_stays_one_message() {
        let body = build_request(&with_messages(vec![Message::ToolResult {
            call_id: "call_1".to_string(),
            text: "no screenshot".to_string(),
            image: None,
        }]));
        assert_eq!(body["messages"].as_array().expect("array").len(), 1);
    }
    use crate::llm::provider::{Message, TurnRequest};

    fn a_request() -> TurnRequest {
        TurnRequest {
            model: "qwen2.5".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
            tools: Vec::new(),
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
    fn reasoning_content_becomes_a_thinking_event_not_a_token() {
        // DeepSeek's convention, which MiMo and others follow. Emitting it as a
        // Token would print the model's working as though it were the answer.
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{"reasoning_content":"let me think"}}]}"#
            )),
            Some(StreamEvent::Thinking("let me think".into()))
        );
    }

    #[test]
    fn the_reasoning_field_is_also_accepted() {
        // Neither spelling is in OpenAI's own spec, so both are checked.
        assert_eq!(
            parse_frame(&frame(r#"{"choices":[{"delta":{"reasoning":"hmm"}}]}"#)),
            Some(StreamEvent::Thinking("hmm".into()))
        );
    }

    #[test]
    fn content_wins_over_reasoning_when_a_frame_carries_both() {
        // The answer must never be delayed behind reasoning that arrived with it.
        assert_eq!(
            parse_frame(&frame(
                r#"{"choices":[{"delta":{"reasoning_content":"hmm","content":"Hi"}}]}"#
            )),
            Some(StreamEvent::Token("Hi".into()))
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

/// Probe mapping and reply reading, kept in their own module because they answer a
/// different question from the streaming tests above: not "does a turn work" but
/// "is a capability claim believable".
#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::llm::provider::{Image, ToolSpec};

    fn probe() -> ProbeRequest {
        ProbeRequest::new("gpt-test", "what digit is shown?")
    }

    fn a_tool() -> ToolSpec {
        ToolSpec {
            name: "get_weather".into(),
            description: "Look up the weather".into(),
            parameters: json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"]
            }),
        }
    }

    #[test]
    fn a_text_probe_sends_content_as_a_bare_string() {
        // Not an array of parts. Several local servers reject the array form, and
        // the reachability probe has to work against those — otherwise a server
        // that is in fact reachable would be reported as unreachable.
        let body = build_probe(&probe());
        assert!(body["messages"][0]["content"].is_string());
        assert_eq!(body["stream"], json!(false));
        // The value, not a literal. Hardcoding 256 here is what made this test pin
        // the budget that was starving reasoning models of room to answer.
        assert_eq!(
            body["max_tokens"],
            json!(crate::llm::provider::PROBE_MAX_TOKENS)
        );
    }

    #[test]
    fn an_image_probe_uses_a_data_url() {
        let mut request = probe();
        request.image = Some(Image::png(vec![1, 2, 3]));
        let body = build_probe(&request);

        let parts = body["messages"][0]["content"]
            .as_array()
            .expect("content becomes an array once an image is attached");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], json!("text"));

        let url = parts[1]["image_url"]["url"].as_str().expect("a data url");
        assert!(
            url.starts_with("data:image/png;base64,"),
            "this family wants the media type inside the URL, got: {url}"
        );
        assert!(url.ends_with("AQID"), "base64 of [1,2,3], got: {url}");
    }

    #[test]
    fn a_tool_probe_wraps_the_schema_in_a_function() {
        let mut request = probe();
        request.tool = Some(a_tool());
        let body = build_probe(&request);

        assert_eq!(body["tools"][0]["type"], json!("function"));
        assert_eq!(body["tools"][0]["function"]["name"], json!("get_weather"));
        // `parameters`, not `input_schema`. Anthropic uses the other name, and
        // sending the wrong one gets the tool silently ignored rather than rejected.
        assert!(body["tools"][0]["function"]["parameters"].is_object());
        assert!(body["tools"][0]["function"]["input_schema"].is_null());
    }

    #[test]
    fn the_tool_probe_does_not_force_the_call() {
        // Forcing it would make any model that can emit the syntax at all look
        // reliable. The probe is about whether the model *chooses* the tool.
        let mut request = probe();
        request.tool = Some(a_tool());
        assert_eq!(build_probe(&request)["tool_choice"], json!("auto"));
    }

    #[test]
    fn a_schema_probe_uses_response_format() {
        let mut request = probe();
        request.json_schema = Some(json!({ "type": "object" }));
        let body = build_probe(&request);

        assert_eq!(body["response_format"]["type"], json!("json_schema"));
        assert_eq!(
            body["response_format"]["json_schema"]["strict"],
            json!(true)
        );
    }

    #[test]
    fn reads_plain_text_out_of_a_reply() {
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"message":{"role":"assistant","content":"seven"}}]}"#,
        )
        .expect("valid");
        assert_eq!(reply.text, "seven");
        assert!(reply.tool_calls.is_empty());
    }

    #[test]
    fn parses_a_tool_call_with_stringified_arguments() {
        // This family sends `arguments` as a JSON string inside JSON, so it needs a
        // second parse.
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"message":{"content":null,"tool_calls":[
                 {"id":"c1","type":"function","function":{
                   "name":"get_weather","arguments":"{\"city\":\"Kitchener\"}"}}]}}]}"#,
        )
        .expect("valid");

        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "get_weather");
        assert_eq!(reply.tool_calls[0].arguments["city"], json!("Kitchener"));
    }

    #[test]
    fn prose_that_talks_about_calling_a_tool_is_not_a_tool_call() {
        // The assertion the whole tool probe rests on. Small local models routinely
        // narrate the call instead of making it, and a substring check for the
        // tool's name would score this as a pass — handing the model tier 1 and an
        // agentic loop it cannot drive.
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"message":{"content":
                 "I will call get_weather({\"city\": \"Kitchener\"}) now."}}]}"#,
        )
        .expect("valid");

        assert!(reply.text.contains("get_weather"));
        assert!(
            reply.tool_calls.is_empty(),
            "talking about a call is not making one"
        );
    }

    #[test]
    fn a_malformed_arguments_string_is_not_a_valid_call() {
        // A model that emits unparseable arguments has produced a broken call, not
        // a call with no arguments. Treating it as the latter would pass a model
        // whose calls cannot actually be executed.
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"message":{"tool_calls":[
                 {"function":{"name":"get_weather","arguments":"{city: Kitchener"}}]}}]}"#,
        )
        .expect("the response itself is valid JSON");

        assert!(
            reply.tool_calls.is_empty(),
            "unparseable arguments must not count as a call"
        );
    }

    #[test]
    fn empty_arguments_are_a_valid_call_with_no_input() {
        // A tool that takes nothing is legitimate, and some models send "" rather
        // than "{}".
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"message":{"tool_calls":[
                 {"function":{"name":"ping","arguments":""}}]}}]}"#,
        )
        .expect("valid");

        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].arguments, json!({}));
    }

    #[test]
    fn a_length_finish_reason_marks_the_reply_truncated() {
        // The signal that separates "the model got it wrong" from "the answer was
        // cut off". A reasoning model can spend the whole budget thinking and
        // return empty content, which every verdict reads as a failure.
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"finish_reason":"length","message":{"content":""}}]}"#,
        )
        .expect("valid");
        assert!(reply.truncated);
        assert!(reply.text.is_empty());
    }

    #[test]
    fn a_normal_finish_is_not_truncated() {
        let reply = parse_probe_reply(
            "http://x/v1/chat/completions",
            r#"{"choices":[{"finish_reason":"stop","message":{"content":"seven"}}]}"#,
        )
        .expect("valid");
        assert!(!reply.truncated);
    }

    #[test]
    fn a_reply_with_no_choices_is_malformed_rather_than_empty() {
        // Reporting this as an empty answer would record "the model said nothing",
        // which reads as a model that failed the probe. It is Magi failing to
        // understand the response, and the two need different messages.
        let error = parse_probe_reply("http://x/v1/chat/completions", r#"{"object":"list"}"#)
            .expect_err("must not be read as an empty success");
        assert!(matches!(error, LlmError::MalformedResponse { .. }));
    }
}
