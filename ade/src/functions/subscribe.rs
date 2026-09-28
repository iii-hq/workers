//! `console::subscribe` — add an address to the iii product-update list.
//!
//! The console's email prompt hands its address here and nowhere else. The
//! POST to the list happens in this worker, not the browser: the list checks
//! an `Origin` allowlist the console's own origin is not on, and one place to
//! change the destination is enough. After the list has taken the address it
//! is announced on `email:signup` for the engine's telemetry worker, which
//! writes it to this machine's person. That announcement is best effort and
//! never fails the signup. The address is never logged.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const SUBSCRIBE_ID: &str = "console::subscribe";

/// Topic the engine's telemetry worker subscribes to.
const IDENTIFY_TOPIC: &str = "email:signup";
const PUBLISH_ID: &str = "iii::durable::publish";
const PUBLISH_TIMEOUT_MS: u64 = 10_000;

/// The same public signup form the iii.dev landing page posts to (its URL is
/// a meta tag in that page's head, so it is not a secret).
const DEFAULT_SIGNUP_URL: &str =
    "https://api.mailmodo.com/api/v1/at/f/y0trGR0lfL/c6aefeeb-e66a-5c8a-9c71-4733d9ea1836";
/// Mailmodo matches `Origin` against an allowlist and this literal is on it.
/// `localhost` and `[::1]` are refused even though they name the same host.
const DEFAULT_SIGNUP_ORIGIN: &str = "http://127.0.0.1";
const SIGNUP_TIMEOUT: Duration = Duration::from_secs(10);

/// RFC 5321's ceiling for a whole address.
pub const MAX_EMAIL_LENGTH: usize = 254;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubscribeInput {
    pub email: String,
    /// Where the address was captured, reported with it. Defaults to `console`.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeOutput {
    pub subscribed: bool,
}

/// One address, an `@`, and a dot after it. The list does the real validation;
/// this only stops an obvious typo, or a pasted paragraph, from becoming a
/// POST to someone else's service.
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

/// The signup endpoint, from `III_CONSOLE_SIGNUP_URL` or the default. An
/// override that downgrades the address to plain HTTP is refused, not used.
pub fn signup_url() -> Result<String, String> {
    let url = std::env::var("III_CONSOLE_SIGNUP_URL").unwrap_or_else(|_| DEFAULT_SIGNUP_URL.into());
    if !url.starts_with("https://") {
        return Err("III_CONSOLE_SIGNUP_URL must be an https URL".into());
    }
    Ok(url)
}

/// What the list's answer means for the caller.
pub fn signup_outcome(status: u16) -> Result<(), String> {
    // Mailmodo answers 200 "added/updated" for an address already on the
    // list, so there is nothing to tell the caller apart from success.
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("the signup service answered {status}"))
    }
}

async fn post_signup(email: &str) -> Result<(), String> {
    let url = signup_url()?;
    let origin =
        std::env::var("III_CONSOLE_SIGNUP_ORIGIN").unwrap_or_else(|_| DEFAULT_SIGNUP_ORIGIN.into());
    let client = reqwest::Client::builder()
        // A redirect would carry the address to a host we never checked.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(SIGNUP_TIMEOUT)
        .build()
        .map_err(|error| format!("http client init failed: {error}"))?;
    // `data` carries the form fields and must be present; each key must match
    // a contact property in Mailmodo, so `email` is the only one sent.
    let response = client
        .post(&url)
        .header(reqwest::header::ORIGIN, origin)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(json!({ "email": email, "data": { "email": email } }).to_string())
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "the signup service did not answer in time".to_string()
            } else {
                // reqwest's message names the URL, never the body.
                format!("the signup service could not be reached: {error}")
            }
        })?;
    signup_outcome(response.status().as_u16())
}

async fn publish_identify(iii: &IIIClient, email: &str, source: &str) {
    let sent = iii
        .trigger(TriggerRequest {
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
                    return Err(Error::Handler(format!(
                        "that does not look like an email address (one address, up to {MAX_EMAIL_LENGTH} characters)"
                    )));
                }
                post_signup(&email).await.map_err(Error::Handler)?;
                let source = input.source.as_deref().unwrap_or("console");
                publish_identify(&iii, &email, source).await;
                Ok::<_, Error>(SubscribeOutput { subscribed: true })
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
    fn only_a_2xx_answer_is_a_signup() {
        assert_eq!(signup_outcome(200), Ok(()));
        assert_eq!(signup_outcome(204), Ok(()));
        assert_eq!(
            signup_outcome(400),
            Err("the signup service answered 400".into())
        );
        assert_eq!(
            signup_outcome(302),
            Err("the signup service answered 302".into())
        );
    }
}
