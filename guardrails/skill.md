# guardrails

Heuristic content-safety layer: detect and redact PII, leaked API keys, jailbreak keywords, and toxic language in agent inputs and outputs.

- [`guardrails`](iii://guardrails)
  - [`guardrails::check_input`](iii://guardrails/check_input) — screen incoming text and block or redact policy violations
  - [`guardrails::check_output`](iii://guardrails/check_output) — screen model or tool output before it reaches the caller
  - [`guardrails::classify`](iii://guardrails/classify) — per-category boolean + toxicity score without a pass/fail verdict

All three functions are pure heuristics (no model calls, no network I/O) and are safe to run on every request. For provider-side content moderation (model-level filters), consult the relevant LLM provider worker.
