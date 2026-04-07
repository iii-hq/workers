# iii-guardrails

Safety layer worker for the III engine that checks function I/O for PII, injection attacks, jailbreaks, and content policy violations.

## Architecture

Pure regex + keyword matching. No LLM calls. Designed to be called on every function invocation as middleware.

## Functions

### `guardrails::check_input`
Validates input text before it reaches a function.

**Input:**
```json
{
  "text": "string (required)",
  "context": {
    "function_id": "string (optional)",
    "user_id": "string (optional)"
  }
}
```

**Output:**
```json
{
  "passed": true,
  "risk": "none|low|medium|high",
  "pii": [{ "pattern_name": "email", "count": 1 }],
  "injections": [{ "keyword": "ignore previous instructions", "position": 0 }],
  "over_length": false,
  "check_id": "chk-in-1712345678-42"
}
```

### `guardrails::check_output`
Validates output text for PII leakage and secret exposure.

**Input:**
```json
{
  "text": "string (required)",
  "context": {
    "function_id": "string (optional)",
    "user_id": "string (optional)"
  }
}
```

**Output:**
```json
{
  "passed": true,
  "risk": "none|low|medium|high",
  "pii": [{ "pattern_name": "ssn", "count": 1 }],
  "secrets": [{ "pattern_name": "openai_key", "count": 1 }],
  "over_length": false,
  "check_id": "chk-out-1712345678-42"
}
```

### `guardrails::classify`
Lightweight classification without blocking or audit trail.

**Input:**
```json
{
  "text": "string (required)"
}
```

**Output:**
```json
{
  "risk": "none|low|medium|high",
  "categories": ["pii", "injection", "secrets", "over_length"],
  "pii_types": ["email", "phone"],
  "details": {
    "pii_count": 2,
    "injection_count": 0,
    "secret_count": 0,
    "text_length": 150,
    "within_input_limit": true
  }
}
```

## Triggers

| Type | Path/Topic | Function |
|------|-----------|----------|
| HTTP POST | `guardrails/check_input` | `guardrails::check_input` |
| HTTP POST | `guardrails/check_output` | `guardrails::check_output` |
| HTTP POST | `guardrails/classify` | `guardrails::classify` |
| Subscribe | `guardrails.check` | `guardrails::check_input` |

## State Scopes

| Scope | Purpose |
|-------|---------|
| `guardrails:checks` | Audit trail of all checks performed |
| `guardrails:rules` | Custom rules (future: user-defined patterns) |
| `guardrails:stats` | Aggregate stats (future: checks/day, block rate) |

## Risk Classification

| Level | Condition |
|-------|-----------|
| `high` | Any injection keyword detected |
| `medium` | More than 2 PII matches OR over length limit |
| `low` | 1-2 PII matches |
| `none` | Clean |

## PII Patterns (default config)

- Email addresses
- US phone numbers
- Social Security Numbers
- Credit card numbers
- IP addresses

## Secret Patterns (hardcoded in check_output)

- Bearer tokens
- OpenAI API keys (`sk-`)
- GitHub PATs (`ghp_`, `ghs_`, `ghr_`)
- AWS access keys (`AKIA`)
- Private key blocks (`-----BEGIN`)

## Configuration

See `config.yaml` for default patterns, keywords, and length limits. All PII regex patterns are compiled once at startup and stored in `Arc` for zero-copy sharing across async handlers.

## Running

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

## Manifest

```bash
cargo run --release -- --manifest
```
