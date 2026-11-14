//! `ade::subscribe` — add an address to the iii product-update list.
//!
//! The console's email prompt hands its address here and nowhere else. The
//! POST to `iii.dev/api/subscribe` happens in this worker, not the browser, so
//! there is one place to change the destination. After the list has taken the
//! address it is announced on `email:signup` for the engine's telemetry
//! worker, which writes it to this machine's person. That announcement is best
//! effort and never fails the signup. The address is never logged.
//!
//! Every answer from the list comes back as an [`Outcome`], not an error, so
//! the prompt can tell a typo it should show from a service it should wait out.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::CallRequest;
use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const SUBSCRIBE_ID: &str = "ade::subscribe";

/// Topic the engine's telemetry worker subscribes to.
const IDENTIFY_TOPIC: &str = "email:signup";
const PUBLISH_ID: &str = "iii::durable::publish";
const PUBLISH_TIMEOUT_MS: u64 = 10_000;

/// iii's own signup API. It sends the confirmation email itself.
const DEFAULT_SIGNUP_URL: &str = "https://iii.dev/api/subscribe";
/// The `source` the API knows this product by. Fixed: the API refuses any
/// source it has not opened to.
const SIGNUP_SOURCE: &str = "ade";
const SIGNUP_TIMEOUT: Duration = Duration::from_secs(10);

/// RFC 5321's ceiling for a whole address.
pub const MAX_EMAIL_LENGTH: usize = 254;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubscribeInput {
    pub email: String,
    /// Where in the console the address was captured, reported with the
    /// telemetry announcement. Defaults to `console`.
    #[serde(default)]
    pub source: Option<String>,
}

/// What the list made of the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A confirmation email went out, or the address was already on the list.
    Subscribed,
    /// The list refused the address itself. Worth showing; the person can fix it.
    InvalidEmail,
    /// The list could not take it now: not open to this source yet, rate
    /// limited, down, or unreachable. Nothing for the person to fix, so the
    /// prompt closes quietly and asks again on a later run.
    RetryLater,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeOutput {
    pub subscribed: bool,
    pub outcome: Outcome,
}

impl From<Outcome> for SubscribeOutput {
    fn from(outcome: Outcome) -> Self {
        Self {
            subscribed: outcome == Outcome::Subscribed,
            outcome,
        }
    }
}

/// One address, an `@`, and a dot after it. The list does the real validation;
/// this only stops an obvious typo, or a pasted paragraph, from becoming a
/// POST to the list.
pub fn is_emailish(value: &str) -> bool {
    let email = value.trim();
    if email.is_empty() || email.len() > MAX_EMAIL_LENGTH {
        return false;
    }
    if email.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain
            .split_once('.')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
}

/// The signup endpoint, from `III_CONSOLE_SIGNUP_URL` or the default.
pub fn signup_url() -> Result<String, String> {
    let url = std::env::var("III_CONSOLE_SIGNUP_URL").unwrap_or_else(|_| DEFAULT_SIGNUP_URL.into());
    check_signup_url(&url)?;
    Ok(url)
}

/// An override that would send the address over plain HTTP is refused, not
/// used. Plain HTTP to this machine is allowed, so a local stand-in for the
/// list can be tested against; nothing leaves the machine. The host is read
/// from the parsed URL, so userinfo (`http://localhost:80@elsewhere/`) cannot
/// pass for it.
fn check_signup_url(url: &str) -> Result<(), String> {
    let refused =
        || "III_CONSOLE_SIGNUP_URL must be an https URL, or http to this machine".to_string();
    let parsed = reqwest::Url::parse(url).map_err(|_| refused())?;
    match (parsed.scheme(), parsed.host_str()) {
        ("https", Some(_)) => Ok(()),
        ("http", Some("127.0.0.1" | "localhost" | "[::1]")) => Ok(()),
        _ => Err(refused()),
    }
}

/// What the list's answer means for the caller, from its status and the
/// `error` code in its body (when there is one).
pub fn signup_outcome(status: u16, error: Option<&str>) -> Outcome {
    match (status, error) {
        // Already subscribed is a 200 too, so there is nothing to tell apart.
        (200..=299, _) => Outcome::Subscribed,
        (400, Some("invalid_email")) => Outcome::InvalidEmail,
        (400, Some("invalid_source")) | (403, _) => {
            tracing::warn!(
                status,
                error = error.unwrap_or(""),
                "the signup API is not open to `{SIGNUP_SOURCE}` yet; asking again on a later run"
            );
            Outcome::RetryLater
        }
        (429, _) => {
            tracing::info!(
                status,
                "the signup API is rate limiting; asking again on a later run"
            );
            Outcome::RetryLater
        }
        _ => {
            tracing::warn!(
                status,
                error = error.unwrap_or(""),
                "the signup API answered unexpectedly; asking again on a later run"
            );
            Outcome::RetryLater
        }
    }
}

/// The `error` code out of a JSON body like `{ "error": "invalid_email" }`.
fn error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_owned)
}

