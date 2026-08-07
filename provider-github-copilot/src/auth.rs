//! GitHub OAuth token acquisition — the long-lived credential the Copilot
//! token exchange consumes. Resolution chain, first hit wins:
//!
//! 1. `GITHUB_COPILOT_TOKEN` env — a ready Copilot **bearer**, skipping the
//!    exchange entirely (tests, short-lived dev sessions).
//! 2. `GITHUB_COPILOT_OAUTH_TOKEN` env — a GitHub OAuth token (`gho_…`). An
//!    explicit override outranks whatever a past sign-in persisted.
//! 3. iii-state (`provider-github-copilot` / `oauth_token`) — written by the
//!    device-flow login surface ([`crate::login`]).
//! 4. Read-only import of an existing editor credential:
//!    `~/.config/github-copilot/apps.json` (VS Code / copilot.vim), then
//!    pi's `~/.pi/agent/auth.json`. Never written back — the editor owns
//!    its own file.
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::state;

/// A resolved long-lived credential for the exchange step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubCredential {
    /// Ready Copilot bearer — use as-is, no exchange.
    Bearer(String),
    /// GitHub OAuth token — exchange at `copilot_internal/v2/token`.
    Oauth(String),
}

/// `~/.config/github-copilot/apps.json`: `{ "<client>:<user>": { "oauth_token": "gho_…" } }`.
fn from_apps_json(root: &Value) -> Option<String> {
    root.as_object()?
        .values()
        .filter_map(|v| v.get("oauth_token").and_then(Value::as_str))
        .next()
        .map(str::to_string)
}

/// pi's `~/.pi/agent/auth.json`: `{ "github-copilot": { … } }` with the OAuth
/// token under one of the conventional keys.
fn from_pi_auth_json(root: &Value) -> Option<String> {
    let entry = root.get("github-copilot")?;
    for key in ["oauth_token", "access_token", "token", "refresh"] {
        if let Some(t) = entry.get(key).and_then(Value::as_str) {
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}

/// The editor-credential import chain (read-only, best effort).
/// `GITHUB_COPILOT_NO_LOCAL_IMPORT` (non-empty) opts out — for operators who
/// keep editor and harness logins separate, and for hermetic tests.
pub fn read_local_editor_credential() -> Option<String> {
    if std::env::var("GITHUB_COPILOT_NO_LOCAL_IMPORT").is_ok_and(|v| !v.is_empty()) {
        return None;
    }
    let home = home()?;
    for rel in [
        ".config/github-copilot/apps.json",
        ".config/github-copilot/hosts.json",
    ] {
        if let Some(root) = read_json(&home.join(rel)) {
            if let Some(t) = from_apps_json(&root) {
                return Some(t);
            }
        }
    }
    if let Some(root) = read_json(&home.join(".pi/agent/auth.json")) {
        if let Some(t) = from_pi_auth_json(&root) {
            return Some(t);
        }
    }
    None
}

pub async fn load_stored_oauth(iii: &IIIClient) -> Option<String> {
    let value = match iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": state::STATE_SCOPE, "key": state::OAUTH_TOKEN_KEY }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(value) => value,
        // A state outage is not the same as "never signed in" — say so, or
        // the fallback to env/editor credentials looks like a lost login.
        Err(e) => {
            eprintln!(
                "[provider-github-copilot] reading the stored GitHub credential failed ({e}); \
                 falling back to the env and editor-credential chain"
            );
            return None;
        }
    };
    value.as_str().filter(|s| !s.is_empty()).map(String::from)
}

pub async fn store_oauth(iii: &IIIClient, token: &str) -> Result<(), iii_sdk::errors::Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({
            "scope": state::STATE_SCOPE,
            "key": state::OAUTH_TOKEN_KEY,
            "value": Value::from(token)
        }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

/// Resolve the long-lived credential per the chain above. `None` = the
/// operator has never signed in anywhere this worker can see — the caller
/// turns that into a permanent "run provider::github-copilot::login" error.
pub async fn resolve_credential(iii: &IIIClient) -> Option<GithubCredential> {
    if let Ok(bearer) = std::env::var("GITHUB_COPILOT_TOKEN") {
        if !bearer.is_empty() {
            return Some(GithubCredential::Bearer(bearer));
        }
    }
    // Before the persisted token: an operator who sets this expects it to
    // take effect, not to be masked by an older sign-in.
    if let Ok(oauth) = std::env::var("GITHUB_COPILOT_OAUTH_TOKEN") {
        if !oauth.is_empty() {
            return Some(GithubCredential::Oauth(oauth));
        }
    }
    if let Some(oauth) = load_stored_oauth(iii).await {
        return Some(GithubCredential::Oauth(oauth));
    }
    read_local_editor_credential().map(GithubCredential::Oauth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apps_json_yields_first_oauth_token() {
        let root = json!({
            "github.com:Iv1.abc": { "oauth_token": "gho_one", "user": "u" }
        });
        assert_eq!(from_apps_json(&root).as_deref(), Some("gho_one"));
        assert_eq!(from_apps_json(&json!({})), None);
        assert_eq!(from_apps_json(&json!({ "x": { "user": "u" } })), None);
    }

    #[test]
    fn pi_auth_json_checks_conventional_keys() {
        let root = json!({ "github-copilot": { "oauth_token": "gho_pi" } });
        assert_eq!(from_pi_auth_json(&root).as_deref(), Some("gho_pi"));
        let root = json!({ "github-copilot": { "access_token": "gho_at" } });
        assert_eq!(from_pi_auth_json(&root).as_deref(), Some("gho_at"));
        assert_eq!(from_pi_auth_json(&json!({ "anthropic": {} })), None);
        assert_eq!(from_pi_auth_json(&json!({ "github-copilot": {} })), None);
    }
}
