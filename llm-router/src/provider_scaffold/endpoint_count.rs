//! Counting against an upstream that meters prompts for us.
//!
//! Several providers answer "how many tokens is this prompt" themselves —
//! Anthropic and llama.cpp through `POST …/v1/messages/count_tokens`, Moonshot
//! through `…/tokenizers/estimate-token-count`, xAI by tokenizing text through
//! `…/tokenize-text`. What they share is everything around the call: a bounded
//! timeout, a status check that keeps the upstream's own words, and a JSON
//! reply with the number somewhere in it. What differs is the body they take
//! and the field they put the number in, which stays with each provider next
//! to the wire mappers it already owns.

use iii_sdk::errors::Error;
use serde_json::Value;

/// The count came from the provider's own metering, not from a local
/// estimate. Distinct from a locally computed count so a reader can tell
/// which one they are looking at.
pub const ESTIMATOR_METERED: &str = "metered";

/// A count is bounded and non-streaming; a tight budget keeps this inside the
/// router's own count_tokens timeout, which in turn sits on the path that
/// finalizes a turn.
pub const COUNT_TOKENS_TIMEOUT_SECS: u64 = 20;

/// A route under the provider's API base, derived from the chat endpoint it
/// was configured with (`…/v1/chat/completions` + `tokenizers/estimate` →
/// `…/v1/tokenizers/estimate`). Deriving rather than hardcoding is what keeps
/// a proxy or gateway serving the counting route too; the suffix stripped is
/// the same one each provider's discovery strips for its models route.
///
/// Trimming a single trailing segment would be wrong here: `chat/completions`
/// is two, so a naive cut lands on `…/v1/chat/tokenizers/estimate`, which no
/// upstream serves.
pub fn base_route_url(api_url: &str, route: &str) -> String {
    let base = api_url
        .strip_suffix("/chat/completions")
        .unwrap_or_else(|| api_url.trim_end_matches('/'));
    format!("{base}/{route}")
}

/// POST a counting request and pull the number out of the reply.
///
/// `pick` is where each upstream's answer shape lives: the reply is handed
/// over whole rather than deserialized into a shared struct, because these
/// upstreams agree on nothing about where the number goes.
pub async fn post_count(
    http: &reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
    pick: impl Fn(&Value) -> Option<u64>,
) -> Result<u64, Error> {
    let mut request = http
        .post(url)
        .timeout(std::time::Duration::from_secs(COUNT_TOKENS_TIMEOUT_SECS));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        // The upstream's own words are worth more than anything this layer
        // could say about a counting failure, so they are carried through.
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(300).collect();
        return Err(Error::Handler(format!(
            "provider/upstream_status: {status}: {excerpt}"
        )));
    }
    let reply: Value = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;

    pick(&reply).ok_or_else(|| {
        let excerpt: String = reply.to_string().chars().take(300).collect();
        Error::Handler(format!(
            "provider/bad_response: counting reply carried no token count: {excerpt}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_hangs_off_the_api_base_not_the_chat_path() {
        // `chat/completions` is two segments. Cutting one would produce
        // `…/v1/chat/tokenizers/…`, which no upstream serves — the mistake
        // this test exists to keep out.
        assert_eq!(
            base_route_url(
                "https://api.moonshot.ai/v1/chat/completions",
                "tokenizers/estimate-token-count"
            ),
            "https://api.moonshot.ai/v1/tokenizers/estimate-token-count"
        );
        assert_eq!(
            base_route_url("https://api.x.ai/v1/chat/completions", "tokenize-text"),
            "https://api.x.ai/v1/tokenize-text"
        );
    }

    #[test]
    fn a_base_configured_directly_keeps_working() {
        assert_eq!(
            base_route_url("https://gateway.internal/v1", "tokenize-text"),
            "https://gateway.internal/v1/tokenize-text"
        );
        assert_eq!(
            base_route_url("https://gateway.internal/v1/", "tokenize-text"),
            "https://gateway.internal/v1/tokenize-text"
        );
    }
}
