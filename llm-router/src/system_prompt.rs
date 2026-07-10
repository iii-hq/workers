//! `router::system_prompt::get`: serve the effective per-provider identity
//! prompt from the reactive in-memory configuration snapshot. Precedence:
//! operator override (slice `system_prompt`, when set and non-empty) →
//! provider-declared (`ProviderDeclaration.system_prompt`) → null. An
//! unset/null override means "use the provider's default"; an unknown
//! provider or no resolvable provider yields null; callers fall back to their
//! own default prompt. Never errors: prompt lookup must not take a turn down.
//!
//! Engine-backed coverage: tests/integration.rs (declared / override /
//! unset / default_provider resolution).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use serde_json::Value;

use crate::config::state::{snapshot, ConfigCell};
use crate::registry::store::RegistryStore;
use crate::settings::provider_slices;
use crate::types::router::{SystemPromptGetRequest, SystemPromptGetResponse};

/// Pure precedence core: a set, non-empty operator override wins; otherwise
/// the provider-declared prompt (null/unset/empty override = provider default).
fn effective_prompt(entry: &Value, provider: &str, declared: Option<String>) -> Option<String> {
    provider_slices(entry)
        .get(provider)
        .and_then(|s| s.get("system_prompt"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| declared.filter(|s| !s.is_empty()))
}

pub fn make_system_prompt_get(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
) -> impl Fn(SystemPromptGetRequest) -> BoxFuture<'static, Result<SystemPromptGetResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: SystemPromptGetRequest| {
        let (config, registry) = (config.clone(), registry.clone());
        Box::pin(async move {
            let config = snapshot(&config);
            let provider = req
                .provider
                .or_else(|| config.settings().default_provider.clone());
            let Some(provider) = provider else {
                return Ok(SystemPromptGetResponse::default());
            };
            let declared = registry
                .get(&provider)
                .await
                .and_then(|r| r.declaration.system_prompt);
            Ok(SystemPromptGetResponse {
                system_prompt: effective_prompt(config.value(), &provider, declared),
                provider: Some(provider),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_prompt_serves_when_slice_is_silent() {
        for entry in [json!(null), json!({ "providers": { "a": {} } })] {
            assert_eq!(
                effective_prompt(&entry, "a", Some("declared".into())).as_deref(),
                Some("declared")
            );
        }
        assert_eq!(effective_prompt(&json!(null), "a", None), None);
        assert_eq!(
            effective_prompt(&json!(null), "a", Some(String::new())),
            None
        );
    }

    #[test]
    fn set_override_wins_but_empty_or_null_serves_the_provider_default() {
        let entry = json!({ "providers": { "a": { "system_prompt": "operator" } } });
        assert_eq!(
            effective_prompt(&entry, "a", Some("declared".into())).as_deref(),
            Some("operator")
        );
        // empty string (freshly toggled to "set") and explicit null (toggled
        // back to "unset") both mean: use the provider's default
        for unset in [json!(""), json!(null)] {
            let entry = json!({ "providers": { "a": { "system_prompt": unset } } });
            assert_eq!(
                effective_prompt(&entry, "a", Some("declared".into())).as_deref(),
                Some("declared")
            );
        }
        // other providers unaffected by a's override
        let entry = json!({ "providers": { "a": { "system_prompt": "operator" } } });
        assert_eq!(
            effective_prompt(&entry, "b", Some("declared".into())).as_deref(),
            Some("declared")
        );
    }
}
