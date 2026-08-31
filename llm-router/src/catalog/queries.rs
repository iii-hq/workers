//! models::list / get / supports semantics (spec § Capability strings).
use crate::types::model::Model;

use super::store::CatalogStore;

/// Capability string → Model flag. Unknown strings match nothing.
pub fn model_supports(model: &Model, capability: &str) -> bool {
    match capability {
        "tools" => model.supports_tools == Some(true),
        "vision" => model.supports_vision == Some(true),
        "cache" => model.supports_cache == Some(true),
        "structured_output" => model.supports_structured_output == Some(true),
        "thinking" | "thinking:low" | "thinking:medium" | "thinking:high" => {
            model.supports_thinking == Some(true)
        }
        "thinking:xhigh" => model.supports_xhigh == Some(true),
        _ => false,
    }
}

pub async fn models_list(
    store: &CatalogStore,
    provider: Option<&str>,
    capability: Option<&str>,
) -> Vec<Model> {
    let mut models = match provider {
        Some(p) => {
            let slice = store.slice(p).await;
            // A composite `provider::model` id handed in where a provider id
            // belongs (the same console display form `models_get` retries):
            // an empty slice retries as the prefix's slice, so filtering by
            // the id the console shows lists that provider instead of
            // nothing. Exact first — this only fills an empty listing.
            match composite_retry(None, p) {
                Some((prefix, _)) if slice.is_empty() => store.slice(prefix).await,
                _ => slice,
            }
        }
        None => store.all().await,
    };
    if let Some(cap) = capability {
        models.retain(|m| model_supports(m, cap));
    }
    models
}

pub async fn models_get(store: &CatalogStore, provider: Option<&str>, id: &str) -> Option<Model> {
    if let Some(model) = lookup(store, provider, id).await {
        return Some(model);
    }
    // Composite `provider::model` ids — the console's display form, and so
    // what callers copy out of it and hand to `harness::send` — match nothing
    // above, because the catalog keys provider and id separately. Retry once
    // with the prefix stripped so a composite resolves like the split form.
    // Without this the miss is silent: budget returns null, context-manager
    // takes its conservative 8k fallback, and a healthy session dies with a
    // bewildering `context/overflow` instead of an unknown-model error.
    // Exact lookups run first, so this can only turn a None into a Some.
    let (prefix, rest) = composite_retry(provider, id)?;
    lookup(store, Some(prefix), rest).await
}

/// The `(provider, id)` pair a composite `provider::model` id should retry as.
/// `None` when the id carries no `::` prefix, when either half is empty, or
/// when the caller named a provider that contradicts the prefix — a
/// contradiction is a caller bug, not something to resolve by guessing.
/// Splits at the FIRST `::` so a model id may itself contain one.
fn composite_retry<'a>(provider: Option<&str>, id: &'a str) -> Option<(&'a str, &'a str)> {
    let (prefix, rest) = id.split_once("::")?;
    if prefix.is_empty() || rest.is_empty() || provider.is_some_and(|p| p != prefix) {
        return None;
    }
    Some((prefix, rest))
}

/// The effective `(provider, model)` pair for a caller-supplied reference:
/// a composite `provider::model` id splits — same rule as [`composite_retry`]
/// — when its prefix names a provider `known` recognizes; anything else
/// passes through untouched, so an id that merely contains `::` keeps meaning
/// itself. Dispatch consumers (routing, chat, count_tokens) resolve through
/// this BEFORE lookup; the catalog queries instead retry AFTER an exact miss,
/// so a literal catalog id containing `::` still wins there.
pub fn effective_model_ref<'a>(
    provider: Option<&'a str>,
    model: &'a str,
    known: impl Fn(&str) -> bool,
) -> (Option<&'a str>, &'a str) {
    match composite_retry(provider, model) {
        Some((prefix, rest)) if known(prefix) => (Some(prefix), rest),
        _ => (provider, model),
    }
}

/// One catalog lookup: exact `(provider, id)` when the provider is known,
/// otherwise the unique-owner match.
async fn lookup(store: &CatalogStore, provider: Option<&str>, id: &str) -> Option<Model> {
    match provider {
        Some(p) => store.get(p, id).await,
        // Provider-less lookup: `router::chat` resolves these via routing, but
        // metadata consumers (context-manager budgets a turn's context window
        // through models::get) often only know the model id — e.g. a react
        // spec carries no provider. Without this they silently fell to the
        // conservative 8k fallback and compacted tiny sessions every step.
        None => find_by_id(store.all().await, id),
    }
}

