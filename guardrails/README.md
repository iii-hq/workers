# guardrails

Local heuristics for PII, leaked API keys, jailbreak keywords, and toxicity on
the iii bus. Registers `guardrails::check_input`, `guardrails::check_output`,
and `guardrails::classify`.

## Installation

```bash
iii worker add guardrails
```

## Run

```bash
iii-guardrails --engine-url ws://127.0.0.1:49134
```

All checks run in-process — no model calls, no network. Patterns and
keyword lists are baked into the binary at compile time.

## Registered functions

| Function | Description |
|---|---|
| `guardrails::check_input` | `{ text, rules? } → { allowed, reasons, redacted? }` |
| `guardrails::check_output` | Same shape; same ruleset (leaked keys + PII first-class on the output lane). |
| `guardrails::classify` | `{ text } → { pii, jailbreak, toxicity, keys_leaked }` |

## Build

```bash
cargo build --release
```
