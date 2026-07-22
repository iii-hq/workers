//! Per-provider caches for the two engine round trips every stream call
//! paid before its first upstream byte: the registration token
//! (`state::get`) and `router::provider::resolve`.
//!
//! The token changes only on re-registration, which happens in-process —
//! `store` updates the cell alongside iii-state, so reads never miss after
//! boot. The resolve response (credential, api_url, max_tokens) changes when
//! an operator edits the ROUTER's configuration entry, which providers
//! cannot observe; a short TTL bounds that staleness, and callers invalidate
//! eagerly on `router::ready` (router restart) and on upstream auth errors
//! (credential rotated out from under us). Invalidation only drops the
//! cache — retry/failover stays the router's job.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::types::router::ProviderResolveResponse;

/// Staleness bound after an unobservable operator edit to the router's
/// provider slice (credential/api_url/max_tokens).
pub const RESOLVE_TTL: Duration = Duration::from_secs(30);

/// The per-provider scaffold state a stream/register/ready handler clones.
#[derive(Clone, Default)]
pub struct ScaffoldCache {
    token: Arc<RwLock<Option<String>>>,
    resolve: Arc<RwLock<Option<(ProviderResolveResponse, Instant)>>>,
}

impl ScaffoldCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached token, falling back to iii-state and populating the cell.
    pub async fn load_token(&self, iii: &IIIClient, scope: &str) -> Option<String> {
        if let Some(token) = self
            .token
            .read()
            .expect("token cache lock poisoned")
            .clone()
        {
            return Some(token);
        }
        let token = super::state::load_token(iii, scope).await?;
        *self.token.write().expect("token cache lock poisoned") = Some(token.clone());
        Some(token)
    }

    /// Persist a (re)issued token and refresh the cell.
    pub async fn store_token(
        &self,
        iii: &IIIClient,
        scope: &str,
        token: &str,
    ) -> Result<(), Error> {
        super::state::store_token(iii, scope, token).await?;
        *self.token.write().expect("token cache lock poisoned") = Some(token.to_string());
        Ok(())
    }

    /// `router::provider::resolve`, served from cache within [`RESOLVE_TTL`].
    pub async fn resolve(
        &self,
        iii: &IIIClient,
        provider_id: &str,
        token: Option<&str>,
        credential_env_var: Option<&str>,
    ) -> Result<ProviderResolveResponse, Error> {
        if let Some(resolved) = self.fresh_resolve() {
            return Ok(resolved);
        }
        // The lock is never held across the await; concurrent misses may
        // duplicate one resolve, which is harmless.
        let resolved =
            super::router_client::resolve(iii, provider_id, token, credential_env_var).await?;
        *self.resolve.write().expect("resolve cache lock poisoned") =
            Some((resolved.clone(), Instant::now()));
        Ok(resolved)
    }

    fn fresh_resolve(&self) -> Option<ProviderResolveResponse> {
        let guard = self.resolve.read().expect("resolve cache lock poisoned");
        let (resolved, stored_at) = guard.as_ref()?;
        (stored_at.elapsed() <= RESOLVE_TTL).then(|| resolved.clone())
    }

    /// Drop the cached resolve response AND token. Call on `router::ready`
    /// (a restarted router may carry new config; the token survives but the
    /// re-declare path refreshes it anyway) and on upstream auth errors
    /// (the operator rotated the credential; the next attempt re-resolves).
    pub fn invalidate(&self) {
        *self.resolve.write().expect("resolve cache lock poisoned") = None;
        *self.token.write().expect("token cache lock poisoned") = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(url: &str) -> ProviderResolveResponse {
        serde_json::from_value(serde_json::json!({
            "configured": true,
            "source": "config",
            "credential": null,
            "api_url": url,
        }))
        .expect("minimal resolve response shape")
    }

    #[test]
    fn resolve_cache_serves_within_ttl_and_invalidates() {
        let cache = ScaffoldCache::new();
        assert!(cache.fresh_resolve().is_none());
        *cache.resolve.write().unwrap() = Some((resolved("https://a"), Instant::now()));
        assert!(cache.fresh_resolve().is_some());
        cache.invalidate();
        assert!(cache.fresh_resolve().is_none());
    }

    #[test]
    fn resolve_cache_expires_after_ttl() {
        let cache = ScaffoldCache::new();
        let stale = Instant::now() - (RESOLVE_TTL + Duration::from_secs(1));
        *cache.resolve.write().unwrap() = Some((resolved("https://a"), stale));
        assert!(cache.fresh_resolve().is_none());
    }

    #[test]
    fn invalidate_also_clears_the_token_cell() {
        let cache = ScaffoldCache::new();
        *cache.token.write().unwrap() = Some("tok".into());
        cache.invalidate();
        assert!(cache.token.read().unwrap().is_none());
    }
}