/// The unique catalog model with `id`, if exactly one provider serves it.
/// Ambiguous ids stay `None` (fail-open cold-window rule) — full precedence
/// resolution lives in routing, not here.
/// ponytail: unique-match only; thread routing::decide through if two
/// providers ever serve the same model id in practice.
fn find_by_id(models: Vec<Model>, id: &str) -> Option<Model> {
    let mut matches = models.into_iter().filter(|m| m.id == id);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// Unknown model → false; request-shaping callers use models::get → null for
/// the fail-open cold-window rule (spec § Capability defaults).
/// Resolution delegates to [`models_get`] — composite ids and provider-less
/// lookups included — so `supports` can never disagree with `get` about
/// whether an id exists. (A get/supports split silently downgraded every
/// harness JSON output contract to the `submit_result` fallback: budget
/// resolved the composite, supports reported it unsupported.)
pub async fn models_supports(
    store: &CatalogStore,
    provider: Option<&str>,
    id: &str,
    capability: &str,
) -> bool {
    match models_get(store, provider, id).await {
        Some(m) => model_supports(&m, capability),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::model::Model;

    fn sonnet() -> Model {
        Model {
            id: "claude-sonnet-4".into(),
            provider: "anthropic".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: Some(false),
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: None,
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        }
    }

    // Provider-less models::get: unique ids resolve (context-manager budgets
    // react turns that only know the model id — a null here silently throttles
    // them to the 8k fallback window); ambiguous/unknown ids stay None.
    #[test]
    fn find_by_id_resolves_only_unambiguous_ids() {
        let a = sonnet();
        let mut b = sonnet();
        b.provider = "other".into();
        let mut c = sonnet();
        c.id = "glm-5.2".into();
        c.provider = "zai".into();

        // Unique id → resolves regardless of provider.
        assert_eq!(
            find_by_id(vec![a.clone(), c.clone()], "glm-5.2").map(|m| m.provider),
            Some("zai".to_string())
        );
        // Two providers serving the same id → ambiguous → None.
        assert_eq!(find_by_id(vec![a.clone(), b], "claude-sonnet-4"), None);
        // Unknown id → None.
        assert_eq!(find_by_id(vec![a], "nope"), None);
        assert_eq!(find_by_id(vec![], "claude-sonnet-4"), None);
    }

    // Composite `provider::model` ids are the console's display form, so they
    // are what callers copy into `harness::send`. Before this retry the lookup
    // missed silently: budget returned null, context-manager fell to its
    // conservative 8k fallback, and a healthy session died with a bewildering
    // `context/overflow`. Store-backed resolution is covered in
    // tests/integration.rs; the split decision is pure and pinned here.
    #[test]
    fn composite_ids_retry_as_provider_and_model() {
        // No prefix → no retry (the exact lookup already had its chance).
        assert_eq!(composite_retry(None, "claude-sonnet-4"), None);
        // Composite, provider unknown to the caller → split it.
        assert_eq!(
            composite_retry(None, "openai-codex::codex/gpt-5.6-luna"),
            Some(("openai-codex", "codex/gpt-5.6-luna"))
        );
        // Caller's provider agrees with the prefix → still splits.
        assert_eq!(
            composite_retry(Some("openai-codex"), "openai-codex::codex/gpt-5.6-luna"),
            Some(("openai-codex", "codex/gpt-5.6-luna"))
        );
        // Caller's provider contradicts the prefix → refuse rather than guess.
        assert_eq!(
            composite_retry(Some("anthropic"), "openai-codex::codex/gpt-5.6-luna"),
            None
        );
        // Splits at the FIRST separator, so a model id may contain one.
        assert_eq!(
            composite_retry(None, "prov::weird::model"),
            Some(("prov", "weird::model"))
        );
        // Degenerate halves resolve to nothing rather than a bogus lookup.
        assert_eq!(composite_retry(None, "::model"), None);
        assert_eq!(composite_retry(None, "prov::"), None);
        assert_eq!(composite_retry(None, "::"), None);
    }

    // Dispatch (routing, chat, count_tokens) resolves composites BEFORE
    // lookup, gated on the prefix naming a known provider — the same split
    // rule as the catalog queries' retry-after-miss, so metadata and dispatch
    // can never disagree about which pair an id means.
    #[test]
    fn effective_model_ref_splits_only_known_provider_prefixes() {
        let known = |p: &str| p == "openai-codex";
        // Known prefix → exactly as if the caller had passed the pair.
        assert_eq!(
            effective_model_ref(None, "openai-codex::codex/gpt-5.6-luna", known),
            (Some("openai-codex"), "codex/gpt-5.6-luna")
        );
        // Unknown prefix → untouched: an id may contain `::` without naming a
        // provider, and such ids must keep meaning themselves.
        assert_eq!(
            effective_model_ref(None, "weird::thing", known),
            (None, "weird::thing")
        );
        // Caller's provider contradicts the prefix → untouched; the caller
        // bug stays visible downstream instead of being resolved by guessing.
        assert_eq!(
            effective_model_ref(Some("anthropic"), "openai-codex::codex/gpt-5.6-luna", known),
            (Some("anthropic"), "openai-codex::codex/gpt-5.6-luna")
        );
        // Plain ids pass through with the caller's provider intact.
        assert_eq!(
            effective_model_ref(Some("openai-codex"), "codex/gpt-5.6-luna", known),
            (Some("openai-codex"), "codex/gpt-5.6-luna")
        );
    }

    // Store-backed list/get/supports flows are exercised against a real engine
    // in tests/integration.rs; the capability mapping is pure and pinned here.
    #[test]
    fn capability_strings_map_to_model_flags() {
        let m = sonnet();
        assert!(model_supports(&m, "tools"));
        assert!(model_supports(&m, "vision"));
        assert!(model_supports(&m, "thinking"));
        assert!(model_supports(&m, "thinking:medium"));
        assert!(!model_supports(&m, "thinking:xhigh"));
        assert!(!model_supports(&m, "cache")); // absent flag reads as false
        assert!(!model_supports(&m, "structured_output"));
        assert!(!model_supports(&m, "bogus")); // unknown capability strings match nothing
    }
}
