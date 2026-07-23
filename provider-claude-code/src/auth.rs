//! OAuth token helpers. This provider is a *dumb consumer*: login and refresh
//! live in the (future) `oauth-claude-code` worker + `auth-credentials` vault.
//! The only token work here is (a) resolving a fresh credential for backend
//! calls, and (b) an optional one-time, READ-ONLY import of an existing
//! `~/.claude/.credentials.json` into the vault (never written back, so the
//! running `claude` CLI's rotating refresh token is never clobbered).
//!
//! Unlike Codex's `~/.codex/auth.json`, Claude Code's credential file stores an
//! opaque access token (not a JWT) with an explicit `expiresAt` in **epoch
//! milliseconds**, so there is no JWT decode here — expiry is read straight
//! from the file/credential (converted to seconds to match the vault shape).
//!
//! macOS stores these credentials in the Keychain rather than a file; that
//! path is out of scope here (the file fallback simply reports NotConfigured).
use crate::{router_client, PROVIDER_ID};
use iii_sdk::IIIClient;
use serde_json::{json, Value};

/// Refresh proactively when the token expires within this margin.
const EXPIRY_MARGIN_SECS: i64 = 60;

/// Function id the vault stores in the credential record and triggers on
/// refresh-when-expiring. Implemented out-of-band (oauth-claude-code); this
/// provider only *triggers* it when the token is near expiry.
pub const REFRESH_FN_ID: &str = "oauth::claude-code::refresh";

/// Token expiry (seconds since epoch) from the credential's `expires_at`.
/// Anthropic OAuth access tokens are opaque, so there is no JWT fallback.
fn credential_expires_at(cred: &Value) -> Option<i64> {
    cred.get("expires_at").and_then(Value::as_i64)
}

fn near_expiry(cred: &Value) -> bool {
    match credential_expires_at(cred) {
        Some(exp) => exp <= crate::now_ms() / 1000 + EXPIRY_MARGIN_SECS,
        None => false, // unknown expiry: let the vault decide, don't force
    }
}

