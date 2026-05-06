# models-catalog

Query and manage the model capabilities catalog on the iii bus.

- [`models-catalog`](iii://models-catalog)
  - [`models::list`](iii://models-catalog/list) — list all models, optionally filtered by provider or capability
  - [`models::get`](iii://models-catalog/get) — look up a single model by provider and model ID
  - [`models::supports`](iii://models-catalog/supports) — check whether a model supports a capability
  - [`models::register`](iii://models-catalog/register) — write a model entry to state

For LLM budget limits and per-model cost tracking, see [`llm-budget`](iii://llm-budget).
