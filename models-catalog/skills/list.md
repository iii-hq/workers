# models::list

List all known models from the bus, optionally filtered by provider or capability.

`({ provider?, capability? }) → { models: [Model] }` — queries iii state under the `models:` prefix. Falls back to the compiled-in `data/models.json` seed when no `models:` keys are registered in state. Both `provider` and `capability` filters may be combined.

## When to use

- Discovering which models are available before selecting one for a task.
- Filtering to a specific provider (e.g. `"anthropic"`, `"openai"`) to enumerate its models.
- Finding all models that support a given capability before routing a request.

## Notes

- `capability` is a string; accepted values: `"thinking"`, `"thinking:low"`, `"thinking:medium"`, `"thinking:high"`, `"thinking:xhigh"`, `"tools"`, `"vision"`, `"cache"`.
- Results are state-first: models written via `models::register` appear immediately; the embedded seed is only consulted when state is empty.
- Omit both fields to return the full catalog.
