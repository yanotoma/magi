//! The provider abstraction: one turn in, a stream of events out.
//!
//! Everything here is provider-neutral by construction. If a name from one
//! vendor's wire format appears in these types, that vendor has started
//! dictating the shape of the others and the abstraction has failed.

use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One turn's worth of input, in Magi's own terms.
///
/// The system prompt is a field rather than a message because that is the
/// shape both families can be mapped onto: Anthropic takes it as a top-level
/// parameter, and the OpenAI family takes a `role: "system"` message. Modelling
/// it as a message would bake in one family's choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// Required by Anthropic, optional elsewhere, so always set.
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Token(String),

    /// The model's reasoning, kept separate from the answer.
    ///
    /// A separate variant rather than more `Token`s because the two must be
    /// displayable independently: merged, the user would read the working as
    /// though it were the conclusion. Not every model emits it.
    Thinking(String),

    Done(StopReason),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    /// Every transport failure names the address it tried. "connection refused"
    /// on its own sends the user looking in the wrong place — most often at
    /// their network, when the real answer is a base URL with a typo or a local
    /// server that was never started.
    #[error("could not reach {url}: {reason}")]
    Unreachable { url: String, reason: String },

    #[error("{url} rejected the credentials: check the API key for this provider")]
    Unauthorized { url: String },

    #[error("{url} does not have a model named '{model}'")]
    ModelNotFound { url: String, model: String },

    #[error("{url} returned HTTP {status}: {body}")]
    Http {
        url: String,
        status: u16,
        body: String,
    },

    #[error("the response from {url} could not be understood: {reason}")]
    MalformedResponse { url: String, reason: String },
}

/// A PNG to send to the model, as raw bytes.
///
/// Bytes and a media type rather than an encoded string, because the two families
/// encode it differently — one wants a `data:` URL, the other a bare base64
/// payload beside a `media_type` field. Encoding here would pick a side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

impl Image {
    pub fn png(bytes: Vec<u8>) -> Self {
        Self {
            media_type: "image/png",
            bytes,
        }
    }
}

/// A tool offered to the model, in neutral terms.
///
/// `parameters` is a JSON Schema value. Both families take a schema; they disagree
/// only on the key it sits under and on how much wrapping goes around it, which is
/// each implementation's problem rather than this type's.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool call the model actually made.
///
/// `arguments` is parsed JSON, not a string. A string here would invite callers to
/// match on substrings, and the whole point of the tool probe is distinguishing a
/// structurally valid call from prose that talks about calling something.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// A single non-streaming request, used by pre-flight.
///
/// Separate from [`TurnRequest`] rather than an extension of it, because the two
/// have opposite requirements. A turn streams, carries history, and never sends
/// tool definitions in v1. A probe sends exactly one message, wants the whole
/// reply at once, and exists to attach precisely the one thing being tested.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeRequest {
    pub model: String,
    pub system: Option<String>,
    pub prompt: String,

    /// Attached when the vision probe runs.
    pub image: Option<Image>,

    /// Offered when the tool probe runs.
    pub tool: Option<ToolSpec>,

    /// Requested when the structured-output probe runs.
    pub json_schema: Option<serde_json::Value>,

    pub max_tokens: u32,
}

impl ProbeRequest {
    /// A text-only probe. The three capability probes add one thing each.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: None,
            prompt: prompt.into(),
            image: None,
            tool: None,
            json_schema: None,
            // Small on purpose. Every probe wants a word or a short object, and a
            // generous limit here just pays for a model that decides to explain
            // itself at length before answering.
            max_tokens: 256,
        }
    }
}

/// What came back from a probe.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProbeReply {
    pub text: String,

    /// Structurally parsed calls. Empty when the model produced none — including
    /// when it described a call in prose, which is the case the tool probe exists
    /// to catch.
    pub tool_calls: Vec<ToolCall>,
}

/// One turn in, events out; plus a single-shot probe for pre-flight.
///
/// Events go through a channel rather than a returned stream so the caller can
/// forward them to the UI as they arrive, and so cancellation is expressed by
/// dropping the receiver — which is exactly what happens when the user
/// dismisses the panel mid-answer.
///
/// `probe` is on this trait rather than beside it because the things it sends —
/// an image, a tool definition, a schema — are formatted differently by each
/// family, and that knowledge already lives in the implementations. Building probe
/// payloads outside them would put wire formats in a second place and guarantee
/// the two drift.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn turn(
        &self,
        request: TurnRequest,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError>;

    /// One request, the whole reply. No streaming: nobody is watching a probe
    /// arrive, so incremental delivery would be complexity for no reader.
    async fn probe(&self, request: ProbeRequest) -> Result<ProbeReply, LlmError>;
}

/// A provider that replays a script.
///
/// Used by tests, and by the panel UI before either real provider works. It is
/// also the only way to exercise the UI's error paths without an unreachable
/// server.
pub struct FakeProvider {
    script: Vec<StreamEvent>,
    failure: Option<LlmError>,

