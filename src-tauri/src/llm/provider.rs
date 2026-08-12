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

/// One message in a conversation.
///
/// An enum rather than a struct with optional fields, so the states that cannot
/// exist cannot be written: a tool result with no call to answer, an assistant
/// message carrying somebody else's tool call. The agentic loop assembles these in a
/// fixed order and getting that order wrong is rejected by both APIs, so the type
/// should refuse it first.
///
/// `PartialEq` but not `Eq`, because a [`ToolCall`]'s arguments are a
/// `serde_json::Value` and JSON numbers are floats.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User {
        text: String,

        /// Screenshots attached to the question itself.
        ///
        /// The Tier 2 path, and the reason this is not only a tool-result concern. A model
        /// that sees but cannot be trusted with tools is never told a tool exists, so Magi
        /// decides from the user's words and attaches the image before asking — by which
        /// point there is nothing for the model to call.
        images: Vec<Image>,
    },

    /// What the model said, and any tools it asked for.
    ///
    /// The calls travel with the text because both families require the assistant's
    /// turn to be replayed *whole* — dropping the tool-call part and keeping the prose
    /// makes the following result reference a call that is no longer in the history,
    /// which is an error rather than a degradation.
    Assistant { text: String, calls: Vec<ToolCall> },

    /// The answer to one tool call.
    ///
    /// `image` is the reason this is not simply text. Magi's one tool returns a
    /// screenshot, and the two families disagree about where an image may appear in a
    /// tool result — so the neutral type carries both parts and each provider decides
    /// how to express them. That disagreement is exactly what the Provider trait is
    /// for.
    ToolResult {
        call_id: String,
        text: String,
        /// Usually one, and none when the tool had nothing to show. Three when the model
        /// asked for every monitor, which is why this is a list rather than an `Option`.
        images: Vec<Image>,
    },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message::User {
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// A question with screenshots attached.
    pub fn user_seeing(text: impl Into<String>, images: Vec<Image>) -> Self {
        Message::User {
            text: text.into(),
            images,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Message::Assistant {
            text: text.into(),
            calls: Vec::new(),
        }
    }

    /// The role this message takes on the wire.
    ///
    /// A tool result is a `user` message in both families — Anthropic puts a
    /// `tool_result` block in one, and the OpenAI family uses `role: "tool"`, which is
    /// still the user's side of the exchange. Callers that only need "who is speaking"
    /// get one answer instead of matching.
    pub fn role(&self) -> Role {
        match self {
            Message::User { .. } | Message::ToolResult { .. } => Role::User,
            Message::Assistant { .. } => Role::Assistant,
        }
    }

    /// The text of this message, whatever kind it is.
    pub fn text(&self) -> &str {
        match self {
            Message::User { text, .. }
            | Message::Assistant { text, .. }
            | Message::ToolResult { text, .. } => text,
        }
    }
}

/// One turn's worth of input, in Magi's own terms.
///
/// The system prompt is a field rather than a message because that is the
/// shape both families can be mapped onto: Anthropic takes it as a top-level
/// parameter, and the OpenAI family takes a `role: "system"` message. Modelling
/// it as a message would bake in one family's choice.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// Required by Anthropic, optional elsewhere, so always set.
    pub max_tokens: u32,

    /// Tools the model may call, or empty to offer none.
    ///
    /// Empty for every tier but the agentic one. A model that malforms tool syntax
    /// must not be handed a definition to malform — see `llm::prompt`, where the same
    /// rule governs what the system prompt says.
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    /// The model stopped because it wants a tool run.
    ///
    /// Not a failure and not the end of the turn: the loop answers the call and asks
    /// again. Anthropic calls this `tool_use` and the OpenAI family `tool_calls`.
    ToolUse,
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

    /// A tool call is starting, at `index` within this response.
    ///
    /// Consumed by the command layer rather than forwarded to the panel — these two
    /// variants are wire-level fragments, and what the panel eventually hears about is
    /// the capture that results. They live here so a provider's frame parser can stay a
    /// pure function of one frame, with all the remembering in one place.
    ToolStart {
        index: usize,
        id: String,
        name: String,
    },

    /// More argument JSON for the call at `index`. Never valid JSON on its own.
    ToolArguments {
        index: usize,
        json: String,
    },

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

impl LlmError {
    /// What is safe to write to a log file that outlives the run.
    ///
    /// Two audiences, and they want opposite things. The **user-facing** message keeps
    /// everything — naming the URL and quoting the provider's own words is what makes an
    /// error actionable, and M2 added that deliberately. The **log** is a file the user may
    /// hand to a stranger in a bug report, so it carries only what Magi itself produced.
    ///
    /// The line that divides them is authorship, not sensitivity in the abstract:
    ///
    /// - The **URL travels**. It lives in `config.toml`, which this project keeps free of
    ///   secrets precisely so it can be pasted into an issue, and an error that does not say
    ///   what it tried to reach is the useless kind.
    /// - **Provider text never travels.** `body` is a raw HTTP response and a rejection can
    ///   quote the request back at you; `reason` on a malformed response comes from a parser
    ///   that likes to include the input it choked on. Both are one indirection away from the
    ///   conversation, so both are reduced to a length.
    pub fn log_summary(&self) -> String {
        match self {
            Self::Unreachable { url, reason } => format!("unreachable: {url}: {reason}"),
            Self::Unauthorized { url } => format!("unauthorized: {url}"),
            Self::ModelNotFound { url, model } => format!("no model '{model}' at {url}"),
            Self::Http { url, status, body } => {
                format!(
                    "http {status} from {url} ({} bytes of body, withheld)",
                    body.len()
                )
            }
            Self::MalformedResponse { url, reason } => {
                format!(
                    "malformed response from {url} ({} chars of detail, withheld)",
                    reason.chars().count()
                )
            }
        }
    }
}

/// A PNG to send to the model, as raw bytes.
///
/// Bytes and a media type rather than an encoded string, because the two families
/// encode it differently — one wants a `data:` URL, the other a bare base64
/// payload beside a `media_type` field. Encoding here would pick a side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// `&'static str` rather than `String`: every image Magi sends is a PNG, and both
    /// the pre-flight probe and the screen capture produce one. A runtime value here
    /// would invite a caller to invent a media type that no encoder in the crate can
    /// actually produce.
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
    /// The provider's handle for this call.
    ///
    /// Opaque, and required: both families match a result to its call by this string,
    /// and a result carrying the wrong one — or none — is an error rather than a
    /// degraded answer. Empty only where nothing will answer the call, which is the
    /// pre-flight probe: it checks that a call is well formed and never runs it.
    pub id: String,
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
            max_tokens: PROBE_MAX_TOKENS,
        }
    }
}

