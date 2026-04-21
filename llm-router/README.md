# iii-llm-router

Policy-based LLM routing brain. **Unopinionated** — ships with zero built-in model names, zero hardcoded pricing, zero provider assumptions. Wraps any gateway (LiteLLM, Bifrost, OpenRouter, a local vLLM, your own proxy) by sitting *in front* of it: gateway asks `router::decide` before every call, router returns a model ID, gateway forwards.

## Why unopinionated matters

Every existing LLM router (RouteLLM, Portkey, LiteLLM's routing block) bakes a specific catalog of models and a specific rank ordering into the library. That catalog is wrong the day you read it — new models ship weekly, pricing moves, quality tiers shift. This worker doesn't know what "Opus" or "GPT" is. You register what you actually use at runtime. The only thing the router enforces is its own logic: match → classify → budget → health → fallback.

## Functions (18)

| id | shape |
|----|-------|
| `router::decide` | hot path — returns `{model, reason, policy_id?, ab_test_id?, fallback?, confidence, request_id}` |
| `router::policy_create` / `update` / `delete` / `list` / `test` | CRUD + dry-run |
| `router::classify` | run the prompt heuristic only; returns `{complexity, confidence, suggested_model}` (suggested_model respects your classifier map) |
| `router::classifier_config` | register `{id, thresholds: {simple/moderate/complex/expert → <your model id>}}` |
| `router::ab_create` / `ab_record` / `ab_report` / `ab_conclude` | A/B tests with weighted variants + quality/latency/cost aggregation |
| `router::health_update` / `health_list` | per-model availability + error rate; feeds fallback path |
| `router::model_register` / `model_unregister` / `model_list` | you tell the router what models exist; used only by the budget-downgrade path and stats |
| `router::stats` | usage by model, by policy, over a day window |

## HTTP triggers (18)

```
POST /api/router/decide
POST /api/router/policy/{create,update,delete,test}
GET  /api/router/policy/list
POST /api/router/classify
POST /api/router/classifier
POST /api/router/ab/{create,record,report,conclude}
POST /api/router/health/update
GET  /api/router/health/list
POST /api/router/model/{register,unregister}
GET  /api/router/model/list
GET  /api/router/stats
```

## Decide logic

```
match policies (by tenant, feature, tags) and pick highest priority
  if matching A/B test is running → sample a variant → return
  else if policy.action.model == "auto" → classify → look up user mapping
  if chosen model is unhealthy → use policy.fallback
  if policy.max_cost_per_request > budget_remaining → search registered
     models for a cheaper one meeting min_quality (if none: return original,
     flag reason)
  return {model, reason, policy_id, fallback, confidence}
no policy matched:
  if classifier exists → classify → map
  else → return empty model + reason (caller should handle)
```

Router **never** invents a model name. If you ask it to pick "auto" without a classifier registered, it tells you so in `reason` and returns the policy's fallback (or empty).

## State (engine-managed)

All stored in `state_scope: "llm-router"` (configurable).

```
policies:<id>         — policy definitions
ab_tests:<id>         — A/B test definitions
ab_events:<test>:…    — recorded outcomes
routing_log:<ts>:<id> — decision audit trail
model_health:<name>   — availability + latency + error_rate
classifier:<id>       — category → model mapping
models:<name>         — registered models (quality, pricing, provider)
```

## Example

```bash
# 1. register two models you actually use
curl -X POST localhost:3111/api/router/model/register -d '{
  "model": "gw/cheap-fast", "quality": "low",
  "input_per_1m": 0.1, "output_per_1m": 0.4
}'
curl -X POST localhost:3111/api/router/model/register -d '{
  "model": "gw/strong", "quality": "high",
  "input_per_1m": 15, "output_per_1m": 75
}'

# 2. configure the classifier (category → model is YOUR choice)
curl -X POST localhost:3111/api/router/classifier -d '{
  "id": "default",
  "thresholds": {
    "simple": "gw/cheap-fast",
    "moderate": "gw/cheap-fast",
    "complex": "gw/strong",
    "expert": "gw/strong"
  }
}'

# 3. write a policy
curl -X POST localhost:3111/api/router/policy/create -d '{
  "name": "support-auto",
  "match": { "feature": "support-chat" },
  "action": { "model": "auto", "fallback": "gw/cheap-fast" },
  "priority": 100
}'

# 4. ask before every call
curl -X POST localhost:3111/api/router/decide -d '{
  "feature": "support-chat",
  "prompt": "How do I reset my password?"
}'
# → {"model":"gw/cheap-fast", "reason":"policy: support-auto + classifier: simple", ...}
```

Your gateway (LiteLLM/Bifrost/OpenRouter/your-own) takes `model` and forwards. The router doesn't make any LLM call itself.

## What this is NOT

- Not a gateway — no LLM traffic passes through it, no API keys stored.
- Not an observability platform — `routing_log` is for audit, use iii's OTel for real telemetry.
- Not a training-based classifier — the shipped classifier is a cheap prompt heuristic (length, code markers, math markers). Swap it by calling `router::classifier_config` with your own mapping, or wrap a stronger classifier as a separate worker and call `router::decide` after you've called it.

## SDK + stack

- `iii-sdk 0.11.0` stable
- State via `state::get`/`set`/`delete`/`list` against scope `llm-router`
- `rand` for A/B variant weighted sampling
- `serde_json` everywhere — all state blobs are JSON

## Tests

17 passing — policy matching, priority ordering, A/B weighted sampling, classifier mapping, auto-without-classifier, unhealthy-fallback, budget-downgrade with and without registered models, health skip thresholds, heuristic category classification.
