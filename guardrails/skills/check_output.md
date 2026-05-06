# guardrails::check_output

Run heuristic guardrails on model or tool output before it is returned to a caller or stored.

`({ text, rules? }) → { allowed: bool, reasons: string[], redacted?: string }` —
identical input/output shape and rule set to `guardrails::check_input`. Scans for PII,
leaked API keys, jailbreak artefacts, and toxicity in the text produced by a model or
downstream function. Returns `allowed: true` when nothing is flagged; `reasons` lists
violation labels on failure. Pass `rules.redact: true` to receive a `redacted` copy with
findings replaced by `[REDACTED:<category>]` tokens.

## When to use

- Screen LLM responses for accidental PII leakage (e.g. a model regurgitating training data containing emails or credit card numbers).
- Detect leaked API keys or secrets in generated code or configuration snippets before surfacing them to the user.
- Enforce output-side content policy in an agentic pipeline where model outputs feed further tools or are persisted.
- Gate tool-call results that contain free-form text before passing them back into context.

## Notes

- Pure function — no model calls, no network I/O, fully deterministic. Safe to call on every response with negligible overhead.
- Shares the same default rule set as `check_input`: `pii: true, keys: true, jailbreak: true, toxicity_threshold: 0.02, redact: false`.
- Leaked-key detection (`rules.keys`) is especially valuable on the output lane where models may reproduce secrets from their context window.
- `rules.redact: true` produces a `redacted` field; the field is omitted when redaction is off.
- For input-side enforcement (user prompts, incoming payloads), use `guardrails::check_input`.
