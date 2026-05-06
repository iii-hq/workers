# models::get

Look up a single model by provider and model ID.

`({ provider, model_id }) → Model | null` — checks iii state under `models:<provider>:<model_id>` first; falls back to the compiled-in seed if the key is absent. Returns `null` when the model is not found in either source.

## When to use

- Fetching full model metadata (context window, pricing, capabilities) before constructing a request.
- Verifying a model exists before passing it to a provider adapter.
- Retrieving thinking budget tiers or transport preferences for a known model.

## Notes

- Both `provider` and `model_id` are required; the call returns an error if either is missing.
- The returned object includes all `Model` fields: `id`, `provider`, `api`, `display_name`, `context_window`, `max_output_tokens`, `supports_thinking`, `supports_xhigh`, `supports_tools`, `supports_vision`, `supports_cache`, `thinking_budgets`, `transports`, `default_cache_retention`, and `pricing`.
- State-first: a model registered via `models::register` shadows the embedded seed entry with the same key.
