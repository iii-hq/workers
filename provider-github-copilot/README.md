# provider-github-copilot

GitHub Copilot subscription provider worker behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router): sign in with GitHub once and the
models the subscription grants appear in the model picker. The wire is
OpenAI Chat Completions against the Copilot API endpoint; what makes this
provider different from the api_key providers is the credential lifecycle,
which it owns end to end.

Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::github-copilot::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::github-copilot::refresh_models` (live `GET /models`, admitted rows
mapped to catalog records → `router::models::reconcile`), plus a device-flow
sign-in surface (`login::start` / `login::poll`).

## Signing in

```
iii trigger provider::github-copilot::login::start
# → { user_code, verification_uri, device_code, interval }
# enter the code at the verification URL, then:
iii trigger provider::github-copilot::login::poll device_code=<device_code>
# → { status: "ok" } — the catalog fills within seconds
```

Machines already signed in through an editor need no login at all: the worker
imports (read-only) `~/.config/github-copilot/apps.json` or pi's auth store.
`GITHUB_COPILOT_NO_LOCAL_IMPORT=1` opts out of that import;
`GITHUB_COPILOT_OAUTH_TOKEN` supplies the GitHub OAuth token directly; and
`GITHUB_COPILOT_TOKEN` supplies a ready Copilot bearer (tests, short-lived
dev sessions).

## Behavior

- **Credential lifecycle:** the long-lived GitHub OAuth token (login, env,
  or editor import) is exchanged at `copilot_internal/v2/token` for a
  short-lived Copilot bearer (~25 minutes) that also names the API endpoint.
  The bearer is cached in-memory, refreshed proactively inside a 2-minute
  margin, and invalidated when a stream dies with an auth error — the next
  call re-exchanges instead of failing again. The GitHub token persists in
  iii-state (scope `provider-github-copilot`, key `oauth_token`).
- **Catalog ids are prefixed:** Copilot serves several vendors' models under
  bare ids (`gpt-5.2`, `claude-sonnet-4.6`) that would collide with the
  sibling single-vendor providers. Catalog ids are `copilot/<id>`; the
  prefix is stripped on every upstream call.
- **Admission:** a listing row must be `type: chat`, support `tool_calls`,
  carry a non-`disabled` `policy.state`, and declare `/chat/completions`
  among its endpoints. The editor's internal feature models (preview rows
  with no picker category — search, compaction, exec agents) are dropped
  too. Windows, ceilings, and capability flags come from the listing's
  `capabilities` tree; there is no pricing — a subscription meters in
  premium requests, so records carry no per-token cost and
  `usage.cost_usd` stays unset.
- **Verified, not guessed:** which models a plan may actually call is *not*
  in the listing. Two models can be identical across every field the API
  exposes — same vendor, same `policy: enabled`, same picker category — and
  one answers while the other returns `model_not_supported` (a free or
  educational plan carries the base families but no premium requests).
  Discovery therefore probes each admitted model with a one-token request,
  four at a time, and reconciles only what answered. Refusals are rejected
  before generation so they consume no quota, and successes cost a single
  token on models the plan already includes. Enabling more models upstream
  needs no code change — the next refresh picks them up.
- **Self-healing:** if a model refuses between refreshes, the stream returns
  an actionable permanent error and the row is removed from the catalog, so
  the picker never offers the same dead model twice.
- **`model_picker_enabled` is deliberately ignored.** It reflects an
  editor-side picker preference and reads `false` for every row on accounts
  that have never toggled models in an editor — gating on it admits nothing.
- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no static `models` slice; a refresh fires right
  after registration and after every successful login.
- **Client identity:** the token exchange, discovery, and every chat call
  carry the integration headers the Copilot gateway requires
  (`Copilot-Integration-Id`, `Editor-Version`, plugin version, user agent),
  plus `X-Initiator: agent` so agent-initiated turns are billed per
  Copilot's convention and never misattributed as user keystrokes.
- **Reasoning:** the wire has no reasoning-effort parameter — thinking
  models decide for themselves and stream reasoning back as `reasoning`
  deltas (surfaced as thinking blocks; the older `reasoning_content` field
  from compatible gateways is honored too). A requested `thinking_level` is
  reported as ignored via a report-and-continue warning.
- **Structured output:** strict `json_schema` mode on models whose listing
  declares `structured_outputs`; a schema requested for any other model
  degrades to `json_object` with a warning.
- **Token counting:** none. The Copilot API exposes no tokenizer endpoint
  and its models span several vendors' vocabularies — `router::count_tokens`
  reports `no_token_counter` for this provider and the harness falls back
  to its own estimate.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** statuses carry subscription semantics on the shared taxonomy:
  401 (bearer or login died) → `auth_expired` and the cached bearer is
  dropped; 403 (no Copilot access / model not authorized) → `permanent`;
  429 → `rate_limited`; 5xx and network failures → `transient`;
  context-length errors → `context_overflow`. The numeric `error.code`
  envelope wins over the transport status. No transport retries here — the
  router owns retry policy.
- **api_url precedence:** operator override in the `llm-router` entry →
  the endpoint the exchange reply names (GitHub Enterprise tenants land
  here automatically) → the public default.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream; the ready-bearer env path short-circuits
the token exchange so no external API is called. A suite-wide lock
serializes the tests (they manage credential env vars), so plain
`cargo test` is safe too.

## Running

The binary takes the standard worker CLI flags: `--url` (engine WebSocket,
default `ws://127.0.0.1:49134`, falls back to the `III_WS_URL` environment
variable), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).
