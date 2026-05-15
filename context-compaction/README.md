# context-compaction

Out-of-band session-history compactor. Subscribes to `agent::events` on the
iii bus, watches every `TurnEnd` event for the per-turn token count, and
when the running transcript crosses the configured threshold, summarises
the older turns via `router::stream_assistant` and writes a
`SessionEntry::Compaction` to the session-tree.

The orchestrator's next turn picks up the compacted transcript via
`session-tree::load_messages` (which already filters `Compaction` entries
out of the active path). No LLM-facing tools. Invisible to the model — the
only observable artefact is one extra entry in the session-tree per
compaction.

## Why

Long sessions ship the full conversation on every request because the
Anthropic / OpenAI APIs are stateless. Shipping a compacted prefix instead
of the raw transcript trades one cheap summariser LLM call for ongoing
per-turn savings against the larger, slower main-loop model. The
heuristic for "long enough to compact" is one env-tunable threshold;
defaults are conservative.

## Install

```bash
make compaction          # spawn alongside the harness engine
```

The worker is intentionally not in `harness/iii.worker.yaml` `dependencies:`
— the upstream registry doesn't index it yet, so it ships as a background
process the harness `Makefile` starts after `engine` is up. Mirrors the
`iii-observability` "optional, side-of-config" pattern. PID file lives at
`$PIDS_DIR/context-compaction.pid`; logs at `$LOGS_DIR/context-compaction.log`.

## Configuration

All knobs are env vars; defaults are baked in.

| Variable | Default | Effect |
|---|---|---|
| `COMPACT_TRIGGER_TOKENS` | `60000` | Token threshold above which `TurnEnd` triggers compaction |
| `COMPACT_KEEP_RECENT_TURNS` | `3` | Number of trailing turns kept verbatim in the post-compaction transcript |
| `COMPACT_SUMMARIZER_PROVIDER` | `anthropic` | Provider for the summariser LLM call |
| `COMPACT_SUMMARIZER_MODEL` | `claude-haiku-4-5` | Model for the summariser LLM call (use a cheap fast model) |

`usage_total` for the threshold check sums `input + output + cache_read` so
the trigger fires on *true* transcript size, not on what happens to be
cache-hot. `cache_write` is excluded — it costs more per turn but doesn't
grow the transcript.

## Coordination

Single-writer correctness across multiple worker instances or rapid-fire
events is enforced via a nonce-and-readback lease at
`session/<id>/compaction_lease`. The engine's `state::*` ops have no CAS
primitive, so each acquisition writes a unique nonce and confirms
ownership via readback (`state::set` is last-write-wins; exactly one
writer sees its own nonce survive). Lease TTL is 300s — comfortably
above the 120s summariser timeout so a slow LLM call can't expire its
own lease and let a peer start a duplicate compaction.

When a compaction lands, the worker stamps
`session/<id>/last_compaction_at` with `chrono::Utc::now().timestamp_millis()`.
The orchestrator watches that key and rebuilds its hot
`session/<id>/messages` view from session-tree on the next
`handle_streaming` entry — no synchronous coupling, no shared writers.

## Testing

```bash
cargo test                                  # unit tests
cargo test --test manifest                  # manifest CLI smoke test
```

Pure logic (lease nonces, timestamp parsing, threshold policy, summary
rendering) is covered by the unit suite. End-to-end orchestration paths
(`acquire_lease`, `summarize_and_append`, `handle_event`) require a live
iii engine and aren't unit-tested today; live exercise is documented in
the implementation plan.
