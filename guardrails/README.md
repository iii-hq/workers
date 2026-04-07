# iii-guardrails

Every LLM call should pass through a safety check before and after. iii-guardrails does this with zero LLM overhead — pure regex and keyword matching, all patterns pre-compiled at startup. It detects PII (email, phone, SSN, credit cards, IP addresses), prompt injection attempts (9 keyword patterns), and leaked secrets (API keys, tokens, private keys). Wire it as middleware in front of any function, or call it on-demand from the agent.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-guardrails --url ws://your-engine:49134`. It registers 3 functions with 5 PII patterns and 7 secret patterns compiled from defaults — no config file needed. Call `guardrails::check_input` before processing user input, `guardrails::check_output` before returning responses, or `guardrails::classify` for a lightweight risk score.

## Functions

| Function ID | Description |
|---|---|
| `guardrails::check_input` | Validate input text for PII, injections, and length limits |
| `guardrails::check_output` | Validate output text for PII leakage and secret exposure |
| `guardrails::classify` | Lightweight risk classification without blocking or audit trail |

## iii Primitives Used

- **State** -- audit trail of checks, custom rules (future), aggregate stats (future)
- **PubSub** -- subscribes to `guardrails.check` topic for async input checks
- **HTTP** -- all functions exposed as POST endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-guardrails --url ws://127.0.0.1:49134 --config ./config.yaml
```

```
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
pii_patterns:
  - name: "email"
    pattern: "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}"
  - name: "phone"
    pattern: "\\b\\d{3}[-.]?\\d{3}[-.]?\\d{4}\\b"
  - name: "ssn"
    pattern: "\\b\\d{3}-\\d{2}-\\d{4}\\b"
  - name: "credit_card"
    pattern: "\\b\\d{4}[- ]?\\d{4}[- ]?\\d{4}[- ]?\\d{4}\\b"
  - name: "ip_address"
    pattern: "\\b\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\b"
injection_keywords:
  - "ignore previous instructions"
  - "ignore all instructions"
  - "disregard the above"
  - "you are now"
  - "pretend you are"
  - "act as if"
  - "system prompt"
  - "reveal your instructions"
  - "what are your rules"
max_input_length: 50000   # max input text length before flagging
max_output_length: 100000  # max output text length before flagging
```

## Tests

```bash
cargo test
```
