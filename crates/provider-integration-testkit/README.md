# Provider integration testkit

Hermetic contract coverage for the real iii engine, `llm-router`, and provider
implementations. Vendor HTTP and SSE traffic terminates at a loopback stub; no
real API key or provider network access is used.

The contract is ignored by ordinary `cargo test` because it needs engine and
state worker binaries. Each run starts isolated processes with temporary,
in-memory state.
Run one provider explicitly:

```bash
III_ENGINE_BIN=/path/to/iii \
III_STATE_BIN=/path/to/state \
  cargo test --manifest-path crates/provider-integration-testkit/Cargo.toml \
  --features provider-openai tests::provider_contract -- --ignored --exact --nocapture
```

CI selects the affected feature. Changes to this testkit or `llm-router` fan
out to every supported provider.

The Codex login tests use the real provider, router, private state store, and
engine. A deterministic OAuth transport supplies one synthetic account; an
empty legacy source prevents local credentials from affecting the result.
They cover login RPCs, durable session records, automatic catalog discovery,
authenticated streaming, logout without fallback, cancellation during polling,
public state read isolation, and suppressed private state events. The event
listener is verified with public writes before login and after logout.
`state::get_group` is also checked: either private-scope denial or the retired
endpoint being absent is accepted. Native OAuth HTTP wire coverage lives in
the provider's `oauth_tests.rs`.

Run the login tests and existing Codex provider contract together:

```bash
III_ENGINE_BIN=/path/to/iii \
III_STATE_BIN=/path/to/state \
  cargo test --manifest-path crates/provider-integration-testkit/Cargo.toml \
  --features provider-openai-codex --lib -- --include-ignored --test-threads=1
```

Codex's test worker uses the production name `provider-openai-codex` because
private namespace claims authenticate the worker name. Other providers retain
their unique test names. Failure diagnostics never render credential records
or raw authenticated requests.
