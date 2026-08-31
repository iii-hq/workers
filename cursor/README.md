# cursor

Use a normal Cursor login in two iii surfaces: a text-only LLM Router provider whose account models appear in the Console picker, and a coding-agent worker with durable local or cloud sessions. Local execution uses the official Cursor Agent CLI ACP. Cloud agent runs, or local agent runs explicitly configured for `sdk-bridge`, use Cursor's separately distributed `sdk.v1` Bridge.

No Cursor API key is required for the provider or local agent path. Both reuse the login created by `cursor-agent login`.

## Install

```bash
iii trigger compose::add worker=cursor
```

Install the official Cursor Agent CLI and authenticate it once outside the worker:

```bash
cursor-agent login
cursor-agent status --format json
```

Some installations expose the official CLI only as `~/.local/bin/agent`; use that absolute path for login when needed. The worker checks its configured `agent_binary`, `~/.local/bin/cursor-agent`, `~/.local/bin/agent`, and `cursor-agent` on `PATH`. It validates that the executable is Cursor before launching ACP, so an unrelated `agent` command is never selected from `PATH`. Set `CURSOR_AGENT_BIN` to an absolute path for any other installation.

Use the worker's redacted status function to confirm that the login is available without returning account details or credentials:

```bash
iii trigger cursor::auth::status --json '{}'
```

## LLM Router provider

On startup, the worker declares provider `cursor` and reconciles the ACP-selectable account catalog as `cursor/*` model IDs. The models then appear under **Cursor** in the Console model picker. Refresh explicitly with:

```bash
iii trigger provider::cursor::refresh_models --json '{}'
```

The provider runs each request in Cursor `ask` mode inside a fresh empty temporary workspace and denies permission requests. Cursor exposes a coding-agent ACP rather than a raw chat-completions API, so this compatibility provider streams text and thinking but does not claim router tool calling, images, structured output, exact token limits, token usage, or per-token cost. Those catalog capabilities are marked unsupported instead of invented. Use `cursor::run` when Cursor should work directly on a repository.

## Quickstart

Read the account-scoped model catalog, then run a direct coding-agent turn without `CURSOR_API_KEY`:

```bash
iii trigger cursor::models::list --json '{}'

iii trigger cursor::run --json '{
  "runtime": "local",
  "cwd": "/absolute/path/to/repository",
  "model": "auto",
  "prompt": "Summarize this repository and do not modify it."
}'
```

The response includes the durable `session_id`, Cursor session ID, worker run ID, terminal result, status, and stop reason. Login-backed ACP does not report usage or cost, so those fields remain `null`. Reuse `session_id` for a follow-up. `cursor::start` returns immediately, while `cursor::stop` requests ACP cancellation.

The login-backed catalog comes from a fresh ACP session and includes only model IDs ACP accepts. The public `auto` ID maps to ACP's `default` model. Do not copy parameterized IDs from `cursor-agent --list-models`; that command exposes additional CLI-only IDs that ACP rejects. Use `cursor::models::list` for worker requests.

`run::start_and_wait` is the standard alias for `cursor::run`:

```bash
iii-acp --brain-fn cursor::run --brain-stop-fn cursor::stop --model auto --provider cursor
```

ACP forwards the editor `cwd`, maps cancellation to `cursor::stop`, and reports Cursor's top-level stop reason.

## Configuration

The built-in `cursor` configuration defaults local execution to login-backed ACP. The API key and Bridge binary are optional unless cloud mode or the explicit Bridge backend is used:

```yaml
local_backend: cli-acp
agent_binary: ${CURSOR_AGENT_BIN:}
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

Login-backed local sessions run in Cursor's `ask` mode, and the worker cancels every permission request. Cursor ACP does not provide an enforceable per-request tool-list control, so explicit `tools` values, including `tools: []`, are rejected. Omit `tools` for CLI ACP. Start a new session when changing backend; legacy Bridge session IDs are never loaded as CLI ACP sessions.

Per-turn `timeout_ms` is supported only by login-backed `cli-acp` sessions. Local `sdk-bridge` sessions reject that field because the Bridge cannot safely cancel a timed-out Send until it has reported a run ID. Omit it and use `cursor::stop` while the owning Bridge process is still live. If a local Bridge Send becomes `recovery-required` before reporting a run ID, the worker reports that cancellation cannot be confirmed and never claims the upstream run was stopped.

Session creation and prompt ownership are claimed with `state::compare-and-set`, so overlapping worker processes cannot both dispatch one prompt. If a worker loses the CLI process after prompt dispatch has started, the durable session becomes `recovery-required`; the worker does not replay that prompt automatically.

## SDK Bridge and cloud mode

Download the standalone `cursor-sdk-bridge` archive for release `v1.0.28` from the [official Cursor SDK Bridge releases](https://github.com/cursor/sdk-bridge/releases/tag/v1.0.28), verify it against the release's `SHA256SUMS.txt`, and either put `bin/cursor-sdk-bridge` on `PATH` or set `CURSOR_SDK_BRIDGE_BIN` to its absolute path. The iii worker does not bundle or redistribute the Bridge binary or `@cursor/sdk`.

Set `CURSOR_API_KEY` for cloud runs or configure `local_backend: sdk-bridge` to retain the earlier local Bridge behavior. The worker reads the Bridge bearer token from its ready-line token file, accepts only a loopback HTTP endpoint, and never logs that token or the API key.

Select the Bridge catalog when choosing a model for a cloud run:

```bash
iii trigger cursor::models::list --json '{"backend":"sdk-bridge"}'
```

Cloud sessions require at least one repository and default both `work_on_current_branch` and `auto_create_pr` to `false`. Creating or sending a cloud agent may incur Cursor charges. Usage and cost fields remain `null` when the Bridge does not report them.

The Bridge protocol is versioned independently. This worker targets the `sdk.v1` contract shipped with Cursor SDK Bridge `v1.0.28` and validates required capabilities during lazy startup. Model IDs are not hard-coded; `cursor::models::list` reads the catalog for the selected backend.

Read-only `cursor::status`, `cursor::sessions::list`, `cursor::auth::status`, and `cursor::models::list` are allowed by the default worker policy. `cursor::run`, `cursor::start`, `cursor::stop`, and `cursor::usage` remain approval-gated because they can spend money or mutate repositories. The internal configuration reload hook is denied outright.
