# models::supports

Check whether a specific model supports a given capability.

`({ provider, model_id, capability }) → { supported: bool }` — resolves the model from state (falling back to the embedded seed) and evaluates the capability flag. Returns `{ supported: false }` when the model is not found.

## When to use

- Gating a feature (e.g. extended thinking, vision input) on whether the selected model supports it.
- Choosing between `thinking:high` and `thinking:xhigh` budget tiers before starting a reasoning task.
- Deciding at routing time whether to enable prompt caching for a given model.

## Notes

- All three fields (`provider`, `model_id`, `capability`) are required; missing or unknown `capability` returns an error.
- `capability` accepted values: `"thinking"`, `"thinking:low"`, `"thinking:medium"`, `"thinking:high"`, `"thinking:xhigh"`, `"tools"`, `"vision"`, `"cache"`.
- `"thinking"` tests the base `supports_thinking` flag; `"thinking:xhigh"` tests the stricter `supports_xhigh` flag (which implies `supports_thinking`).
- State-first: a model entry written via `models::register` takes precedence over the compiled-in seed.
