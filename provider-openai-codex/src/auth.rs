//! OAuth token helpers. This provider is a *dumb consumer*: login and refresh
//! live in the `oauth-openai-codex` worker + `auth-credentials` vault. The only
//! token work here is (a) decoding the unsigned JWT payload for the ChatGPT
//! account id / expiry, and (b) an optional one-time, READ-ONLY import of an
//! existing `~/.codex/auth.json` into the vault (never written back, so the
//! running `codex` CLI's rotating refresh token is never clobbered).
use crate::{router_client, PROVIDER_ID};
use base64::Engine as _;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

/// The ChatGPT account-id claim namespace in Codex OAuth JWTs.
const AUTH_CLAIM: &str = "https://api.openai.com/auth";

/// Function id the vault stores in the credential record and triggers on
/// refresh-when-expiring.
pub const REFRESH_FN_ID: &str = "oauth::openai-codex::refresh";

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let seg = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(seg.as_bytes())
        .or_else(|_| {
            let pad = (4 - seg.len() % 4) % 4;
            let padded = format!("{seg}{}", "=".repeat(pad));
            base64::engine::general_purpose::URL_SAFE.decode(padded.as_bytes())
        })
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// ChatGPT account id from a Codex OAuth access/id token (unsigned decode).
pub fn account_id_from_access_token(token: &str) -> Option<String> {
    let v = decode_jwt_payload(token)?;
    if let Some(a) = v
        .get(AUTH_CLAIM)
        .and_then(|c| c.get("chatgpt_account_id"))
        .and_then(Value::as_str)
    {
        return Some(a.to_string());
    }
    v.get("chatgpt_account_id")
        .or_else(|| v.get("account_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Access-token expiry (`exp`, seconds since epoch) from the JWT payload.
pub fn expires_at_from_access_token(token: &str) -> Option<i64> {
    decode_jwt_payload(token)?
        .get("exp")
        .and_then(Value::as_i64)
}

/// `${CODEX_HOME:-$HOME/.codex}/auth.json`.
fn codex_auth_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("CODEX_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".codex"))
        })?;
    Some(home.join("auth.json"))
}

/// Build a vault credential Value from a `~/.codex/auth.json` `tokens` object.
fn credential_from_auth_json(root: &Value) -> Option<Value> {
    if root.get("auth_mode").and_then(Value::as_str) != Some("chatgpt") {
        return None;
    }
    let tokens = root.get("tokens")?;
    let access_token = tokens.get("access_token").and_then(Value::as_str)?;
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| account_id_from_access_token(access_token))?;
    let mut cred = json!({
        "type": "oauth",
        "provider": PROVIDER_ID,
        "access_token": access_token,
        "provider_extra": { "account_id": account_id },
        "refresh_fn": REFRESH_FN_ID,
    });
    if let Some(rt) = tokens.get("refresh_token").and_then(Value::as_str) {
        cred["refresh_token"] = json!(rt);
    }
    if let Some(idt) = tokens.get("id_token").and_then(Value::as_str) {
        cred["id_token"] = json!(idt);
    }
    if let Some(exp) = expires_at_from_access_token(access_token) {
        cred["expires_at"] = json!(exp);
    }
    Some(cred)
}

/// Read a credential straight from `~/.codex/auth.json` (DEV FALLBACK for when
/// no auth-credentials vault is running). Read-only — the `codex` CLI owns the
/// file's refresh; this provider never writes it. Returns None if the file is
/// absent/unreadable (e.g. a sandboxed home), not JSON, or not a ChatGPT login.
pub fn read_codex_home_credential() -> Option<Value> {
    let path = codex_auth_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&contents).ok()?;
    credential_from_auth_json(&root)
}

/// One-time, best-effort READ-ONLY import of `~/.codex/auth.json` into the
/// vault — only when the vault has no credential yet (so a fresher, rotated
/// vault token is never downgraded). Never writes back to auth.json.
pub async fn import_codex_home_if_absent(iii: &IIIClient) {
    // Don't clobber a credential the vault already holds.
    if matches!(
        router_client::get_token(iii, PROVIDER_ID).await,
        Ok(Some(_))
    ) {
        return;
    }
    let Some(path) = codex_auth_path() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return; // no file / unreadable (e.g. sandboxed home) — silent, expected
    };
    let Ok(root) = serde_json::from_str::<Value>(&contents) else {
        eprintln!(
            "[provider-openai-codex] {} is not valid JSON — skipping import",
            path.display()
        );
        return;
    };
    let Some(cred) = credential_from_auth_json(&root) else {
        eprintln!(
            "[provider-openai-codex] {} is not a ChatGPT login (auth_mode != chatgpt) — skipping import",
            path.display()
        );
        return;
    };
    match router_client::set_token(iii, PROVIDER_ID, cred).await {
        Ok(()) => println!(
            "[provider-openai-codex] imported ChatGPT credential from {} into the vault",
            path.display()
        ),
        Err(e) => eprintln!("[provider-openai-codex] vault import failed ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn jwt_with(payload: Value) -> String {
        let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("h.{b}.s")
    }

    #[test]
    fn account_id_from_nested_and_flat_claims() {
        let nested =
            jwt_with(json!({ "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1" } }));
        assert_eq!(
            account_id_from_access_token(&nested).as_deref(),
            Some("acc-1")
        );
        let flat = jwt_with(json!({ "chatgpt_account_id": "acc-2" }));
        assert_eq!(
            account_id_from_access_token(&flat).as_deref(),
            Some("acc-2")
        );
        assert_eq!(account_id_from_access_token("not-a-jwt"), None);
    }

    #[test]
    fn exp_is_read_in_seconds() {
        let t = jwt_with(json!({ "exp": 1_900_000_000i64 }));
        assert_eq!(expires_at_from_access_token(&t), Some(1_900_000_000));
    }

    #[test]
    fn credential_requires_chatgpt_mode() {
        let apikey = json!({ "auth_mode": "apikey", "OPENAI_API_KEY": "sk" });
        assert!(credential_from_auth_json(&apikey).is_none());
        let chatgpt = json!({
            "auth_mode": "chatgpt",
            "tokens": { "access_token": "a.b.c", "account_id": "acc", "refresh_token": "rt" }
        });
        let cred = credential_from_auth_json(&chatgpt).unwrap();
        assert_eq!(cred["type"], "oauth");
        assert_eq!(cred["provider_extra"]["account_id"], "acc");
        assert_eq!(cred["refresh_fn"], REFRESH_FN_ID);
    }
}
