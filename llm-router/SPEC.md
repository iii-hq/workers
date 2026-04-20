# llm-router worker

Policy-based model selection engine on iii-engine. Not another gateway — a composable routing brain that plugs INTO existing gateways (LiteLLM, Bifrost, OpenRouter). Routing rules are iii functions: hot-swappable, observable, testable without redeploying.

## Why This Is Different

RouteLLM trains ML classifiers on preference data. LiteLLM hardcodes model names in YAML. Portkey uses static rules in a dashboard. All of them bake routing decisions into their infrastructure layer.

This worker separates routing decisions from routing execution:

- **Decisions** live in iii-engine (functions + state) — changeable at runtime, testable in isolation, observable via OTel.
- **Execution** lives in your existing gateway — it just asks the router "which model?" before forwarding.

This means you can change routing rules without redeploying your gateway, A/B test models without code changes, and enforce budget constraints from `llm-budget` in the same routing decision.

## Architecture

```
Your App → Gateway (LiteLLM/Bifrost) → router::decide → Gateway forwards to chosen model
                                            ↓
                                    Checks: policies, budgets, A/B tests, classifier
```

One HTTP call adds ~5-15ms. The gateway caches the decision for identical request signatures if latency matters.

## State Scopes

```
policies         — routing rules {id, name, match: {tenant, feature, tags}, action: {model, fallback}}
classifiers      — complexity classifier configs {id, model, threshold, categories}
ab_tests         — A/B test definitions {id, name, variants: [{model, weight}], metric, status}
ab_events        — recorded outcomes {test_id, variant, quality_score, latency_ms, cost_usd}
routing_log      — decision audit trail {timestamp, request_id, policy_matched, model_selected, reason}
model_health     — provider status {model → {available, latency_p99, error_rate, last_checked}}
```

## Functions (18)

### Core Routing

```
router::decide
  Input:  {
    tenant, feature?, user?,
    prompt: string,              # first 500 chars is enough for classification
    tags?: ["payments", "p0"],   # application-level tags
    budget_remaining_usd?: 1.50, # from budget::check
    latency_slo_ms?: 2000,       # max acceptable latency
    min_quality?: "high"|"medium"|"low"
  }
  Output: {
    model: "claude-haiku-4.5",
    reason: "policy:support-default matched, budget constraint applied",
    policy_id?: "pol-xxx",
    ab_test_id?: "ab-xxx",
    fallback: "gpt-4.1-mini",
    confidence: 0.92
  }
  Notes:  The hot path. Evaluation order:
          1. Check model health (skip unavailable providers)
          2. Match policies by tenant → feature → tags (most specific wins)
          3. If policy says "auto", run complexity classifier
          4. If A/B test active for this scope, apply variant weights
          5. If budget constraint, cap to cheapest model that meets min_quality
          6. Return model + reason + fallback

router::decide_batch
  Input:  {requests: [{tenant, feature, prompt, ...}]}
  Output: {decisions: [{model, reason, ...}]}
  Notes:  Batch version for pipeline workloads. Single round-trip.
```

### Policy Management

```
router::policy_create
  Input:  {
    name: "support-default",
    match: {
      tenant?: "acme-corp",      # exact match
      feature?: "support-chat",  # exact match
      tags?: ["low-risk"],       # any-of match
    },
    action: {
      model: "claude-haiku-4.5", # or "auto" for classifier-based
      fallback: "gpt-4.1-mini",
      max_cost_per_request_usd?: 0.01,
    },
    priority: 100,               # higher = checked first
    enabled: true
  }
  Output: {policy_id, created}

router::policy_update
  Input:  {policy_id, ...partial fields}
  Output: {updated policy}

router::policy_delete
  Input:  {policy_id}
  Output: {deleted: true}

router::policy_list
  Input:  {tenant?, enabled?}
  Output: {policies: [...]}

router::policy_test
  Input:  {prompt, tenant?, feature?, tags?}
  Output: {matched_policy, model, reason}
  Notes:  Dry-run. Shows which policy would match without recording anything.
```

### Complexity Classification

```
router::classify
  Input:  {prompt: string, classifier_id?: "default"}
  Output: {
    complexity: "simple"|"moderate"|"complex"|"expert",
    confidence: 0.87,
    suggested_model: "claude-haiku-4.5",
    reasoning: "Short factual question, no multi-step reasoning needed"
  }
  Notes:  Uses a cheap model (Haiku/Flash-Lite) as a shadow classifier.
          Adds ~200ms but saves $$ by avoiding frontier models for simple queries.
          Classifier prompt is stored in state and hot-swappable.

router::classifier_config
  Input:  {
    id: "default",
    model: "claude-haiku-4.5",   # the classifier model
    prompt_template: "...",       # the classification prompt
    thresholds: {
      simple: {max_score: 0.3, route_to: "gpt-4.1-nano"},
      moderate: {max_score: 0.7, route_to: "claude-haiku-4.5"},
      complex: {max_score: 0.9, route_to: "claude-sonnet-4"},
      expert: {min_score: 0.9, route_to: "claude-opus-4"}
    }
  }
  Output: {configured: true}
```

### A/B Testing

