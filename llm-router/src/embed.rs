//! `router::embed` — one front door for text embeddings, mirroring the
//! chat surface's shape: callers name a provider or let the router pick
//! one from its own provider registry. Providers implement
//! `provider::<id>::embed` (batch in, one vector per input, order
//! preserved); registered providers without the surface are skipped on a
//! function-not-found, so a new embed-capable provider needs zero router
//! changes to serve embeddings. An auto-pick also moves past a provider
//! that is not configured for good (no api key, a missing secret), but
//! never past a transient failure. When every registered provider has no
//! embed surface or is not configured for good, an auto-pick tries
//! embed-only providers last: a live `provider::<id>::embed` from a worker
//! that never declared itself (it serves no chat models, so it has no
//! place in the provider list). A registered provider whose `embed` has
//! answered before (its record's `embeds` flag) and that this router saw
//! live in the last 15 minutes blocks that fallback while its `embed` is
//! not found, whatever else of it is live: its vectors are the space
//! callers hold, and it is most likely restarting. One gone for longer
//! (removed, say) or never seen embedding does not block.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::unconfigured_for_good;
use crate::registry::store::RegistryStore;
use crate::secrets::SecretCache;
use crate::types::errors::{is_function_not_found, is_provider_unconfigured};

const EMBED_TIMEOUT_MS: u64 = 30_000;
/// Bound on the embed-only discovery list, so a slow engine cannot eat
/// the caller's whole budget before any provider is tried.
const LIST_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouterEmbedRequest {
    /// Embedding model id; the provider's default when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Provider id (e.g. `openai`); the first embed-capable provider in
    /// the live registry when omitted.
    #[serde(default)]
    pub provider: Option<String>,
    /// Texts to embed, one vector returned per input, order preserved.
    pub input: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RouterEmbedResponse {
    pub provider: String,
    pub model: String,
    /// One embedding per input, in input order.
    pub embeddings: Vec<Vec<f32>>,
}

/// One provider attempt. `Ok(None)` = this provider has no embed surface
/// (function not found) — try the next.
async fn try_provider(
    iii: &IIIClient,
    provider: &str,
    model: &Option<String>,
    input: &[String],
) -> Result<Option<RouterEmbedResponse>, Error> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: format!("provider::{provider}::embed"),
            payload: json!({ "model": model, "input": input }),
            action: None,
            timeout_ms: Some(EMBED_TIMEOUT_MS),
        })
        .await;
    let reply = match reply {
        Ok(v) => v,
        Err(e) if is_function_not_found(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let model = reply
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            Error::Handler(format!(
                "router/bad_provider_response: provider::{provider}::embed returned no model"
            ))
        })?;
    let embeddings: Vec<Vec<f32>> = reply
        .get("embeddings")
        .cloned()
        .and_then(|e| serde_json::from_value(e).ok())
        .ok_or_else(|| {
            Error::Handler(format!(
                "router/bad_provider_response: provider::{provider}::embed returned no embeddings array"
            ))
        })?;
    // Callers zip vectors back onto their inputs by position; a count
    // mismatch would misattribute every vector after the gap.
    if embeddings.len() != input.len() {
        return Err(Error::Handler(format!(
            "router/bad_provider_response: provider::{provider}::embed returned {} embeddings for {} inputs",
            embeddings.len(),
            input.len()
        )));
    }
    Ok(Some(RouterEmbedResponse {
        provider: provider.to_string(),
        model,
        embeddings,
    }))
}

