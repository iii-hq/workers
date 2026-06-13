# Harness storage: provider credentials & settings

Provider API keys and per-provider settings live in a single entry — id
`harness` — in the engine's built-in
[`configuration`](https://github.com/iii-ai/iii/tree/main/workers/configuration)
worker. The harness meta-worker owns that entry through its **provider
registry** ([src/harness/providers/](../src/harness/providers/)).

This replaces the former `database`-backed `auth-credentials` and
`provider-config` workers — the harness no longer depends on the `database`
worker.

## The `harness` configuration entry

```jsonc
{
  "providers": {
    "anthropic": { "api_key": "sk-ant-…", "api_url": "https://…", "max_tokens": 8192 },
    "openai":    { "api_key": "sk-…" },
    "lmstudio":  { "max_tokens": 8192 }
  }
}
```

The JSON Schema for `providers` is **composed dynamically**: each provider
worker self-declares its slice at startup via `harness::provider::register`,
so the editable shape grows/shrinks with the set of running providers. The
console renders this entry with its schema-driven configuration form.

A configured `max_tokens` is an explicit override: it wins over the
per-model default (`min(catalog max_output_tokens, 32_000)` — see
[src/runtime/output-tokens.ts](../src/runtime/output-tokens.ts)) but is
clamped down to the model's catalog ceiling when known, so a too-large
value can't produce upstream 400s. The 32k default cap is overridable via
the `HARNESS_OUTPUT_TOKEN_MAX` env var.

> Secrets are stored as plaintext in the configuration value. Agents are
> denied `configuration::get`/`set` and `harness::provider::resolve` in
> [iii-permissions.yaml](../../iii-permissions.yaml); the console edits the
> entry as a user-initiated SDK call, which bypasses the agent gate.

## Bus surface

| Function | Caller | Purpose |
|---|---|---|
| `harness::provider::register` | provider workers (startup) | Declare id + config schema + defaults; recomposes and re-registers the `harness` entry. |
| `harness::provider::resolve` | provider workers (per request) | Resolve `{ credential, api_url, max_tokens }`. Falls back to the provider's declared `credential_env_var` when no `api_key` is configured. |
| `harness::provider::list` | console | Enumerate declared providers. |

Provider workers never read keys from disk or env directly for cloud
providers — they call `harness::provider::resolve`. The env-var fallback is
applied centrally by the registry so existing `ANTHROPIC_API_KEY`-style
setups keep working.

## Permissions

Deployment approval defaults (`default_mode`, `always_allow_seed`,
`pending_timeout_ms`) live in the standalone approval-gate worker's own
`approval-gate` configuration entry — the harness entry no longer carries a
permissions block. See
[tech-specs/2026-06-agentic/approval-gate.md § Configuration](../../tech-specs/2026-06-agentic/approval-gate.md#configuration).

## Adding a provider

1. In the provider's `register.ts`, call `declareProvider(iii, { id, credential_env_var, defaults, supports_model_listing })` (see [src/runtime/provider-resolve.ts](../src/runtime/provider-resolve.ts)).
2. In `buildConfig`, call `resolveProvider(iii, id)` and use the returned credential + `api_url`/`max_tokens` (falling back to the worker's config.yaml defaults).

No database pool, table, or `storage:` block is required.
