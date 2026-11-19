//! `auth::token`: hands the current Codex access token to the workers named in
//! `TOKEN_CONSUMERS`. Kept apart from `login.rs`, whose responses are all
//! token-free. Never returns the refresh or id token.
use crate::config::extract_access_token;
use crate::session::{AuthManager, CredentialSource, SessionError};
use crate::surface;
use iii_sdk::{CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Worker names allowed to read the access token.
const TOKEN_CONSUMERS: &[&str] = &["judge-openai"];
const WORKERS_LIST: &str = "engine::workers::list";
const LOOKUP_TIMEOUT_MS: u64 = 5_000;
/// The engine's namespace for workers that declare none.
const DEFAULT_NAMESPACE: &str = "default";

#[derive(Deserialize, JsonSchema)]
pub struct TokenRequest {
    /// The access token an upstream call just rejected with HTTP 401. When it
    /// is still the current ChatGPT sign-in token, the session is refreshed
    /// once. Ignored for vault and Codex CLI credentials.
    #[serde(default)]
    pub rejected_token: Option<String>,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TokenResponse {
    /// The OAuth access token for `Authorization: Bearer`; null when signed out.
    pub access_token: Option<String>,
}

pub async fn token(
    auth: &Arc<AuthManager>,
    caller: Option<&str>,
    rejected: Option<&str>,
) -> Result<TokenResponse, SessionError> {
    if !caller.is_some_and(|caller| TOKEN_CONSUMERS.contains(&caller)) {
        return Err(SessionError::new(
            "forbidden",
            "Only judge-openai in this namespace may read the Codex access token.",
        ));
    }
    let mut resolved = auth.resolve(None).await?;
    if let (Some(rejected), Some(current)) = (rejected, resolved.as_ref()) {
        // Vault and CLI credentials cannot be refreshed here; resolve(Some)
        // would mark them rejected for the provider's own streams too.
        if current.source == CredentialSource::Managed
            && extract_access_token(&current.value).as_deref() == Some(rejected)
        {
            resolved = auth.resolve(Some(rejected)).await?;
        }
    }
    Ok(TokenResponse {
        access_token: resolved.and_then(|r| extract_access_token(&r.value)),
    })
}

/// The rule of `secrets/src/access.rs` (`parse_workers` + `authorized_name`),
/// uncached: the worker name behind `id` when it lives in `own_namespace`; an
/// engine that reports no namespace is trusted as-is.
fn caller_in(list: &Value, id: &str, own_namespace: &str) -> Option<String> {
    let worker = list
        .get("workers")?
        .as_array()?
        .iter()
        .find(|w| w.get("id").and_then(Value::as_str) == Some(id))?;
    match worker.get("namespace").and_then(Value::as_str) {
        Some(namespace) if namespace != own_namespace => None,
        _ => Some(worker.get("name")?.as_str()?.trim().to_owned()),
    }
}

/// The engine stamps `_caller_worker_id` on every call, so a caller cannot
/// claim another worker's identity.
async fn caller_name(iii: &IIIClient, id: Option<&str>) -> Result<Option<String>, SessionError> {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let list = iii
        .call(CallRequest {
            function_id: WORKERS_LIST.to_owned(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(LOOKUP_TIMEOUT_MS),
        })
        .await
        .map_err(|_| {
            SessionError::new(
                "caller_lookup_failed",
                "Could not identify the calling worker. Retry the operation.",
            )
        })?;
    let own_namespace = iii
        .namespace()
        .filter(|namespace| !namespace.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NAMESPACE.to_owned());
    Ok(caller_in(&list, id, &own_namespace))
}

pub fn register(iii: &IIIClient, auth: Arc<AuthManager>) {
    let client = iii.clone();
    iii.register_function(
        surface::AUTH_TOKEN_ID,
        RegisterFunction::new_async(move |req: TokenRequest| {
            let (iii, auth) = (client.clone(), auth.clone());
            async move {
                let caller = caller_name(&iii, req.caller_worker_id.as_deref())
                    .await
                    .map_err(SessionError::into_bus)?;
                token(&auth, caller.as_deref(), req.rejected_token.as_deref())
                    .await
                    .map_err(SessionError::into_bus)
            }
        })
        .description(surface::AUTH_TOKEN_DESC)
        .metadata(json!({"internal": true})),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::SessionRecord;
    use crate::session::tests::{credential, setup};
    use iii_sdk::errors::Error;
    use std::sync::atomic::Ordering;

    const JUDGE: Option<&str> = Some("judge-openai");

    fn bus_code(e: SessionError) -> String {
        match e.into_bus() {
            Error::Remote { code, .. } => code,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn judge_gets_only_the_access_token() {
        let (manager, store, _, _) = setup();
        *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
        let response = token(&manager, JUDGE, None).await.unwrap();
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            json!({"access_token": "secret-account"})
        );
    }

    #[tokio::test]
    async fn other_callers_are_forbidden_before_any_resolve() {
        let (manager, _, oauth, legacy) = setup();
        for caller in [None, Some(""), Some("llm-router"), Some("judge-openai-x")] {
            let err = token(&manager, caller, None).await.err().unwrap();
            assert_eq!(bus_code(err), "provider/forbidden");
        }
        assert_eq!(legacy.reads.load(Ordering::SeqCst), 0);
        assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn caller_must_be_a_listed_worker_in_this_namespace() {
        let list = json!({"workers": [
            {"id": "a", "name": "judge-openai", "namespace": "default"},
            {"id": "b", "name": "judge-openai", "namespace": "other-project"},
            {"id": "c", "name": " judge-openai "},
        ]});
        assert_eq!(caller_in(&list, "a", "default").as_deref(), JUDGE);
        assert_eq!(caller_in(&list, "b", "default"), None);
        assert_eq!(caller_in(&list, "c", "default").as_deref(), JUDGE);
        assert_eq!(caller_in(&list, "missing", "default"), None);
        assert_eq!(caller_in(&json!({}), "a", "default"), None);
    }

    #[tokio::test]
    async fn rejected_managed_token_forces_exactly_one_refresh() {
        let (manager, store, oauth, _) = setup();
        *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
        let stale = token(&manager, JUDGE, Some("an-older-token"))
            .await
            .unwrap();
        assert_eq!(stale.access_token.as_deref(), Some("secret-account"));
        assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 0);
        let renewed = token(&manager, JUDGE, Some("secret-account"))
            .await
            .unwrap();
        assert_eq!(renewed.access_token.as_deref(), Some("rotated-access"));
        assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejected_legacy_token_is_ignored() {
        let (manager, _, oauth, _) = setup();
        let response = token(&manager, JUDGE, Some("secret-legacy")).await.unwrap();
        assert_eq!(response.access_token.as_deref(), Some("secret-legacy"));
        assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 0);
        // The provider's own streams still see the credential as valid.
        assert!(manager.resolve(None).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn signed_out_is_null() {
        let (manager, store, _, _) = setup();
        *store.value.lock().await = Some(SessionRecord::logged_out());
        let response = token(&manager, JUDGE, None).await.unwrap();
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            json!({"access_token": null})
        );
    }

    #[tokio::test]
    async fn expired_session_is_auth_expired() {
        let (manager, store, _, _) = setup();
        let mut record = SessionRecord::connected(credential("account", 3600));
        record.needs_login = true;
        *store.value.lock().await = Some(record);
        let err = token(&manager, JUDGE, None).await.err().unwrap();
        assert_eq!(bus_code(err), "provider/auth_expired");
    }

    #[test]
    fn request_reads_the_engine_stamped_caller() {
        let req: TokenRequest =
            serde_json::from_value(json!({"rejected_token": "t", "_caller_worker_id": "w-1"}))
                .unwrap();
        assert_eq!(req.caller_worker_id.as_deref(), Some("w-1"));
        assert_eq!(req.rejected_token.as_deref(), Some("t"));
        let empty: TokenRequest = serde_json::from_value(json!({})).unwrap();
        assert!(empty.rejected_token.is_none() && empty.caller_worker_id.is_none());
    }
}
