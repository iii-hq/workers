# Native Context fixtures for Harness generation capture

`native-context.json` holds one complete `context::assemble` input/output pair
(with `options.provenance`) and seven `render_system_prompt_with_span` outputs,
all produced by the real context-manager APIs with in-memory fakes. No engine,
provider, model, or private data is used.

The producer side checks it: the context-manager test
`origin_real_compaction_renderer_and_harness_fixture`
(`context-manager/tests/origin_binding.rs`) rebuilds the fixture and fails on
any byte of drift, so a change to `AssembleResponse` or the summary renderer
fails `cargo test` in context-manager. Review the producer change, then
regenerate from `context-manager/`:

```sh
UPDATE_GOLDENS=1 cargo test --test origin_binding
```

The assembly's random correlation `assembly_id` is the only normalized output
field; prompt bytes, SHA-256 hashes, byte spans, indices, budgets and messages
are produced by context-manager. Harness tests include the tracked bytes at
compile time and never run the producer.