pub fn make_embed(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    config: ConfigCell,
    secrets: Arc<SecretCache>,
) -> impl Fn(RouterEmbedRequest) -> BoxedEmbedFuture + Send + Sync + 'static {
    move |req: RouterEmbedRequest| {
        let iii = iii.clone();
        let registry = registry.clone();
        let config = snapshot(&config);
        let secrets = secrets.clone();
        Box::pin(async move {
            if req.input.is_empty() {
                return Err(Error::Handler(
                    "router/invalid_input: input must not be empty".into(),
                ));
            }
            let explicit = req.provider.filter(|p| !p.trim().is_empty());
            let auto = explicit.is_none();
            let candidates: Vec<String> = match explicit {
                Some(p) => vec![p],
                None => {
                    // Registered providers, `openai` preferred so the
                    // default choice is deterministic.
                    let mut ids = registry.ids().await;
                    ids.sort();
                    if let Some(pos) = ids.iter().position(|p| p == "openai") {
                        ids.swap(0, pos);
                    }
                    ids
                }
            };
            let registered = candidates.clone();
            let (lister, records) = (iii.clone(), registry.clone());
            let resp = first_embedding(
                candidates,
                auto,
                |provider| {
                    let (iii, model, input) = (iii.clone(), req.model.clone(), req.input.clone());
                    async move { try_provider(&iii, &provider, &model, &input).await }
                },
                |provider| {
                    let (config, secrets) = (config.clone(), secrets.clone());
                    async move { unconfigured_for_good(&config, &provider, &secrets).await }
                },
                |not_found| async move {
                    let awaited = records.awaited_embedders().await;
                    embed_only_ids(&lister, &registered, &awaited, &not_found).await
                },
            )
            .await?;
            // Off the reply path: the first answer costs one state write.
            let provider = resp.provider.clone();
            tokio::spawn(async move { registry.mark_embeds(&provider).await });
            Ok(resp)
        })
    }
}

/// Embed-only providers: a live `provider::<id>::embed` whose id never
/// declared itself to the router. A worker that serves embeddings and no
/// chat (iii-directory's local MiniLM) stays out of `router::provider::list`
/// this way, so model pickers never show it as a provider without models.
/// Alphabetical; empty when the engine cannot list functions. An error
/// when a provider whose vectors callers may hold is not serving right
/// now (see [`fallback_ids`]).
async fn embed_only_ids(
    iii: &IIIClient,
    registered: &[String],
    awaited: &[String],
    not_found: &[String],
) -> Result<Vec<String>, Error> {
    let listed = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            // Embed surfaces are tagged internal (never agent-discovered),
            // and the default list hides exactly those.
            payload: json!({ "prefix": "provider::", "include_internal": true }),
            action: None,
            timeout_ms: Some(LIST_TIMEOUT_MS),
        })
        .await;
    match listed {
        Ok(value) => {
            // The engine lists every namespace; `provider::<id>::embed` is
            // triggered in this worker's own.
            let namespace = iii.namespace().unwrap_or_else(|| "default".into());
            fallback_ids(&value, &namespace, registered, awaited, not_found).map_err(Error::Handler)
        }
        Err(e) => {
            tracing::debug!(error = %e, "embed-only provider discovery failed");
            Ok(Vec::new())
        }
    }
}

/// `(id, surface)` of every `provider::<id>::<surface>` row of an
/// `engine::functions::list` reply in `namespace` (a row without one is in
/// `default`).
fn provider_surfaces<'a>(listed: &'a Value, namespace: &str) -> Vec<(&'a str, &'a str)> {
    listed
        .get("functions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("namespace")
                .and_then(Value::as_str)
                .unwrap_or("default")
                == namespace
        })
        .filter_map(|row| {
            row.get("function_id")?
                .as_str()?
                .strip_prefix("provider::")?
                .split_once("::")
        })
        .filter(|(id, _)| !id.is_empty())
        .collect()
}

