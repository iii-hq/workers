# Provider integration testkit

Hermetic contract coverage for the real iii engine, `llm-router`, and provider
implementations. Vendor HTTP and SSE traffic terminates at a loopback stub; no
real API key or provider network access is used.

The contract is ignored by ordinary `cargo test` because it needs an engine.
Run one provider explicitly:

```bash
III_ENGINE_BIN=/path/to/iii \
  cargo test --manifest-path crates/provider-integration-testkit/Cargo.toml \
  --features provider-openai tests::provider_contract -- --ignored --exact --nocapture
```

CI selects the affected feature. Changes to this testkit or `llm-router` fan
out to every supported provider.
