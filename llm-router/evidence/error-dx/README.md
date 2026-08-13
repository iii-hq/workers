# LLM router error DX evidence

This bundle records the public failure contract, the operator recovery states,
and the verification performed for MOT-4414. The video is a captioned,
no-audio walkthrough generated from the checked-in presentation source.

## Assets

- [`error-dx-demo.webm`](./error-dx-demo.webm) — 1440×900 guided walkthrough.
- [`error-dx-poster.png`](./error-dx-poster.png) — recording poster and summary.
- [`presentation.html`](./presentation.html) — self-contained presentation.
- [`sample-failure.json`](./sample-failure.json) — structured failure example.
- [`sample-provider-status.json`](./sample-provider-status.json) — sanitized
  provider diagnostic example.
- [`record.mjs`](./record.mjs) — deterministic Playwright recorder.

Recorded artifact: 29.08 seconds, 1440×900, VP8 WebM, 25 fps, 2.4 MiB.
Its SHA-256 is
`16d80a2c1d1a70692b5047e2c83ca62ab9607914fbb1bb147867adb53b6f8588`.
The poster is a 1440×900 RGB PNG with SHA-256
`f24ec06d3a9877fcf4dbd0f97c577aade8417d0f0532269f96d71aa31660430d`.

## Verified behavior

| Surface | Evidence | Result |
| --- | --- | --- |
| Router core | `cargo test --manifest-path llm-router/Cargo.toml --lib` | 115 passed |
| Public schemas | `cargo test --manifest-path llm-router/Cargo.toml --test schemas` | 4 passed |
| Provider family | `cargo test --manifest-path <provider>/Cargo.toml --lib` for 11 providers | 820 passed |
| Harness consumer | `cargo test --manifest-path harness/Cargo.toml --lib` | 313 passed |
| Router configuration UI | `npm --prefix llm-router/ui test` | 5 passed |
| Router configuration bundle | `npm --prefix llm-router/ui run build` | passed |
| Context-manager consumer | `cargo check --manifest-path context-manager/Cargo.toml` | passed |
| Console consumer | `npm --prefix console/web run typecheck` | passed |
| Patch hygiene | `git diff --check` | passed |

The automated suites exercise 1,257 tests in total. The provider-family row
covers Anthropic, Claude Code, DeepSeek, GitHub Copilot, Kimi, llama.cpp,
OpenAI Codex, OpenAI, OpenRouter, xAI, and Z.AI.

## Contract assertions

- `failure_mode: "structured"` makes semantic pre-stream failures return an
  unsuccessful response instead of throwing.
- The terminal stream event and direct response carry the same `RouterFailure`.
- A failed stream emits one terminal error frame; retryability is explicit.
- Stable `router/*` codes are separate from human-readable messages.
- Public failure messages are single-line, bounded, and redact common secret
  shapes. Raw JSON, HTML, and upstream bodies are replaced with safe guidance.
- Provider diagnostics expose credential/catalog state, last safe failure,
  freshness, and model count without exposing credential material.
- Configuration validation blocks invalid timeouts, retry limits, regular
  expressions, provider references, URLs, and token limits before save.
- Harness and context-manager opt into structured failures and preserve the
  failure payload for recovery and transcript visibility.

## Reproduce the recording

From the repository root:

```sh
node llm-router/evidence/error-dx/record.mjs
```

The script uses the repository's Playwright dependency and writes the poster
and WebM next to the presentation. It removes only its private temporary
recording directory before each run.
