# context-compaction

Out-of-band session-history compactor.

Subscribes to `agent::events::TurnEnd` on every assistant turn. When the
running token count for a session exceeds a configurable threshold, the
worker summarises the older portion of the transcript via
`router::stream_assistant` and appends a `Compaction` entry to the
session-tree. `session-tree::load_messages` filters Compaction entries out
of the active-path transcript, so the next assistant turn reads a
compressed history without any orchestrator changes — except a tiny
"reload-from-session-tree-when-a-fresh-Compaction-is-at-the-tail" check.

The worker has no LLM-facing tools (`tools: []`). It is invisible to the
model; the only artefact it produces is a side-effect on the session-tree.

## Configuration

| Env var | Default | What it does |
|---|---|---|
| `COMPACT_TRIGGER_TOKENS` | `60000` | Trigger when running `usage.input + output + cache_read` since the last Compaction crosses this many tokens. |
| `COMPACT_KEEP_RECENT_TURNS` | `3` | Number of trailing assistant/user turns kept verbatim. Older turns become the summary input. |
| `COMPACT_SUMMARIZER_PROVIDER` | (orchestrator's provider) | Cheap-model override for the summarisation call. |
| `COMPACT_SUMMARIZER_MODEL` | (orchestrator's model) | Cheap-model override. Default to a small model like `claude-haiku-4-5` to keep summarisation cost trivial. |

## How to disable

Either drop `context-compaction` from `harness/iii.worker.yaml` or set
`COMPACT_TRIGGER_TOKENS` to a number larger than any realistic session.
