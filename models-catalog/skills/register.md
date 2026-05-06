# models::register

Write a model entry to iii state, making it available to `models::list`, `models::get`, and `models::supports`.

`(Model) → { key: string, registered: true }` — persists the model under `models:<provider>:<id>` in the `models` scope. Returns the state key on success. Returns an error if the payload does not conform to the `Model` schema.

## When to use

- Adding a new model to the live catalog without restarting or recompiling this worker.
- Overriding an embedded seed entry with updated metadata (pricing, context window, new capability flags).
- Syncing a provider's model list from a registry-sync worker or CLI tool.

## Notes

- The full `Model` object is required; required fields are `id`, `provider`, `api`, `display_name`, and `context_window`.
- The embedded `data/models.json` is seeded into state once at startup when state is empty; models written via `models::register` win over the seed and are never overwritten by subsequent boots.
- Writes are scoped to `models` — they persist across worker restarts as long as the iii bus state store is durable.
- To remove a model, use `state::delete` with scope `models` and key `models:<provider>:<id>`.