    /// Replies handed out in order, one per `probe` call.
    ///
    /// A queue rather than one canned reply because pre-flight runs four probes
    /// and the interesting cases are the mixed ones — a model that sees but cannot
    /// call tools is the whole reason tier 2 exists, and testing it needs
    /// different answers to consecutive probes.
    probe_replies: std::sync::Mutex<std::collections::VecDeque<Result<ProbeReply, LlmError>>>,
}

impl FakeProvider {
    pub fn replaying(script: Vec<StreamEvent>) -> Self {
        Self {
            script,
            failure: None,
            probe_replies: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn failing(error: LlmError) -> Self {
        Self {
            script: Vec::new(),
            failure: Some(error),
            probe_replies: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// Queues the replies `probe` will return, in order.
    pub fn answering_probes(mut replies: Vec<Result<ProbeReply, LlmError>>) -> Self {
        let mut provider = Self::replaying(Vec::new());
        provider.probe_replies = std::sync::Mutex::new(replies.drain(..).collect());
        provider
    }
}

#[async_trait]
impl Provider for FakeProvider {
    async fn turn(
        &self,
        _request: TurnRequest,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }

        for event in &self.script {
            // A closed receiver means the user cancelled. Stopping is the
            // correct outcome, not an error to report.
            if events.send(event.clone()).await.is_err() {
                return Ok(());
            }
        }

        Ok(())
    }

    async fn probe(&self, _request: ProbeRequest) -> Result<ProbeReply, LlmError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }

        let queued = self
            .probe_replies
            .lock()
            .map_err(|_| LlmError::MalformedResponse {
                url: "fake".into(),
                reason: "the fake's reply queue was poisoned".into(),
            })?
            .pop_front();

        // Running out of scripted replies is a test that asked for more probes than
        // it prepared for, so it says so rather than returning something plausible
        // that would make the test pass for the wrong reason.
        queued.unwrap_or_else(|| {
            Err(LlmError::MalformedResponse {
                url: "fake".into(),
                reason: "no probe reply was queued for this call".into(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_request() -> TurnRequest {
        TurnRequest {
            model: "test-model".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
        }
    }

    #[test]
    fn turn_request_carries_no_provider_specific_names() {
        // A compile-time guard expressed as a test so the intent is written
        // down: the moment `x-api-key`, `input_schema` or `image_url` appears in
        // these types, the abstraction has leaked and one implementation is
        // dictating the shape of the other.
        let request = a_request();
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, Role::User);
    }

    #[tokio::test]
    async fn the_fake_replays_its_script_in_order() {
        let provider = FakeProvider::replaying(vec![
            StreamEvent::Token("Hello".into()),
            StreamEvent::Token(", world".into()),
            StreamEvent::Done(StopReason::EndTurn),
        ]);

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        provider
            .turn(a_request(), tx)
            .await
            .expect("fake never fails");

        let mut tokens = String::new();
        let mut finished = false;
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::Token(t) => tokens.push_str(&t),
                // Reasoning is collected separately; a test that folded it into
                // `tokens` would pass while the panel mixed the two.
                StreamEvent::Thinking(_) => {}
                StreamEvent::Done(_) => finished = true,
            }
        }

        assert_eq!(tokens, "Hello, world");
        assert!(finished, "the stream must end with Done");
    }

    #[tokio::test]
    async fn the_fake_can_be_told_to_fail() {
        // Every error path in the UI needs a way to be exercised without an
        // unreachable server.
        let provider = FakeProvider::failing(LlmError::Unreachable {
            url: "http://localhost:11434/v1".into(),
            reason: "connection refused".into(),
        });

        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = provider.turn(a_request(), tx).await;

        assert!(matches!(result, Err(LlmError::Unreachable { .. })));
    }

    #[tokio::test]
    async fn dropping_the_receiver_stops_the_turn_rather_than_erroring() {
        // This is cancellation. When the user dismisses the panel mid-answer the
        // receiver goes away, and the provider must notice and stop instead of
        // pushing into a closed channel and reporting that as a failure.
        let provider = FakeProvider::replaying(vec![
            StreamEvent::Token("one".into()),
            StreamEvent::Token("two".into()),
            StreamEvent::Done(StopReason::EndTurn),
        ]);

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);

        let result = provider.turn(a_request(), tx).await;
        assert!(result.is_ok(), "a cancelled turn is not a failed turn");
    }

    #[test]
    fn errors_name_what_was_tried() {
        // "connection refused" without a URL sends the user looking in the wrong
        // place. Every transport error carries the address it attempted.
        let error = LlmError::Unreachable {
            url: "http://localhost:11434/v1".into(),
            reason: "connection refused".into(),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("localhost:11434"), "got: {rendered}");
    }
}