/// After every registered candidate failed to serve: the embed-only ids to
/// try (a live `embed`, no `on_router_ready` since they never declare, not
/// registered), or a `router/provider_unavailable` message when one of
/// `not_found` is `awaited` (`ProviderRecord::awaited_embedder`): falling
/// back would switch the caller's vector space, whatever else of it is live
/// (`listed` plays no part). Callers retry later, as on any failure.
fn fallback_ids(
    listed: &Value,
    namespace: &str,
    registered: &[String],
    awaited: &[String],
    not_found: &[String],
) -> Result<Vec<String>, String> {
    let waiting: Vec<&str> = not_found
        .iter()
        .filter(|&id| awaited.contains(id))
        .map(String::as_str)
        .collect();
    if !waiting.is_empty() {
        return Err(format!(
            "router/provider_unavailable: waiting for {} (down or restarting); embed-only \
             providers are not tried meanwhile",
            waiting.join(", ")
        ));
    }
    let surfaces = provider_surfaces(listed, namespace);
    let has = |id: &str, surface: &str| surfaces.contains(&(id, surface));
    let mut ids: Vec<String> = surfaces
        .iter()
        .filter(|(id, surface)| {
            *surface == "embed"
                && !has(id, "on_router_ready")
                && !registered.iter().any(|r| r == id)
        })
        .map(|(id, _)| id.to_string())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Walk the candidates in order. `Ok(None)` (no embed surface) moves on;
/// so does, on an auto-pick (`auto`), a provider that reports itself not
/// configured when the router's own state says that lasts (`for_good`).
/// Any other error is returned as is: falling back past a transient
/// failure (an upstream error, a secrets outage) would embed the same
/// caller's corpus into another model's vector space, and callers cannot
/// tell the spaces apart (they retry later instead). An auto-pick that
/// runs out asks `more` once (the embed-only providers), passing the
/// candidates whose surface was not found; its error is returned as is.
/// Never when a registered provider answered, so a working setup pays no
/// extra lookup.
async fn first_embedding<F, Fut, G, GFut, M, MFut>(
    mut candidates: Vec<String>,
    auto: bool,
    mut attempt: F,
    mut for_good: G,
    more: M,
) -> Result<RouterEmbedResponse, Error>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<RouterEmbedResponse>, Error>>,
    G: FnMut(String) -> GFut,
    GFut: Future<Output = bool>,
    M: FnOnce(Vec<String>) -> MFut,
    MFut: Future<Output = Result<Vec<String>, Error>>,
{
    let mut more = auto.then_some(more);
    let mut unconfigured = Vec::new();
    let mut not_found = Vec::new();
    let mut next = 0;
    loop {
        if next == candidates.len() {
            match more.take() {
                Some(more) => candidates.extend(more(std::mem::take(&mut not_found)).await?),
                None => break,
            }
            if next == candidates.len() {
                break;
            }
        }
        let provider = candidates[next].clone();
        next += 1;
        match attempt(provider.clone()).await {
            Ok(Some(resp)) => return Ok(resp),
            Ok(None) => not_found.push(provider),
            Err(e) if auto && is_provider_unconfigured(&e) && for_good(provider.clone()).await => {
                unconfigured.push(provider)
            }
            Err(e) => return Err(e),
        }
    }
    if candidates.is_empty() {
        return Err(Error::Handler(
            "router/no_embed_provider: no providers are registered".into(),
        ));
    }
    let detail = if unconfigured.is_empty() {
        String::new()
    } else {
        format!("; not configured: {}", unconfigured.join(", "))
    };
    Err(Error::Handler(format!(
        "router/no_embed_provider: none of the providers ({}) can serve \
         provider::<id>::embed{detail}",
        candidates.join(", ")
    )))
}

type BoxedEmbedFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterEmbedResponse, Error>> + Send>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn ok(provider: &str) -> RouterEmbedResponse {
        RouterEmbedResponse {
            provider: provider.into(),
            model: format!("{provider}-model"),
            embeddings: vec![vec![1.0]],
        }
    }

    fn remote(message: &str) -> Error {
        Error::Remote {
            code: "invocation_failed".into(),
            message: format!("handler error: {message}"),
            stacktrace: None,
        }
    }

    fn candidates() -> Vec<String> {
        vec!["openai".into(), "llamacpp".into()]
    }

    async fn openai_unconfigured(p: String) -> Result<Option<RouterEmbedResponse>, Error> {
        match p.as_str() {
            "openai" => Err(remote("provider/not_configured: no api_key")),
            _ => Ok(Some(ok(&p))),
        }
    }

    async fn no_more(_: Vec<String>) -> Result<Vec<String>, Error> {
        Ok(Vec::new())
    }

    async fn minilm(_: Vec<String>) -> Result<Vec<String>, Error> {
        Ok(vec!["minilm".to_string()])
    }

    #[tokio::test]
    async fn an_auto_pick_moves_past_a_provider_unconfigured_for_good() {
        let resp = first_embedding(
            candidates(),
            true,
            openai_unconfigured,
            |_| async { true },
            no_more,
        )
        .await
        .unwrap();
        assert_eq!(resp.provider, "llamacpp");
    }

    #[tokio::test]
    async fn an_unresolved_secret_is_transient_and_returned() {
        // The provider says "not configured" while its secret is still
        // resolving (or the secrets worker is down): no fallback.
        let err = first_embedding(
            candidates(),
            true,
            openai_unconfigured,
            |_| async { false },
            no_more,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("provider/not_configured"));
    }

    #[tokio::test]
    async fn a_transient_failure_is_returned_not_fallen_back_from() {
        let upstream = |p: String| async move {
            match p.as_str() {
                "openai" => Err(remote("provider/upstream_status: 503")),
                _ => Ok(Some(ok(&p))),
            }
        };
        let err = first_embedding(candidates(), true, upstream, |_| async { true }, no_more)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("provider/upstream_status: 503"));
    }

    #[tokio::test]
    async fn an_explicit_provider_is_never_substituted() {
        let asked = AtomicBool::new(false);
        let err = first_embedding(
            vec!["openai".to_string()],
            false,
            openai_unconfigured,
            |_| async { true },
            |_| async {
                asked.store(true, Ordering::SeqCst);
                Ok(vec!["minilm".to_string()])
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("provider/not_configured"));
        assert!(!asked.load(Ordering::SeqCst), "no embed-only discovery");
    }

    #[tokio::test]
    async fn none_left_names_the_unconfigured_ones() {
        let no_surface = |p: String| async move {
            match p.as_str() {
                "openai" => Err(remote("provider/not_configured: no api_key")),
                _ => Ok(None),
            }
        };
        let err = first_embedding(candidates(), true, no_surface, |_| async { true }, no_more)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("router/no_embed_provider"), "{msg}");
        assert!(msg.contains("not configured: openai"), "{msg}");
    }

    /// Only `minilm` serves; every registered provider lacks the surface.
    async fn only_minilm(p: String) -> Result<Option<RouterEmbedResponse>, Error> {
        Ok((p == "minilm").then(|| ok(&p)))
    }

    #[tokio::test]
    async fn embed_only_providers_are_tried_after_the_registered_ones() {
        let resp = first_embedding(candidates(), true, only_minilm, |_| async { true }, minilm)
            .await
            .unwrap();
        assert_eq!(resp.provider, "minilm");

        let alone = first_embedding(Vec::new(), true, only_minilm, |_| async { true }, minilm)
            .await
            .unwrap();
        assert_eq!(alone.provider, "minilm", "no provider registered at all");
    }

    #[tokio::test]
    async fn a_registered_provider_that_serves_skips_discovery() {
        let asked = AtomicBool::new(false);
        let resp = first_embedding(
            candidates(),
            true,
            |p: String| async move { Ok(Some(ok(&p))) },
            |_| async { true },
            |_| async {
                asked.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.provider, "openai");
        assert!(!asked.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_embed_only_provider_unconfigured_is_named_too() {
        let err = first_embedding(
            Vec::new(),
            true,
            |_| async { Err(remote("provider/not_configured: MiniLM bundle missing")) },
            |_| async { true },
            minilm,
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("router/no_embed_provider"), "{msg}");
        assert!(msg.contains("not configured: minilm"), "{msg}");

        let nothing = first_embedding(Vec::new(), true, only_minilm, |_| async { true }, no_more)
            .await
            .unwrap_err();
        assert!(nothing.to_string().contains("no providers are registered"));
    }

    #[tokio::test]
    async fn a_blocked_fallback_is_returned_and_nothing_else_is_tried() {
        let tried = std::sync::Mutex::new(Vec::new());
        let err = first_embedding(
            candidates(),
            true,
            |p: String| {
                tried.lock().unwrap().push(p);
                async { Ok(None) }
            },
            |_| async { true },
            |not_found| async move {
                assert_eq!(
                    not_found,
                    candidates(),
                    "every registered miss is passed on"
                );
                Err(Error::Handler(
                    "router/provider_unavailable: waiting for openai".into(),
                ))
            },
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("router/provider_unavailable"),
            "{err}"
        );
        assert_eq!(*tried.lock().unwrap(), candidates(), "minilm never tried");
    }

    fn row(function_id: &str) -> Value {
        json!({ "function_id": function_id, "namespace": "default" })
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn embed_only_ids_are_live_embed_surfaces_nobody_declared() {
        let listed = json!({ "functions": [
            row("provider::openai::embed"),
            row("provider::openai::on_router_ready"),
            row("provider::minilm::embed"),
            row("provider::minilm::embed"),
            row("provider::anthropic::on_router_ready"),
            row("provider::anthropic::stream"),
            row("provider::a::b::embed"),
            row("provider::::embed"),
            row("directory::embed"),
            { "function_id": 7 },
            // No namespace field: the default one.
            { "function_id": "provider::local::embed" },
        ]});
        let registered = ids(&["openai", "anthropic"]);
        assert_eq!(
            fallback_ids(&listed, "default", &registered, &[], &ids(&["anthropic"])),
            Ok(ids(&["local", "minilm"])),
            "a live chat-only provider is known to have no embed surface"
        );
        assert_eq!(
            fallback_ids(&json!({}), "default", &[], &[], &[]),
            Ok(Vec::new())
        );
    }

    #[test]
    fn only_an_awaited_embedder_blocks_the_fallback_whatever_of_it_is_live() {
        let openai = ids(&["openai"]);
        let minilm = row("provider::minilm::embed");
        let liveness = [
            json!({ "functions": [minilm.clone()] }), // down
            json!({ "functions": [row("provider::openai::stream"), minilm.clone()] }), // partly back
            json!({ "functions": [row("provider::openai::embed"), minilm.clone()] }),  // back
        ];
        for listed in &liveness {
            let err = fallback_ids(listed, "default", &openai, &openai, &openai).unwrap_err();
            assert!(
                err.starts_with("router/provider_unavailable: waiting for openai"),
                "{err}"
            );
            // Not awaited (never seen embedding, or gone past the grace
            // window: removed), or not a miss (keyless): falls back.
            assert_eq!(
                fallback_ids(listed, "default", &openai, &[], &openai),
                Ok(ids(&["minilm"]))
            );
            assert_eq!(
                fallback_ids(listed, "default", &openai, &openai, &[]),
                Ok(ids(&["minilm"]))
            );
        }
    }

    #[test]
    fn a_declaring_provider_is_never_tried_as_embed_only() {
        // Registered or not (it has not re-declared yet), it is no fallback.
        let listed = json!({ "functions": [
            row("provider::openai::embed"),
            row("provider::openai::on_router_ready"),
            row("provider::minilm::embed"),
        ]});
        assert_eq!(
            fallback_ids(&listed, "default", &[], &[], &[]),
            Ok(ids(&["minilm"]))
        );
    }

    #[test]
    fn only_the_routers_own_namespace_counts() {
        let listed = json!({ "functions": [
            { "function_id": "provider::minilm::embed", "namespace": "default" },
            { "function_id": "provider::other::embed", "namespace": "my-ns" },
        ]});
        assert_eq!(
            fallback_ids(&listed, "my-ns", &[], &[], &[]),
            Ok(ids(&["other"]))
        );
        assert_eq!(
            fallback_ids(&listed, "default", &[], &[], &[]),
            Ok(ids(&["minilm"]))
        );
    }
}
