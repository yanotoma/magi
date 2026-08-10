//! Turning configuration into a usable provider.

use crate::config::{ProviderConfig, ProviderKind};
use crate::llm::anthropic::Anthropic;
use crate::llm::openai::OpenAiCompatible;
use crate::llm::provider::Provider;

/// Builds the implementation that speaks this provider's protocol.
///
/// The match is on `kind`, not on the vendor, and that is the whole point:
/// Xiaomi serves both protocols on one host at different paths, so a vendor-keyed
/// registry would have to pick one of them. Keyed by protocol, the same vendor is
/// simply registered twice.
pub fn build(
    client: reqwest::Client,
    provider: &ProviderConfig,
    api_key: Option<String>,
) -> Box<dyn Provider> {
    match provider.kind {
        ProviderKind::OpenaiCompatible => Box::new(OpenAiCompatible::new(
            client,
            provider.base_url.clone(),
            api_key,
        )),
        ProviderKind::Anthropic => {
            Box::new(Anthropic::new(client, provider.base_url.clone(), api_key))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{Message, StreamEvent, TurnRequest};

    fn config(kind: ProviderKind) -> ProviderConfig {
        ProviderConfig {
            id: "test".into(),
            kind,
            base_url: "http://localhost:9/v1".into(),
            models: vec!["m".into()],
            requires_key: false,
        }
    }

    #[test]
    fn each_protocol_resolves_to_its_own_implementation() {
        // Both arms must be constructible. Without this the second one is dead
        // code until someone configures it, which is the worst time to find out.
        let client = reqwest::Client::new();
        let _openai = build(
            client.clone(),
            &config(ProviderKind::OpenaiCompatible),
            None,
        );
        let _anthropic = build(client, &config(ProviderKind::Anthropic), None);
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_the_url_it_tried() {
        // Port 9 is the discard port: reliably nothing listening, no network
        // needed, no external host contacted.
        let provider = build(
            reqwest::Client::new(),
            &config(ProviderKind::OpenaiCompatible),
            None,
        );

        let (tx, _rx) = tokio::sync::mpsc::channel::<StreamEvent>(4);
        let error = provider
            .turn(
                TurnRequest {
                    model: "m".into(),
                    system: None,
                    messages: vec![Message::user("hi")],
                    max_tokens: 16,
                    tools: Vec::new(),
                },
                tx,
            )
            .await
            .expect_err("nothing is listening on port 9");

        assert!(
            error.to_string().contains("localhost:9"),
            "the message must name what it tried: {error}"
        );
    }
}
