//! Bus-side credential fetch for provider crates.
//!
//! Providers call `fetch_credential(iii, name)` once per request. The helper
//! issues an `auth::get_token` iii trigger and unwraps the resulting
//! [`Credential`] into a header-ready string. Providers that need to know
//! whether the credential came from API-key or OAuth (Anthropic does — its
//! API-key path uses `x-api-key`, its OAuth path uses `Authorization: Bearer`)
//! receive the [`Credential`] enum verbatim.
//!
//! End-to-end behaviour over a live engine is asserted by
//! `workers/replay-test/tests/p5_provider_contract.rs` (P5 Task 11).

use anyhow::{anyhow, Context, Result};
use auth_credentials::Credential;
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

/// Fetch the credential for `provider_name` via the iii bus. Returns
/// `Ok(None)` when the provider has neither a stored credential nor an env
/// match (callers usually map `None` → "no credential configured" error).
pub async fn fetch_credential(iii: &III, provider_name: &str) -> Result<Option<Credential>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "auth::get_token".to_string(),
            payload: json!({ "provider": provider_name }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| anyhow!("auth::get_token trigger failed: {e}"))?;
    if resp.is_null() {
        return Ok(None);
    }
    let cred: Credential = serde_json::from_value(resp).with_context(|| {
        format!("auth::get_token returned invalid Credential for {provider_name}")
    })?;
    Ok(Some(cred))
}

/// Collapse a credential into the header-bearer string.
///
/// `Credential::ApiKey` → the api key; `Credential::OAuth` → the access
/// token. Providers that branch on credential type (e.g. Anthropic switching
/// `x-api-key` vs `Authorization: Bearer`) match on the enum directly
/// instead of calling this helper.
pub fn credential_to_string(cred: &Credential) -> &str {
    match cred {
        Credential::ApiKey { key } => key.as_str(),
        Credential::OAuth { access_token, .. } => access_token.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_to_string_handles_both_variants() {
        let api = Credential::ApiKey { key: "k".into() };
        assert_eq!(credential_to_string(&api), "k");
        let oauth = Credential::OAuth {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        assert_eq!(credential_to_string(&oauth), "t");
    }
}
