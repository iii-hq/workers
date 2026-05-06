# guardrails::check_input

Run heuristic guardrails on agent or user input before it reaches a model or downstream function.

`({ text, rules? }) → { allowed: bool, reasons: string[], redacted?: string }` —
scans `text` for PII (email, phone, credit card, SSN), leaked API keys, jailbreak keywords,
and toxicity. Returns `allowed: true` when nothing is flagged. `reasons` is an empty array on
clean input and a list of violation labels (e.g. `"pii:email"`, `"jailbreak"`, `"toxicity: 0.150"`)
otherwise. Pass `rules.redact: true` to receive a `redacted` copy of the text with findings
replaced by `[REDACTED:<category>]` tokens.

## When to use

- Gate user-supplied prompts before forwarding them to an LLM to prevent prompt injection and PII leakage.
- Validate tool call arguments that carry free-form text fields originating from end users.
- Enforce input-side content policy (no jailbreak keywords, no embedded PII) in an agentic pipeline.
- Audit incoming webhook or API payloads for credential leakage before logging them.

## Notes

- Pure function — no model calls, no network I/O, fully deterministic. Safe to call on every request with negligible overhead.
- Default rule set: `pii: true, keys: true, jailbreak: true, toxicity_threshold: 0.02, redact: false`. Pass a partial `rules` object to override individual fields; unspecified fields keep their defaults.
- `rules.redact` is `false` by default; set it to `true` only when you need the sanitised copy — the `redacted` field is omitted entirely when redaction is off.
- Toxicity scoring is a simple term-frequency ratio; at the default threshold of `0.02` roughly two toxic tokens per hundred words triggers a flag.
- For output-side enforcement (model responses, tool results), use `guardrails::check_output`.
