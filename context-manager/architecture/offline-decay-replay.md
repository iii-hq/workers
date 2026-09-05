# Offline decay replay

This is a reproducible, opt-in comparison of assembled-history estimates. It
does not reconstruct provider requests or contact an engine, and it never
writes or prints transcript contents.

## Method

The ignored `context_replay` integration test accepts the input directory only
through `CONTEXT_REPLAY_DIR`. It parses JSONL records with `type: "entry"` and
`entry.kind: "message"`, retains the final revision for each message ID while
preserving its first-occurrence position, and rejects malformed records with
only a file and line reference. Metadata, leaf, and non-message entries are
not treated as conversation messages.

Each actual user message without an inline function-result block defines a
prefix endpoint. For every endpoint, the replay clones that deduplicated
prefix and calls the production `context::assemble` handler independently with
`decay_user_turns=0` and `decay_user_turns=4`. All other worker configuration
remains at its shipped default. The request supplies inline limits of
1,000,000 context tokens and one output token, no system prompt, tools, or
request overhead, and `allow_compaction=false`. The harness fails if the
pre-prune estimate exceeds the usable budget, preventing emergency reduction
or overflow from affecting the comparison.

It reports each session's final endpoint total and the mean over its final
`ceil(user_turns / 4)` endpoints, along with absolute and percentage savings.
Savings are signed, so an unexpected token increase is visible as a negative
saving. The command's output uses filenames and token totals only.

## Re-run

From the repository root:

```sh
CONTEXT_REPLAY_DIR=/home/anderson/.iii/data/session-manager \
  cargo test --manifest-path context-manager/Cargo.toml --test context_replay \
  replays_an_explicit_session_manager_directory -- --ignored --nocapture
```

The measured production range was base `629647d91447a63298d11bf1c37fb636fcec0254`
through `eca84b17b53dccb90f8b4bcb35b6edc894b4b720`. Re-run after any later
production change before comparing results.

## 2026-09-04 corpus result

Input inventory: 36 JSONL files, 10,487,859 bytes, and 463 deduplicated
message entries. It contained 49 actual user-turn endpoints: 33 sessions had
one endpoint, and the remaining sessions had 5, 5, and 6 endpoints.

All 36 sessions had zero decay savings. Aggregate final totals were 54,990
tokens with both settings (0 saved, 0.00%). Over the 39 last-quarter endpoints,
the totals were 98,381 tokens with both settings (0 saved, 0.00%); each mean
was 2,522.59 tokens. The three multi-turn sessions also had zero savings.
These totals were re-measured at the production revision above after adding
the age-only enclosing-message savings guard; the result remained zero.

This corpus is mostly single-turn and is not evidence about a long,
steady-state conversation. The synthetic long-history test uses 130 actual
user turns and 129 medium-sized (1,999-character) results with the shipped
protection, minimum-saving, and window guards intact; it verifies that the
same production assembler produces a strictly smaller final estimate with
decay four than with decay disabled.