async fn post_signup(email: &str) -> Outcome {
    let url = match signup_url() {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!("{error}");
            return Outcome::RetryLater;
        }
    };
    let client = match reqwest::Client::builder()
        // A redirect would carry the address to a host we never checked.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(SIGNUP_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(error = %error, "http client init failed");
            return Outcome::RetryLater;
        }
    };
    let response = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(json!({ "email": email, "source": SIGNUP_SOURCE }).to_string())
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // reqwest's message names the URL, never the body.
            tracing::warn!(
                error = %error,
                timeout = error.is_timeout(),
                "the signup API could not be reached; asking again on a later run"
            );
            return Outcome::RetryLater;
        }
    };
    let status = response.status().as_u16();
    // The body is a short JSON object; a read that fails just means no code.
    let body = response.text().await.unwrap_or_default();
    let error = error_code(&body);
    // One line per attempt, so a signup can be followed in the worker log.
    // The address itself is never logged.
    tracing::info!(
        url = %url,
        status,
        error = error.as_deref().unwrap_or(""),
        "signup API answered"
    );
    signup_outcome(status, error.as_deref())
}

async fn publish_identify(iii: &IIIClient, email: &str, source: &str) {
    let sent = iii
        .call(CallRequest {
            function_id: PUBLISH_ID.into(),
            payload: json!({ "topic": IDENTIFY_TOPIC, "data": { "email": email, "source": source } }),
            action: None,
            timeout_ms: Some(PUBLISH_TIMEOUT_MS),
        })
        .await;
    if let Err(error) = sent {
        // An engine with telemetry off drains this topic, and a project
        // without the `queue` worker never delivers it. Neither fails the
        // signup the operator asked for.
        tracing::debug!(error = %error, topic = IDENTIFY_TOPIC, "identify publish failed");
    }
}

pub fn register(iii: &Arc<IIIClient>) {
    let iii_for_fn = iii.clone();
    iii.register_function(
        SUBSCRIBE_ID,
        RegisterFunction::new_async(move |input: SubscribeInput| {
            let iii = iii_for_fn.clone();
            async move {
                let email = input.email.trim().to_string();
                if !is_emailish(&email) {
                    return Ok::<SubscribeOutput, Error>(Outcome::InvalidEmail.into());
                }
                let outcome = post_signup(&email).await;
                if outcome != Outcome::Subscribed {
                    return Ok(outcome.into());
                }
                // The list has the address; that is the signup. The
                // announcement runs off the response path so its own timeout
                // cannot push this call past the browser's deadline.
                let source = input.source.unwrap_or_else(|| "console".into());
                tokio::spawn(async move { publish_identify(&iii, &email, &source).await });
                Ok(Outcome::Subscribed.into())
            }
        })
        .description("Add an email address to the iii product-update list.")
        .metadata(json!({ "internal": true })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_plain_address() {
        assert!(is_emailish("yes@iii.dev"));
        assert!(is_emailish("  first.last+tag@sub.example.co.uk "));
    }

    #[test]
    fn rejects_what_is_not_one_address() {
        for value in [
            "",
            "   ",
            "no at sign",
            "@iii.dev",
            "user@host",
            "user@.dev",
            "user@iii.",
            "a@b.com c@d.org",
            "a@b@c.com",
        ] {
            assert!(!is_emailish(value), "{value:?} should be refused");
        }
        let long = format!("{}@iii.dev", "a".repeat(MAX_EMAIL_LENGTH));
        assert!(!is_emailish(&long));
    }

    #[test]
    fn a_2xx_answer_is_a_signup() {
        // Sent, and already subscribed, are both a 200.
        assert_eq!(signup_outcome(200, None), Outcome::Subscribed);
        assert_eq!(signup_outcome(204, None), Outcome::Subscribed);
    }

    #[test]
    fn only_invalid_email_is_shown_to_the_person() {
        assert_eq!(
            signup_outcome(400, Some("invalid_email")),
            Outcome::InvalidEmail
        );
    }

    #[test]
    fn everything_else_waits_for_a_later_run() {
        for (status, error) in [
            (400, Some("invalid_source")),
            (403, Some("forbidden")),
            (403, None),
            (429, Some("rate_limited")),
            (500, None),
            (503, Some("unavailable")),
            // A redirect is not followed and not a signup.
            (302, None),
            (400, None),
            (404, None),
        ] {
            assert_eq!(
                signup_outcome(status, error),
                Outcome::RetryLater,
                "{status} {error:?}"
            );
        }
    }

    #[test]
    fn only_https_or_this_machine_is_a_signup_url() {
        for url in [
            "https://iii.dev/api/subscribe",
            "http://127.0.0.1:8787/api/subscribe",
            "http://localhost:8787",
            "http://[::1]:8787/x",
        ] {
            assert_eq!(check_signup_url(url), Ok(()), "{url}");
        }
        for url in [
            "http://example.com/api/subscribe",
            "http://127.0.0.1.evil.example/x",
            "http://localhost.evil.example",
            // Userinfo, not the host: the address would go to evil.example.
            "http://localhost:80@evil.example/api/subscribe",
            "http://127.0.0.1@evil.example/",
            "ftp://127.0.0.1",
            "not a url",
        ] {
            assert!(check_signup_url(url).is_err(), "{url} should be refused");
        }
    }

    #[test]
    fn reads_the_error_code_from_the_body() {
        assert_eq!(
            error_code(r#"{"error":"invalid_email"}"#).as_deref(),
            Some("invalid_email")
        );
        assert_eq!(error_code(r#"{"ok":true}"#), None);
        assert_eq!(error_code("<html>bad gateway</html>"), None);
        assert_eq!(error_code(""), None);
    }

    #[test]
    fn output_names_the_outcome_in_snake_case() {
        let out = serde_json::to_value(SubscribeOutput::from(Outcome::RetryLater)).unwrap();
        assert_eq!(
            out,
            json!({ "subscribed": false, "outcome": "retry_later" })
        );
    }
}
