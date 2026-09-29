# Investigate: UnknownOperation: unknown compose operation `<str>`

- group: `grp_0199f1a2b3c44d5e8f9a0b1c2d3e4f50`
- fingerprint: `6a1f0c2d9e4b7a83c5d0e1f2a3b4c5d6`
- worker: `compose` · function `compose::operation`
- state: new · 7 occurrence(s) · 3 session(s) affected
- first seen 2026-09-24T12:34:56.789Z · last seen 2026-09-25T12:34:56.789Z
- this occurrence: `occ_0199f1a2b3c44d5e8f9a0b1c2d3e4f51` at 2026-09-25T12:34:56.789Z · worker version 0.24.0
- checkout you are reading: git:abc1234 — it may differ from the version that failed; say so if it matters

Message as captured:

```
unknown compose operation `<str>`
```

## Evidence — trace `9f2b7c1d4e5a6b8c9d0e1f2a3b4c5d6e`

### Where it failed

**`execute compose::operation`**

- function: `compose::operation`
- service: `compose`
- status: error — unknown compose operation `up`

**event `exception`**

- `exception.message`:

```
unknown compose operation `up`
```
- `exception.type`:

```
UnknownOperation
```

**attributes**

- `function_id` = `compose::operation`
- `iii.namespace` = `my-project`

### Carried upward

- `execute harness::turn` — error: a child span failed

### Other failures in the same trace

- `call state::get` (`state::get`) — key not found

### Logs of this trace

```
ERROR compose::operation failed: unknown compose operation `up`
```

### Trace tags

- `iii.session.id` = `s_9f2b7c1d`

## Earlier occurrences

- 2026-09-25T11:34:56.789Z · version 0.24.0 · unknown compose operation `down`
- 2026-09-25T10:34:56.789Z · version 0.23.1 · unknown compose operation `restart`

## Where the code lives

The code of `compose` is most likely under `/home/dev/workers/compose/`. The whole checkout at `/home/dev/workers` is readable with the `coder::*` functions; nothing else on this machine is.

## Recording what you find

Call `sentinel::diagnosis::record` with `group_id: "grp_0199f1a2b3c44d5e8f9a0b1c2d3e4f50"` and a diagnosis. Record again whenever the conclusion changes. If the evidence runs out first, record with `confidence: "low"` and list what was missing.

The live engine is reachable through `sentinel::trace::get` and `sentinel::logs::list` if the trace is still there; the frozen bundle is `sentinel::evidence::get` with `occurrence_id: "occ_0199f1a2b3c44d5e8f9a0b1c2d3e4f51"`.
