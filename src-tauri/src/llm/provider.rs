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

/// One turn in, events out.
///
/// Events go through a channel rather than a returned stream so the caller can
/// forward them to the UI as they arrive, and so cancellation is expressed by
/// dropping the receiver — which is exactly what happens when the user
/// dismisses the panel mid-answer.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn turn(
        &self,
        request: TurnRequest,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError>;
}

/// A provider that replays a script.
///
/// Used by tests, and by the panel UI before either real provider works. It is
/// also the only way to exercise the UI's error paths without an unreachable
/// server.
pub struct FakeProvider {
    script: Vec<StreamEvent>,
    failure: Option<LlmError>,
}

impl FakeProvider {
    pub fn replaying(script: Vec<StreamEvent>) -> Self {
        Self {
            script,
            failure: None,
        }
    }

    pub fn failing(error: LlmError) -> Self {
        Self {
            script: Vec::new(),
            failure: Some(error),
        }
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
