# guardrails::classify

Return a boolean + score breakdown per risk category for a piece of text without making a pass/fail decision.

`({ text }) → { pii: bool, keys_leaked: bool, jailbreak: bool, toxicity: number }` —
scans `text` and returns one boolean per category (`pii`, `keys_leaked`, `jailbreak`) and a
numeric toxicity ratio (`toxicity`). The toxicity value is a floating-point term-frequency
ratio (toxic terms / total tokens); a value of `0.0` means no toxic terms were found.
No redaction is performed and no `rules` object is accepted — this function always runs all
four detectors and leaves the threshold judgement to the caller.

## When to use

- Build a content dashboard or audit log that needs per-category signals rather than a single allow/block verdict.
- Route requests to different handling paths based on the presence of specific risk categories (e.g. quarantine PII, escalate jailbreak attempts separately).
- Compute a risk profile for a piece of text before deciding which of `check_input` or `check_output` to invoke with custom thresholds.
- Feed classification signals into a policy engine or analytics pipeline without coupling it to guardrails' built-in thresholds.

## Notes

- Pure function — no model calls, no network I/O, fully deterministic. Safe to call on every request.
- Unlike `check_input` / `check_output`, `classify` accepts no `rules` parameter and always runs all detectors.
- `toxicity` is a ratio, not a boolean; callers choose their own threshold. The default threshold used by `check_input` / `check_output` is `0.02`.
- `pii` is `true` if any PII pattern matches (email, phone, credit card, SSN). Use `check_input` with `rules.redact: true` to obtain the redacted text.
- `keys_leaked` covers common API key patterns (OpenAI, Anthropic, AWS, etc.).
