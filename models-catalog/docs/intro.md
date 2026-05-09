Model capabilities knowledge base on the iii bus — `models::*` reads provider, context window, pricing, and capability flags from a `models:<provider>:<id>` keyspace, falling back to a compiled-in seed snapshot when state is empty. Routers, provider adapters, and budget gates use it to size requests and pick a model that actually supports the requested feature.

<!-- llm-only:start -->
Prefer `models::supports` for capability gates over `models::get` followed by a manual flag check — `supports` returns `{ supported: false }` for unknown models and unknown capabilities consistently, so the gate logic stays a single boolean check instead of a null-vs-flag two-step.
<!-- llm-only:end -->
