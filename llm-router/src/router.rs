use crate::config::RouterConfig;
use crate::types::{
    AbTest, AbVariant, ClassifierConfig, ModelHealth, ModelRegistration, Policy, RoutingDecision,
    RoutingRequest,
};
use rand::Rng;

// Router is intentionally UNOPINIONATED about model names. It only matches
// user-registered policies, classifiers, models, and health records stored in
// engine state. No hardcoded model catalog.

pub fn match_policy(req: &RoutingRequest, p: &Policy) -> bool {
    if !p.enabled {
        return false;
    }
    if let Some(t) = &p.match_rule.tenant {
        if req.tenant.as_deref() != Some(t.as_str()) {
            return false;
        }
    }
    if let Some(f) = &p.match_rule.feature {
        if req.feature.as_deref() != Some(f.as_str()) {
            return false;
        }
    }
    if let Some(want_tags) = &p.match_rule.tags {
        match &req.tags {
            Some(have) => {
                if !want_tags.iter().any(|t| have.iter().any(|h| h == t)) {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

pub fn match_ab(req: &RoutingRequest, t: &AbTest) -> bool {
    if t.status != "running" {
        return false;
    }
    if let Some(tn) = &t.match_rule.tenant {
        if req.tenant.as_deref() != Some(tn.as_str()) {
            return false;
        }
    }
    if let Some(f) = &t.match_rule.feature {
        if req.feature.as_deref() != Some(f.as_str()) {
            return false;
        }
    }
    true
}

pub fn pick_ab_variant<R: Rng>(variants: &[AbVariant], rng: &mut R) -> Option<String> {
    // Accumulate as u64 so N variants with max-u32 weights can't overflow.
    let total: u64 = variants.iter().map(|v| v.weight as u64).sum();
    if total == 0 {
        return None;
    }
    let mut pick = rng.gen_range(0..total);
    for v in variants {
        let w = v.weight as u64;
        if pick < w {
            return Some(v.model.clone());
        }
        pick -= w;
    }
    None
}

pub fn policy_specificity(p: &Policy) -> usize {
    let mut score = 0usize;
    if p.match_rule.tenant.is_some() {
        score += 1;
    }
    if p.match_rule.feature.is_some() {
        score += 1;
    }
    if let Some(tags) = &p.match_rule.tags {
        score += tags.len();
    }
    score
}

pub fn skip_unavailable(model: &str, health: &[ModelHealth], error_rate_skip: f64) -> bool {
    if let Some(h) = health.iter().find(|h| h.model == model) {
        if !h.available {
            return true;
        }
        if let Some(r) = h.error_rate {
            if r >= error_rate_skip {
                return true;
            }
        }
    }
    false
}

/// Classify a prompt into a category label. Returns (category, confidence).
/// The category is abstract — it does NOT name a model. The user's classifier
/// config is what maps category → model.
pub fn heuristic_complexity(prompt: &str) -> (&'static str, f64) {
    let len = prompt.chars().count();
    let has_code = prompt.contains("```") || prompt.contains("fn ") || prompt.contains("def ");
    let has_math = prompt.contains('$') || prompt.contains("prove") || prompt.contains("derive");
    let multi_step =
        prompt.contains("first") || prompt.contains("then") || prompt.contains("after that");

    if len < 80 && !has_code && !has_math {
        ("simple", 0.85)
    } else if len < 300 && !multi_step && !has_math {
        ("moderate", 0.75)
    } else if len < 1200 && !has_math {
        ("complex", 0.8)
    } else {
        ("expert", 0.9)
    }
}

#[derive(Default)]
pub struct DecideContext<'a> {
    pub policies: &'a [Policy],
    pub ab_tests: &'a [AbTest],
    pub health: &'a [ModelHealth],
    pub classifier: Option<&'a ClassifierConfig>,
    pub models: &'a [ModelRegistration],
}

pub fn decide(
    req: &RoutingRequest,
    ctx: DecideContext<'_>,
    cfg: &RouterConfig,
    rng: &mut impl Rng,
) -> RoutingDecision {
    let mut matched: Vec<&Policy> = ctx
        .policies
        .iter()
        .filter(|p| match_policy(req, p))
        .collect();
    // Deterministic ordering: priority desc, then specificity desc (more
    // match-rule fields = more specific), then policy id asc as a stable
    // tie-breaker so the same inputs always pick the same policy.
    matched.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| policy_specificity(b).cmp(&policy_specificity(a)))
            .then_with(|| a.id.cmp(&b.id))
    });

    // Path 1: policy match → resolve → health → budget.
    if let Some(policy) = matched.first().copied() {
        let mut chosen = policy.action.model.clone();
        let mut confidence = 0.9;
        let mut reason = format!("policy: {}", policy.name);

        if chosen == "auto" {
            let (cls, conf) = heuristic_complexity(&req.prompt);
            match ctx.classifier.and_then(|c| c.thresholds.get(cls)) {
                Some(m) => {
                    chosen = m.clone();
                    confidence = conf;
                    reason = format!("policy: {} + classifier: {}", policy.name, cls);
                }
                None => {
                    return RoutingDecision {
                        model: policy.action.fallback.clone().unwrap_or_default(),
                        reason: format!(
                            "policy: {} asks auto but no classifier mapping for '{}'",
                            policy.name, cls
                        ),
                        policy_id: Some(policy.id.clone()),
                        ab_test_id: None,
                        fallback: None,
                        confidence: 0.3,
                        provider: None,
                    };
                }
            }
        }

        if skip_unavailable(&chosen, ctx.health, cfg.health_skip_threshold_error_rate) {
            if let Some(fb) = &policy.action.fallback {
                if skip_unavailable(fb, ctx.health, cfg.health_skip_threshold_error_rate) {
                    // Both primary and fallback are unhealthy. Return the
                    // primary with a warning so the caller sees the
                    // degradation instead of masking it with a bad fallback.
                    reason = format!(
                        "{} (primary + fallback both unhealthy — returning primary)",
                        reason
                    );
                    confidence *= 0.4;
                } else {
                    return RoutingDecision {
                        model: fb.clone(),
                        reason: format!("{} (primary unhealthy → fallback)", reason),
                        policy_id: Some(policy.id.clone()),
                        ab_test_id: None,
                        fallback: None,
                        confidence: confidence * 0.8,
                        provider: None,
                    };
                }
            }
        }

        if let Some(remaining) = req.budget_remaining_usd {
            if let Some(max_per_req) = policy.action.max_cost_per_request_usd {
                if max_per_req > remaining && remaining > 0.0 {
                    if let Some(downgraded) = downgrade_to_fit(remaining, req, ctx.models) {
                        let degraded = skip_unavailable(
                            &downgraded,
                            ctx.health,
                            cfg.health_skip_threshold_error_rate,
                        );
                        let mut dreason = format!("budget constraint: downgraded from {}", chosen);
                        let mut dconf = confidence * 0.7;
                        if degraded {
                            dreason = format!("{} (downgrade target unhealthy)", dreason);
                            dconf *= 0.5;
                        }
                        return RoutingDecision {
                            model: downgraded,
                            reason: dreason,
                            policy_id: Some(policy.id.clone()),
                            ab_test_id: None,
                            fallback: policy.action.fallback.clone(),
                            confidence: dconf,
                            provider: None,
                        };
                    }
                    reason = format!(
                        "{} (over budget but no registered model fits — using original)",
                        reason
                    );
                }
            }
        }

        return RoutingDecision {
            model: chosen,
            reason,
            policy_id: Some(policy.id.clone()),
            ab_test_id: None,
            fallback: policy.action.fallback.clone(),
            confidence,
            provider: None,
        };
    }

    // Path 2: no policy — try classifier, still subject to health check.
    if let Some(classifier) = ctx.classifier {
        let (cls, conf) = heuristic_complexity(&req.prompt);
        if let Some(m) = classifier.thresholds.get(cls) {
            let mut confidence = conf;
            let mut reason = format!("no policy, classifier: {}", cls);
            if skip_unavailable(m, ctx.health, cfg.health_skip_threshold_error_rate) {
                reason = format!("{} (model unhealthy, no policy fallback)", reason);
                confidence *= 0.5;
            }
            return RoutingDecision {
                model: m.clone(),
                reason,
                policy_id: None,
                ab_test_id: None,
                fallback: None,
                confidence,
                provider: None,
            };
        }
    }

    // Path 3: no policy, no classifier — AB test is the last resort. Still
    // subject to health check.
    if let Some(ab) = ctx.ab_tests.iter().find(|t| match_ab(req, t)) {
        if let Some(model) = pick_ab_variant(&ab.variants, rng) {
            let mut confidence = 1.0;
            let mut reason = format!("ab-test: {}", ab.name);
            if skip_unavailable(&model, ctx.health, cfg.health_skip_threshold_error_rate) {
                reason = format!("{} (variant unhealthy, no policy fallback)", reason);
                confidence *= 0.5;
            }
            return RoutingDecision {
                model,
                reason,
                policy_id: None,
                ab_test_id: Some(ab.id.clone()),
                fallback: None,
                confidence,
                provider: None,
            };
        }
    }

    RoutingDecision {
        model: String::new(),
        reason: "no policy matched and no classifier configured".to_string(),
        policy_id: None,
        ab_test_id: None,
        fallback: None,
        confidence: 0.0,
        provider: None,
    }
}

fn downgrade_to_fit(
    remaining_usd: f64,
    req: &RoutingRequest,
    models: &[ModelRegistration],
) -> Option<String> {
    if models.is_empty() {
        return None;
    }
    let mut candidates: Vec<(&ModelRegistration, f64)> = models
        .iter()
        .filter_map(|m| {
            let est = match (m.input_per_1m, m.output_per_1m) {
                (Some(i), Some(o)) => (i + o) / 2_000_000.0 * 1_000.0,
                _ => return None,
            };
            if est > remaining_usd {
                return None;
            }
            if let Some(min_q) = &req.min_quality {
                if m.quality.as_deref() != Some(min_q.as_str())
                    && !matches_higher_or_equal(&m.quality, min_q)
                {
                    return None;
                }
            }
            Some((m, est))
        })
        .collect();
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    candidates.first().map(|(m, _)| m.model.clone())
}

fn matches_higher_or_equal(have: &Option<String>, want: &str) -> bool {
    const ORDER: &[&str] = &["low", "medium", "high", "flagship"];
    let rank = |s: &str| ORDER.iter().position(|x| *x == s);
    match (have.as_deref().and_then(rank), rank(want)) {
        (Some(h), Some(w)) => h >= w,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PolicyAction, PolicyMatch};
    use rand::SeedableRng;
    use std::collections::HashMap;

    fn mk_policy(
        id: &str,
        tenant: Option<&str>,
        feature: Option<&str>,
        model: &str,
        priority: i32,
    ) -> Policy {
        Policy {
            id: id.into(),
            name: id.into(),
            match_rule: PolicyMatch {
                tenant: tenant.map(String::from),
                feature: feature.map(String::from),
                tags: None,
            },
            action: PolicyAction {
                model: model.into(),
                fallback: None,
                max_cost_per_request_usd: None,
            },
            priority,
            enabled: true,
            created_at_ms: 0,
        }
    }

    fn mk_req(tenant: Option<&str>, feature: Option<&str>, prompt: &str) -> RoutingRequest {
        RoutingRequest {
            tenant: tenant.map(String::from),
            feature: feature.map(String::from),
            user: None,
            prompt: prompt.into(),
            tags: None,
            budget_remaining_usd: None,
            latency_slo_ms: None,
            min_quality: None,
            classifier_id: None,
        }
    }

    fn empty_ctx<'a>() -> DecideContext<'a> {
        DecideContext::default()
    }

    #[test]
    fn test_match_policy_tenant_and_feature() {
        let p = mk_policy("p1", Some("acme"), Some("support"), "model-a", 100);
        assert!(match_policy(
            &mk_req(Some("acme"), Some("support"), "hi"),
            &p
        ));
        assert!(!match_policy(
            &mk_req(Some("other"), Some("support"), "hi"),
            &p
        ));
    }

    #[test]
    fn test_match_policy_disabled_rejected() {
        let mut p = mk_policy("p", None, None, "m", 1);
        p.enabled = false;
        assert!(!match_policy(&mk_req(None, None, "hi"), &p));
    }

    #[test]
    fn test_priority_highest_wins() {
        let lo = mk_policy("lo", Some("acme"), None, "lo-model", 10);
        let hi = mk_policy("hi", Some("acme"), None, "hi-model", 100);
        let cfg = RouterConfig::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[lo, hi],
            ..empty_ctx()
        };
        let d = decide(&mk_req(Some("acme"), None, "hi"), ctx, &cfg, &mut rng);
        assert_eq!(d.model, "hi-model");
        assert_eq!(d.policy_id.as_deref(), Some("hi"));
    }

    #[test]
    fn test_unhealthy_primary_uses_fallback() {
        let mut p = mk_policy("p", None, None, "primary-m", 10);
        p.action.fallback = Some("fallback-m".into());
        let health = vec![ModelHealth {
            model: "primary-m".into(),
            available: false,
            latency_p99_ms: None,
            error_rate: None,
            last_checked_ms: 0,
        }];
        let cfg = RouterConfig::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[p],
            health: &health,
            ..empty_ctx()
        };
        let d = decide(&mk_req(None, None, "hi"), ctx, &cfg, &mut rng);
        assert_eq!(d.model, "fallback-m");
        assert!(d.reason.contains("fallback"));
    }

    #[test]
    fn test_auto_needs_classifier_mapping() {
        let p = mk_policy("auto-p", None, None, "auto", 10);
        let cfg = RouterConfig::default();
        let mut thresholds = HashMap::new();
        thresholds.insert("simple".to_string(), "cheap-model".to_string());
        thresholds.insert("moderate".to_string(), "mid-model".to_string());
        thresholds.insert("complex".to_string(), "strong-model".to_string());
        thresholds.insert("expert".to_string(), "frontier-model".to_string());
        let classifier = ClassifierConfig {
            id: "default".to_string(),
            thresholds,
            created_at_ms: 0,
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[p],
            classifier: Some(&classifier),
            ..empty_ctx()
        };
        let d = decide(&mk_req(None, None, "hi there"), ctx, &cfg, &mut rng);
        assert_eq!(d.model, "cheap-model");
        assert!(d.reason.contains("classifier: simple"));
    }

    #[test]
    fn test_auto_without_classifier_returns_empty_and_reason() {
        let p = mk_policy("auto-p", None, None, "auto", 10);
        let cfg = RouterConfig::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[p],
            ..empty_ctx()
        };
        let d = decide(&mk_req(None, None, "hi"), ctx, &cfg, &mut rng);
        assert!(d.reason.contains("no classifier mapping"));
        assert_eq!(d.confidence, 0.3);
    }

    #[test]
    fn test_no_policy_no_classifier_empty_model() {
        let cfg = RouterConfig::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let d = decide(&mk_req(None, None, "hi"), empty_ctx(), &cfg, &mut rng);
        assert!(d.model.is_empty());
        assert!(d.reason.contains("no policy"));
    }

    #[test]
    fn test_no_policy_with_classifier_falls_through() {
        let cfg = RouterConfig::default();
        let mut thresholds = HashMap::new();
        thresholds.insert("simple".to_string(), "cheap".to_string());
        let classifier = ClassifierConfig {
            id: "default".to_string(),
            thresholds,
            created_at_ms: 0,
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            classifier: Some(&classifier),
            ..empty_ctx()
        };
        let d = decide(&mk_req(None, None, "hi"), ctx, &cfg, &mut rng);
        assert_eq!(d.model, "cheap");
    }

    #[test]
    fn test_heuristic_returns_category_only() {
        assert_eq!(heuristic_complexity("hi").0, "simple");
        let long = "prove that ".repeat(200);
        assert_eq!(heuristic_complexity(&long).0, "expert");
    }

    #[test]
    fn test_ab_pick_variant() {
        let variants = vec![
            AbVariant {
                model: "a".into(),
                weight: 50,
            },
            AbVariant {
                model: "b".into(),
                weight: 50,
            },
        ];
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let picked = pick_ab_variant(&variants, &mut rng).unwrap();
        assert!(picked == "a" || picked == "b");
    }

    #[test]
    fn test_ab_zero_weight_none() {
        let variants = vec![AbVariant {
            model: "a".into(),
            weight: 0,
        }];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert!(pick_ab_variant(&variants, &mut rng).is_none());
    }

    #[test]
    fn test_downgrade_needs_registered_models() {
        let mut p = mk_policy("p", None, None, "expensive", 10);
        p.action.max_cost_per_request_usd = Some(5.0);
        let cfg = RouterConfig::default();
        let mut req = mk_req(None, None, "hi");
        req.budget_remaining_usd = Some(0.01);
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[p],
            ..empty_ctx()
        };
        let d = decide(&req, ctx, &cfg, &mut rng);
        assert_eq!(d.model, "expensive");
        assert!(d.reason.contains("no registered model fits"));
    }

    #[test]
    fn test_downgrade_with_registered_model_picks_cheapest() {
        let mut p = mk_policy("p", None, None, "expensive", 10);
        p.action.max_cost_per_request_usd = Some(5.0);
        let cfg = RouterConfig::default();
        let mut req = mk_req(None, None, "hi");
        req.budget_remaining_usd = Some(0.01);
        let models = vec![
            ModelRegistration {
                model: "cheap-mini".into(),
                quality: Some("low".into()),
                input_per_1m: Some(0.5),
                output_per_1m: Some(1.0),
                provider: None,
                max_tokens: None,
                metadata: None,
                registered_at_ms: 0,
            },
            ModelRegistration {
                model: "cheap-nano".into(),
                quality: Some("low".into()),
                input_per_1m: Some(0.1),
                output_per_1m: Some(0.4),
                provider: None,
                max_tokens: None,
                metadata: None,
                registered_at_ms: 0,
            },
        ];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let ctx = DecideContext {
            policies: &[p],
            models: &models,
            ..empty_ctx()
        };
        let d = decide(&req, ctx, &cfg, &mut rng);
        assert_eq!(d.model, "cheap-nano");
        assert!(d.reason.contains("downgraded"));
    }

    #[test]
    fn test_skip_unavailable_respects_error_rate() {
        let h = vec![ModelHealth {
            model: "m".into(),
            available: true,
            latency_p99_ms: None,
            error_rate: Some(0.5),
            last_checked_ms: 0,
        }];
        assert!(skip_unavailable("m", &h, 0.3));
        assert!(!skip_unavailable("m", &h, 0.8));
    }

    #[test]
    fn test_routing_decision_serializes_provider_when_set() {
        let d = RoutingDecision {
            model: "claude-sonnet-4".into(),
            reason: "test".into(),
            policy_id: None,
            ab_test_id: None,
            fallback: None,
            confidence: 1.0,
            provider: Some("anthropic".into()),
        };
        let v = serde_json::to_value(&d).expect("serialize");
        assert_eq!(
            v.get("provider").and_then(|x| x.as_str()),
            Some("anthropic")
        );
    }

    #[test]
    fn test_routing_decision_omits_provider_when_unset() {
        let d = RoutingDecision {
            model: "x".into(),
            reason: "test".into(),
            policy_id: None,
            ab_test_id: None,
            fallback: None,
            confidence: 1.0,
            provider: None,
        };
        let v = serde_json::to_value(&d).expect("serialize");
        assert!(
            v.get("provider").is_none(),
            "provider field must be skipped when None"
        );
    }

    #[test]
    fn test_routing_request_tolerates_iii_sdk_caller_metadata() {
        // iii-sdk injects `_caller_worker_id` into every trigger payload.
        // RoutingRequest must accept (and ignore) that field; otherwise
        // every `iii.trigger("router::decide", ...)` call gets rejected
        // before any router logic runs. Regression test for the prior
        // `deny_unknown_fields` posture.
        let raw = serde_json::json!({
            "_caller_worker_id": "worker-uuid-123",
            "prompt": "hello",
            "tenant": "acme",
        });
        let parsed: RoutingRequest =
            serde_json::from_value(raw).expect("RoutingRequest must accept caller metadata");
        assert_eq!(parsed.prompt, "hello");
        assert_eq!(parsed.tenant.as_deref(), Some("acme"));
    }
}
