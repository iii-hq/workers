//! Client-credentials tokens: the form a token request sends, one POST to
//! the token endpoint, and the single-flight holder of the current token.

use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use crate::server::{ErrorCode, ServerError};
use crate::translate::scrub;

/// The form body of a client-credentials token request; `scope` as written.
pub fn token_request_body(client_id: &str, client_secret: &str, scope: Option<&str>) -> String {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "client_credentials")
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret);
    if let Some(scope) = scope {
        form.append_pair("scope", scope);
    }
    form.finish()
}

/// The client of every token request: never follows a redirect, so the
/// client secret goes to the configured origin only, and keeps no idle
/// connection.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .build()
        .expect("a reqwest client needs the rustls provider installed at start")
}

/// One token request. A refusal names the endpoint's `error_description`,
/// else its `error`, else the status: never the token, the body, the client
/// secret or a reqwest error, whose text carries the URL.
pub async fn mint(
    http: &reqwest::Client,
    token_url: &str,
    body: String,
) -> Result<String, ServerError> {
    // Scrubbed before the cut, which could leave part of it unmatched.
    let secret: Vec<String> = url::form_urlencoded::parse(body.as_bytes())
        .filter(|(k, _)| k == "client_secret")
        .map(|(_, v)| v.into_owned())
        .collect();
    let unreachable =
        || ServerError::new(ErrorCode::ConnectFailed, "token endpoint did not answer");
    let rejected = |detail: &str| {
        let message = format!("token request rejected: {detail}");
        ServerError::new(ErrorCode::CredentialRejected, message)
    };
    let response = http
        .post(token_url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .timeout(Duration::from_secs(30))
        .body(body)
        .send()
        .await
        .map_err(|_| unreachable())?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| unreachable())?;
    let answer: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    if !status.is_success() {
        let detail = match answer["error_description"]
            .as_str()
            .or(answer["error"].as_str())
        {
            Some(words) => scrub(words, &secret).chars().take(200).collect(),
            None => format!("HTTP {}", status.as_u16()),
        };
        return Err(rejected(&detail));
    }
    match answer["access_token"].as_str() {
        Some(token) if !token.is_empty() => Ok(token.into()),
        _ => Err(rejected("the answer holds no access_token")),
    }
}

/// The current token of one server. Concurrent callers share one POST: the
/// lock is held across it. No `Debug`: it holds the client secret.
pub struct Minter {
    http: reqwest::Client,
    token_url: String,
    body: String,
    token: tokio::sync::Mutex<Option<String>>,
}

impl Minter {
    pub fn new(http: reqwest::Client, token_url: String, body: String) -> Minter {
        Minter {
            http,
            token_url,
            body,
            token: tokio::sync::Mutex::new(None),
        }
    }

    /// The token held, else a new one.
    pub async fn token(&self) -> Result<String, ServerError> {
        let mut held = self.token.lock().await;
        if let Some(token) = &*held {
            return Ok(token.clone());
        }
        let token = mint(&self.http, &self.token_url, self.body.clone()).await?;
        *held = Some(token.clone());
        Ok(token)
    }

    /// Drops the token held: the next `token` mints.
    pub async fn invalidate(&self) {
        *self.token.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn an_echoed_client_secret_is_scrubbed_before_the_cut() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let secret = "secret-across-the-cut-0123456789";
        let words = format!("{}{secret} tail", "x".repeat(190));
        let reply = serde_json::json!({"error": "invalid_client", "error_description": words});
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(reply))
            .mount(&server)
            .await;
        let body = token_request_body("app", secret, None);
        let err = mint(&http_client(), &server.uri(), body).await.unwrap_err();
        for start in 0..=secret.len() - 6 {
            let piece = &secret[start..start + 6];
            assert!(!err.message.contains(piece), "{piece} in {}", err.message);
        }
        assert!(err.message.ends_with("[redacted]"), "{}", err.message);
    }

    #[test]
    fn token_body_is_form_encoded_and_keeps_the_scope_verbatim() {
        assert_eq!(
            token_request_body("app id", "s&cret=", Some("read,write")),
            "grant_type=client_credentials&client_id=app+id&client_secret=s%26cret%3D&scope=read%2Cwrite"
        );
        assert_eq!(
            token_request_body("a", "b", None),
            "grant_type=client_credentials&client_id=a&client_secret=b"
        );
    }
}
