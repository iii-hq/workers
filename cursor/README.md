# cursor

Run Cursor coding agents through iii with durable local or cloud sessions, raw Cursor events on `cursor::events`, and normalized AgentEvent streaming on `agent::events`. This worker speaks Cursor's open `sdk.v1` Bridge protocol and does not bundle or redistribute Cursor's SDK or Bridge binary.

## Install

```bash
iii worker add cursor
```

## Quickstart

Set `CURSOR_API_KEY`, install the separately distributed Cursor SDK Bridge described below, then run a sandboxed local turn:

```bash
iii trigger cursor::run --payload '{
  "runtime": "local",
  "cwd": "/absolute/path/to/repository",
  "model": "composer-2",
  "prompt": "Summarize this repository and do not modify it."
}'
```

The response includes the durable `session_id`, Cursor `agent_id` and `run_id`, terminal result, status, stop reason, and only the usage or cost that Cursor reports. Reuse `session_id` to send a follow-up. `cursor::start` returns immediately, while `cursor::stop` requests cancellation.

`run::start_and_wait` is the standard alias for `cursor::run`. Its ACP-compatible request shape creates sandboxed local sessions from the editor's working directory. A model is required because Cursor's model catalog is dynamic:

```bash
iii-acp --brain-fn cursor::run --brain-stop-fn cursor::stop --model composer-2 --provider cursor
```

ACP forwards the editor `cwd`, maps cancellation to `cursor::stop`, and reports Cursor's top-level stop reason. Direct cloud turns use `cursor::run` with an explicit cloud payload instead.

## Configuration

The worker registers a built-in `cursor` configuration entry with the configuration worker. Set secrets through environment references, never literal committed values:

```yaml
api_key: ${CURSOR_API_KEY:}
bridge_binary: ${CURSOR_SDK_BRIDGE_BIN:}
workspace: .
startup_timeout_ms: 30000
shutdown_timeout_ms: 5000
rpc_timeout_ms: 60000
max_frame_bytes: 16777216
events_stream: agent::events
raw_events_stream: cursor::events
```

Download the standalone `cursor-sdk-bridge` archive for release `v1.0.28` from the [official Cursor SDK Bridge releases](https://github.com/cursor/sdk-bridge/releases/tag/v1.0.28), verify it against the release's `SHA256SUMS.txt`, and either put `bin/cursor-sdk-bridge` on `PATH` or set `CURSOR_SDK_BRIDGE_BIN` to its absolute path. The Bridge is a separate Cursor-distributed prerequisite governed by Cursor's terms; the iii worker release contains neither the Bridge binary nor `@cursor/sdk`.

Every remote function checks for a non-empty API key before launching the Bridge. The worker reads the Bridge's bearer token from its ready-line token file, accepts only a loopback HTTP endpoint, and never logs the token or API key.

For local sessions, the worker always enables the Cursor sandbox and limits built-in tools to `read`, `grep`, `glob`, and `ls` unless `tools` is explicitly supplied. `tools: []` creates a text-only agent. Cursor's sandbox is the execution boundary; review automation is not treated as a security boundary. Local sessions must resume with their original working directory and tool list so the Bridge uses the same durable store.

Cloud sessions require at least one repository and default both `work_on_current_branch` and `auto_create_pr` to `false`. Creating or sending a cloud agent may incur Cursor charges. Usage and cost fields remain `null` when the Bridge does not report them.

Session creation and Send ownership are claimed with `state::compare-and-set`, so overlapping worker processes cannot both dispatch one local prompt. A recent foreign claim returns busy. A stale local claim may be reclaimed only when its durable marker proves Send had not started; once Send may have started without yielding a run ID, the session becomes `recovery-required` and the worker refuses to replay it.

The Bridge protocol is versioned independently. This worker targets the `sdk.v1` contract shipped with Cursor SDK Bridge `v1.0.28` and validates required capabilities during its lazy startup. Model IDs are not hard-coded; call `cursor::models::list` to read the current catalog.

`cursor::run`, `cursor::start`, `cursor::stop`, status, session listing, model listing, and usage remain approval-gated by the repository's default policy because they can spend money, mutate repositories, or reveal account and workspace metadata. Only the internal configuration reload hook is denied outright.
