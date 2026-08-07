//! Asking an endpoint which models it serves.
//!
//! Split into a pure parser and a thin fetch, so the interesting half is tested
//! without a network. Both protocol families expose the list at `/models` with
//! ids under `data[].id`, which is why one parser covers both — the divergence
//! between them is in the chat call, not here.

use crate::config::{ProviderConfig, ProviderKind};

/// Joins the base URL and `models` without producing a double slash.
///
/// Users paste base URLs with and without a trailing slash in roughly equal
/// measure, and `//models` 404s on several servers.
pub fn models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

/// Extracts model ids from a `/models` response body.
///
/// Sorted and de-duplicated: endpoints promise no order, and a list that
/// reshuffles between runs looks like something changed when nothing did.
pub fn parse_model_list(body: &str) -> Result<Vec<String>, String> {
    let parsed: serde_json::Value = serde_json::from_str(body).map_err(|_| {
        "the endpoint did not return a model list (the response was not JSON)".to_string()
    })?;

    let entries = parsed
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "the endpoint did not return a model list (no `data` array)".to_string())?;

    let mut ids: Vec<String> = entries
        .iter()
        // A single malformed entry should not cost the user the other forty.
        .filter_map(|entry| entry.get("id")?.as_str().map(str::to_string))
        .collect();

    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Fetches the model list from a provider.
///
/// The only part of this module that touches the network, and deliberately the
/// only part with no test.
pub async fn discover_models(
    client: &reqwest::Client,
    provider: &ProviderConfig,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let url = models_url(&provider.base_url);
    let mut request = client.get(&url);

    // The auth header is where the two families diverge, even for this call.
    if let Some(key) = api_key {
        request = match provider.kind {
            ProviderKind::OpenaiCompatible => request.bearer_auth(key),
            ProviderKind::Anthropic => request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
        };
    }

    let response = request
        .send()
        .await
        // Every transport error names the address it tried: "connection refused"
        // alone sends people to look at their network when the cause is almost
        // always a typo in the URL or a local server that was never started.
        .map_err(|e| format!("could not reach {url}: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("could not read the response from {url}: {e}"))?;

    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => format!("{url} rejected the credentials: check this provider's API key"),
            404 => {
                format!("{url} returned 404 — is the base URL right? It should usually end in /v1")
            }
            _ => format!(
                "{url} returned HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            ),
        });
    }

    parse_model_list(&body).map_err(|e| format!("{url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_openai_list_shape() {
        let body = r#"{
            "object": "list",
            "data": [
                {"id": "qwen2.5-vl:7b", "object": "model"},
                {"id": "llama3.2", "object": "model"}
            ]
        }"#;
        // Sorted, not in server order — see the sorting test below for why.
        assert_eq!(
            parse_model_list(body).unwrap(),
            ["llama3.2", "qwen2.5-vl:7b"]
        );
    }

    #[test]
    fn reads_the_anthropic_list_shape() {
        // Same `data[].id` path, extra fields alongside. Both families are
        // covered by one parser, which is why this is not two functions.
        let body = r#"{
            "data": [
                {"type": "model", "id": "claude-opus-5", "display_name": "Claude Opus 5"}
            ],
            "has_more": false
        }"#;
        assert_eq!(parse_model_list(body).unwrap(), ["claude-opus-5"]);
    }

    #[test]
    fn results_are_sorted_so_the_list_does_not_reshuffle() {
        // Endpoints do not promise an order. Re-running discovery and seeing the
        // list rearrange makes it look like something changed when nothing did.
        let body = r#"{"data": [{"id": "zeta"}, {"id": "alpha"}, {"id": "mid"}]}"#;
        assert_eq!(parse_model_list(body).unwrap(), ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn entries_without_an_id_are_skipped_rather_than_failing_the_batch() {
        // One malformed entry should not cost the user the other forty.
        let body = r#"{"data": [{"id": "good"}, {"object": "model"}, {"id": "also-good"}]}"#;
        assert_eq!(parse_model_list(body).unwrap(), ["also-good", "good"]);
    }

    #[test]
    fn duplicates_are_collapsed() {
        let body = r#"{"data": [{"id": "same"}, {"id": "same"}]}"#;
        assert_eq!(parse_model_list(body).unwrap(), ["same"]);
    }

    #[test]
    fn an_empty_list_is_a_valid_answer_not_an_error() {
        // A freshly installed Ollama with nothing pulled answers exactly this.
        // Reporting it as a failure would send the user looking for a broken
        // endpoint when the real message is "pull a model first".
        assert_eq!(
            parse_model_list(r#"{"data": []}"#).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_body_that_is_not_a_model_list_is_reported_clearly() {
        // Pointing a provider at a web page rather than an API is a common
        // mistake, and "expected value at line 1" would not explain it.
        let error = parse_model_list("<html><body>Not found</body></html>").unwrap_err();
        assert!(
            error.contains("did not return a model list"),
            "got: {error}"
        );
    }

    #[test]
    fn the_models_url_is_built_without_doubling_the_slash() {
        assert_eq!(
            models_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1/models"
        );
        assert_eq!(
            models_url("http://localhost:11434/v1/"),
            "http://localhost:11434/v1/models"
        );
    }
}