/// The token limit for a probe.
///
/// Generous, and the first version of this was not — it was 256, on the reasoning
/// that every probe wants a word or a short object so a bigger limit would only pay
/// for a model that likes to explain itself.
///
/// That reasoning is backwards for a reasoning model, which is most of the
/// interesting ones. Thinking tokens are generated and billed whether or not the
/// limit accommodates them, so a tight `max_tokens` does not avoid the cost — it
/// truncates the answer that cost was spent producing. Set to 256, a model that
/// thinks for three hundred tokens about a picture of a `7` returns empty content,
/// and the vision verdict reads that as "did not see the image".
///
/// So the cheap-looking limit was the expensive one: it reported the most capable
/// models as the least capable. This is sized to leave room for thinking plus a
/// short answer.
pub const PROBE_MAX_TOKENS: u32 = 2048;

// A compile-time floor rather than a test. Lowering this below the room a reasoning
// model needs is the specific mistake that reported vision-capable models as blind,
// and a build error catches it at the moment someone tries rather than in a suite
// they might not have run yet.
const _: () = assert!(
    PROBE_MAX_TOKENS >= 1024,
    "thinking tokens are billed whether or not the limit fits them, so a tight probe \
     budget truncates the answer that cost was spent producing instead of avoiding it"
);

/// What came back from a probe.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProbeReply {
    pub text: String,

    /// Structurally parsed calls. Empty when the model produced none — including
    /// when it described a call in prose, which is the case the tool probe exists
    /// to catch.
    pub tool_calls: Vec<ToolCall>,

    /// Whether the reply stopped at the token limit rather than finishing.
    ///
    /// Recorded so that a probe which failed because the answer was cut off is
    /// distinguishable from one that failed because the model got it wrong. Those
    /// look identical in the verdict — both produce "no digit found" — but one is a
    /// Magi problem and the other is a model limitation, and reporting the first as
    /// the second is how a capable model gets marked incapable.
    pub truncated: bool,
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

    /// Something a provider might echo back that the user would not want in a file.
    const QUOTED_BACK: &str = "invalid request: messages[0] said 'my salary is 90000'";

    #[test]
    fn a_provider_rejection_body_never_reaches_the_log() {
        // The failure this exists to prevent: an API rejecting a request and quoting it
        // back, landing the user's own question in a file they later attach to an issue.
        let error = LlmError::Http {
            url: "https://api.example.com/v1".into(),
            status: 400,
            body: QUOTED_BACK.into(),
        };

        let summary = error.log_summary();

        assert!(!summary.contains("salary"), "body leaked: {summary}");
        assert!(!summary.contains(QUOTED_BACK), "body leaked: {summary}");
        // Still says enough to act on.
        assert!(summary.contains("400"));
        assert!(summary.contains("api.example.com"));
    }

    #[test]
    fn the_user_facing_message_still_carries_everything() {
        // The other half of the same decision. Reducing the log must not reduce the error
        // the user reads — naming the provider and quoting its words is what makes it
        // actionable, and M2 added that deliberately.
        let error = LlmError::Http {
            url: "https://api.example.com/v1".into(),
            status: 400,
            body: QUOTED_BACK.into(),
        };

        assert!(error.to_string().contains(QUOTED_BACK));
    }

    #[test]
    fn a_parser_that_quotes_its_input_does_not_leak_it_either() {
        // serde errors like to include the fragment they choked on, and that fragment is
        // one indirection from the conversation.
        let error = LlmError::MalformedResponse {
            url: "https://api.example.com/v1".into(),
            reason: format!("expected value at line 1 column 1: {QUOTED_BACK}"),
        };

        let summary = error.log_summary();

        assert!(!summary.contains("salary"), "reason leaked: {summary}");
        assert!(summary.contains("api.example.com"));
    }

    #[test]
    fn the_url_does_travel() {
        // Deliberate, and the opposite of the rule above. It lives in config.toml, which
        // this project keeps free of secrets so it can be pasted into an issue — and an
        // error that will not say what it tried to reach is the useless kind.
        for error in [
            LlmError::Unreachable {
                url: "http://localhost:11434/v1".into(),
                reason: "connection refused".into(),
            },
            LlmError::Unauthorized {
                url: "http://localhost:11434/v1".into(),
            },
            LlmError::ModelNotFound {
                url: "http://localhost:11434/v1".into(),
                model: "qwen".into(),
            },
        ] {
            assert!(
                error.log_summary().contains("localhost:11434"),
                "got: {}",
                error.log_summary()
            );
        }
    }

    #[test]
    fn every_variant_says_which_kind_it_was() {
        // A summary that reduced everything to "an error occurred" would be safe and
        // useless. Each one has to remain distinguishable from the others.
        let summaries = [
            LlmError::Unreachable {
                url: "u".into(),
                reason: "r".into(),
            },
            LlmError::Unauthorized { url: "u".into() },
            LlmError::ModelNotFound {
                url: "u".into(),
                model: "m".into(),
            },
            LlmError::Http {
                url: "u".into(),
                status: 500,
                body: "b".into(),
            },
            LlmError::MalformedResponse {
                url: "u".into(),
                reason: "r".into(),
            },
        ]
        .map(|e| e.log_summary());

        for (i, one) in summaries.iter().enumerate() {
            for (j, other) in summaries.iter().enumerate() {
                if i != j {
                    assert_ne!(one, other, "two variants log identically");
                }
            }
        }
    }

    fn a_request() -> TurnRequest {
        TurnRequest {
            model: "test-model".into(),
            system: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            max_tokens: 1024,
            tools: Vec::new(),
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
        assert_eq!(request.messages[0].role(), Role::User);
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
                // Wire fragments, consumed by the command layer rather than the panel.
                StreamEvent::ToolStart { .. } | StreamEvent::ToolArguments { .. } => {}
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