/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json`.
fn claude_credentials_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".claude"))
        })?;
    Some(dir.join(".credentials.json"))
}

/// Build a vault credential Value from a `~/.claude/.credentials.json`
/// `claudeAiOauth` object. `expiresAt` is epoch milliseconds in the file; the
/// vault credential stores `expires_at` in seconds (matching the Codex shape).
fn credential_from_credentials_json(root: &Value) -> Option<Value> {
    let oauth = root.get("claudeAiOauth")?;
    let access_token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let mut cred = json!({
        "type": "oauth",
        "provider": PROVIDER_ID,
        "access_token": access_token,
        "refresh_fn": REFRESH_FN_ID,
    });
    if let Some(rt) = oauth.get("refreshToken").and_then(Value::as_str) {
        cred["refresh_token"] = json!(rt);
    }
    if let Some(exp_ms) = oauth.get("expiresAt").and_then(Value::as_i64) {
        cred["expires_at"] = json!(exp_ms / 1000);
    }
    let mut extra = serde_json::Map::new();
    if let Some(st) = oauth.get("subscriptionType").and_then(Value::as_str) {
        extra.insert("subscription_type".into(), json!(st));
    }
    if let Some(scopes) = oauth.get("scopes") {
        extra.insert("scopes".into(), scopes.clone());
    }
    if !extra.is_empty() {
        cred["provider_extra"] = Value::Object(extra);
    }
    Some(cred)
}

/// Read a credential straight from `~/.claude/.credentials.json` (DEV FALLBACK
/// for when no auth-credentials vault is running). Read-only — the `claude` CLI
/// owns the file's refresh; this provider never writes it. Returns None if the
/// file is absent/unreadable (e.g. macOS Keychain-only, or a sandboxed home),
/// not JSON, or missing the `claudeAiOauth` block.
pub fn read_claude_home_credential() -> Option<Value> {
    let path = claude_credentials_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&contents).ok()?;
    credential_from_credentials_json(&root)
}

/// Fetch a usable credential for either streaming or model discovery.
///
/// The vault is authoritative when present. A near-expiry vault token triggers
/// the vault-owned refresh. Local development falls back to the Claude Code
/// CLI's read-only `.credentials.json` when no vault credential is available.
pub async fn fetch_fresh_credential(iii: &IIIClient) -> Option<Value> {
    if let Some(cred) = router_client::get_token_if_available(iii, PROVIDER_ID)
        .await
        .ok()
        .flatten()
    {
        if near_expiry(&cred)
            && matches!(
                router_client::refresh_if_available(iii, PROVIDER_ID).await,
                Ok(true)
            )
        {
            return router_client::get_token_if_available(iii, PROVIDER_ID)
                .await
                .ok()
                .flatten()
                .or(Some(cred));
        }
        return Some(cred);
    }
    if let Some(cred) = read_claude_home_credential() {
        eprintln!(
            "[provider-claude-code] no auth-credentials vault — using local \
             ~/.claude/.credentials.json (dev fallback)"
        );
        return Some(cred);
    }
    None
}

/// One-time, best-effort READ-ONLY import of `~/.claude/.credentials.json` into
/// the vault — only when the vault has no credential yet (so a fresher, rotated
/// vault token is never downgraded). Never writes back to the file.
pub async fn import_claude_home_if_absent(iii: &IIIClient) {
    if !router_client::auth_get_token_available(iii).await {
        return;
    }
    // Don't clobber a credential the vault already holds.
    if matches!(
        router_client::get_token(iii, PROVIDER_ID).await,
        Ok(Some(_))
    ) {
        return;
    }
    let Some(path) = claude_credentials_path() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return; // no file / unreadable (macOS Keychain, sandboxed home) — silent, expected
    };
    let Ok(root) = serde_json::from_str::<Value>(&contents) else {
        eprintln!(
            "[provider-claude-code] {} is not valid JSON — skipping import",
            path.display()
        );
        return;
    };
    let Some(cred) = credential_from_credentials_json(&root) else {
        eprintln!(
            "[provider-claude-code] {} has no claudeAiOauth credential — skipping import",
            path.display()
        );
        return;
    };
    match router_client::set_token_if_available(iii, PROVIDER_ID, cred).await {
        Ok(true) => println!(
            "[provider-claude-code] imported Claude Code OAuth credential from {} into the vault",
            path.display()
        ),
        Ok(false) => {}
        Err(e) => eprintln!("[provider-claude-code] vault import failed ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_parsed_from_claude_ai_oauth_with_ms_expiry() {
        let root = json!({
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-xyz",
                "refreshToken": "sk-ant-ort01-abc",
                "expiresAt": 1_900_000_000_000i64, // epoch MILLISECONDS
                "scopes": ["user:inference"],
                "subscriptionType": "max"
            }
        });
        let cred = credential_from_credentials_json(&root).unwrap();
        assert_eq!(cred["type"], "oauth");
        assert_eq!(cred["provider"], PROVIDER_ID);
        assert_eq!(cred["access_token"], "sk-ant-oat01-xyz");
        assert_eq!(cred["refresh_token"], "sk-ant-ort01-abc");
        // ms → s
        assert_eq!(cred["expires_at"], 1_900_000_000i64);
        assert_eq!(cred["refresh_fn"], REFRESH_FN_ID);
        assert_eq!(cred["provider_extra"]["subscription_type"], "max");
    }

    #[test]
    fn missing_or_wrong_shape_yields_none() {
        assert!(credential_from_credentials_json(&json!({})).is_none());
        // no accessToken
        assert!(credential_from_credentials_json(
            &json!({ "claudeAiOauth": { "refreshToken": "r" } })
        )
        .is_none());
        // blank accessToken
        assert!(credential_from_credentials_json(
            &json!({ "claudeAiOauth": { "accessToken": "   " } })
        )
        .is_none());
    }

    #[test]
    fn near_expiry_uses_margin_in_seconds() {
        let past = json!({ "access_token": "a", "expires_at": 1 });
        assert!(near_expiry(&past));
        let far = json!({ "access_token": "a", "expires_at": crate::now_ms() / 1000 + 3600 });
        assert!(!near_expiry(&far));
        let unknown = json!({ "access_token": "no-exp" });
        assert!(!near_expiry(&unknown));
    }
}