```
router::ab_create
  Input:  {
    name: "haiku-vs-flash-support",
    match: {tenant: "acme-corp", feature: "support-chat"},
    variants: [
      {model: "claude-haiku-4.5", weight: 50},
      {model: "gemini-2.5-flash", weight: 50}
    ],
    metric: "quality_score",     # what to optimize
    min_samples: 100,            # before declaring winner
    max_duration_days: 14
  }
  Output: {test_id, created}

router::ab_record
  Input:  {test_id, variant_model, quality_score: 0.0-1.0, latency_ms, cost_usd}
  Output: {recorded: true, total_samples}
  Notes:  Called after each request to record outcome quality.

router::ab_report
  Input:  {test_id}
  Output: {
    variants: [
      {model, samples, avg_quality, avg_latency_ms, avg_cost_usd, p95_latency},
      ...
    ],
    winner?: {model, confidence, improvement_pct},
    status: "running"|"concluded"|"insufficient_data"
  }

router::ab_conclude
  Input:  {test_id, winner_model}
  Output: {concluded: true, policy_updated: true}
  Notes:  Concludes test and updates the matching policy to use the winner model.
```

### Model Health

```
router::health_update
  Input:  {model, available: true|false, latency_p99_ms?, error_rate?}
  Output: {updated: true}
  Notes:  Called by the gateway or a health-check cron. router::decide skips unavailable models.

router::health_list
  Input:  {}
  Output: {models: [{model, available, latency_p99, error_rate, last_checked}]}
```

### Analytics

```
router::stats
  Input:  {tenant?, feature?, days?: 7}
  Output: {
    total_requests, total_cost_usd,
    by_model: {model → {count, cost_usd, avg_latency_ms}},
    by_policy: {policy_name → {count, cost_usd}},
    savings_vs_always_frontier_usd: 142.50
  }
  Notes:  The money stat. Shows how much the router saved vs always using the most expensive model.
```

## Triggers (20)

```
HTTP triggers (18):
POST /api/router/decide           → router::decide
POST /api/router/decide-batch     → router::decide_batch
POST /api/router/policy/create    → router::policy_create
POST /api/router/policy/update    → router::policy_update
POST /api/router/policy/delete    → router::policy_delete
GET  /api/router/policy/list      → router::policy_list
POST /api/router/policy/test      → router::policy_test
POST /api/router/classify         → router::classify
POST /api/router/classifier       → router::classifier_config
POST /api/router/ab/create        → router::ab_create
POST /api/router/ab/record        → router::ab_record
GET  /api/router/ab/:id/report    → router::ab_report
POST /api/router/ab/conclude      → router::ab_conclude
POST /api/router/health/update    → router::health_update
GET  /api/router/health/list      → router::health_list
GET  /api/router/stats            → router::stats

Cron triggers (2):
*/60 * * * * *  → router::health_check  (every 60s — ping providers, update health)
0 * * * *       → router::ab_evaluate   (hourly — check if any A/B test has enough samples to conclude)
```

## Example Flow

```bash
# 1. Create policies
curl -X POST localhost:3111/api/router/policy/create -d '{
  "name": "payments-frontier",
  "match": {"feature": "payments", "tags": ["p0"]},
  "action": {"model": "claude-opus-4", "fallback": "gpt-5"},
  "priority": 200
}'

curl -X POST localhost:3111/api/router/policy/create -d '{
  "name": "support-cheap",
  "match": {"feature": "support-chat"},
  "action": {"model": "auto", "fallback": "gpt-4.1-mini"},
  "priority": 100
}'

# 2. Configure the complexity classifier for "auto" policies
curl -X POST localhost:3111/api/router/classifier -d '{
  "id": "default",
  "model": "claude-haiku-4.5",
  "thresholds": {
    "simple": {"max_score": 0.3, "route_to": "gpt-4.1-nano"},
    "moderate": {"max_score": 0.7, "route_to": "claude-haiku-4.5"},
    "complex": {"max_score": 0.9, "route_to": "claude-sonnet-4"},
    "expert": {"min_score": 0.9, "route_to": "claude-opus-4"}
  }
}'

# 3. Before each LLM call, ask the router
curl -X POST localhost:3111/api/router/decide -d '{
  "tenant": "acme-corp",
  "feature": "support-chat",
  "prompt": "How do I reset my password?",
  "budget_remaining_usd": 45.00
}'
# → {"model": "gpt-4.1-nano", "reason": "classifier: simple (0.12), policy: support-cheap", "confidence": 0.95}

# 4. Start an A/B test
curl -X POST localhost:3111/api/router/ab/create -d '{
  "name": "haiku-vs-flash-support",
  "match": {"feature": "support-chat"},
  "variants": [
    {"model": "claude-haiku-4.5", "weight": 50},
    {"model": "gemini-2.5-flash", "weight": 50}
  ],
  "min_samples": 200,
  "max_duration_days": 7
}'

# 5. Check savings
curl localhost:3111/api/router/stats?tenant=acme-corp&days=7
# → {"savings_vs_always_frontier_usd": 142.50, "total_cost_usd": 23.80, ...}
```

## Integration with llm-budget

`router::decide` accepts `budget_remaining_usd` from `budget::check`. When the budget is tight:

1. Policy says "use opus" but budget has $0.50 left
2. Router downgrades to cheapest model that meets `min_quality`
3. Returns `reason: "budget constraint: downgraded from opus to haiku"`
4. Gateway records actual spend via `budget::record`

The budget worker enforces limits. The router worker respects them. Neither needs to know the other's internals — they communicate through the request payload.

## What This Is NOT

- Not a gateway. Use LiteLLM or Bifrost for proxying, caching, and failover.
- Not a model serving layer. Use Ollama, vLLM, or TGI for running local models.
- Not an observability platform. Use Langfuse, Helicone, or iii-engine's OTel module.
- Not a RouteLLM replacement. RouteLLM trains ML classifiers; this is a policy engine. They're complementary — you could use RouteLLM's classifier inside `router::classify`.
