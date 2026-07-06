# devin

[Devin](https://docs.devin.ai) as an iii worker: the Devin coding agent exposed as functions and streams on the iii bus. Devin has two faces, and this worker exposes both. The local [`devin` CLI](https://docs.devin.ai/cli) drives one headless turn through `devin::run`, streaming its output onto the bus. The [Devin REST API](https://docs.devin.ai/api-reference/overview) drives the cloud agent: `devin::session::*` wrap the session lifecycle, and `devin::api` reaches any v3 endpoint the typed wrappers do not cover.

This worker is deliberately thin. It does not re-implement scheduling, sub-agents, or persistence that the engine already provides. Schedule a Devin session with the `cron` worker, fan Devin runs out with `harness::spawn`, and let the engine trace and persist every call. The worker's job is to put Devin on the bus, nothing more.

## Install

```bash
iii worker add devin
```

For the API surface (`devin::session::*`, `devin::api`), set `DEVIN_API_KEY` in the worker environment. Get a key from the [Devin settings page](https://app.devin.ai/settings/api-keys).

For the CLI surface (`devin::run` / `devin::start`), install the local [Devin CLI](https://cli.devin.ai/docs) (`brew install --cask devin-cli`, or `curl -fsSL https://cli.devin.ai/install.sh | bash`) and run `devin auth login`. This is the local coding agent (SWE-1.6), distinct from the Devin cloud the API targets: the worker drives it in non-interactive `--print` mode, so the agent runs on the host, works with the local files under `cwd`, and returns its reply in `result`. Because it runs locally, an `iii_context` turn can reach the engine at `localhost` with no exposure.

## Skills

Install the `devin` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill devin
```

## Quickstart

From zero to a Devin cloud session over the bus:

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
export DEVIN_API_KEY=...       # personal token (v1) or service key (cog_..., v3)
# service-key users only: also `export DEVIN_ORG_ID=org_...` and set base_url to v3
iii worker add devin
iii   # starts the engine + worker
```

The API has two shapes and the worker picks one from your config. A **personal token** uses the flat v1 API and is the default (leave `org_id` empty, `base_url` stays `.../v1`). A **service key** (`cog_...`) uses the v3 API scoped to an organization: set `org_id` and `base_url` to `https://api.devin.ai/v3`. The `devin::pr-review::*` and `devin::code-scan::*` functions are v3/enterprise features.

Start a cloud session and read it back:

```bash
# start an autonomous Devin session in the cloud
iii trigger devin::session::create --timeout-ms 60000 \
  --json '{"prompt":"Open a PR that adds a /health endpoint to the api repo","title":"health endpoint"}'
# { "session_id": "devin-...", "url": "https://app.devin.ai/sessions/...", ... }

# poll its status, output, and messages
iii trigger devin::session::get session_id=devin-...

# send a follow-up
iii trigger devin::session::message \
  --json '{"session_id":"devin-...","message":"Also add a test for it"}'

# list recent sessions
iii trigger devin::api --json '{"method":"GET","path":"sessions","query":{"limit":10}}'
```

Run the local CLI as one turn and stream it onto the bus:

```bash
iii trigger devin::run --timeout-ms 600000 \
  --json '{"prompt":"summarize what this repo does","cwd":"/path/to/repo"}'
# streams raw stdout onto devin::events, AgentEvent frames onto agent::events
```

Reach any endpoint the typed wrappers do not cover:

```bash
# paths are relative to base_url. v1 (personal token) uses flat paths:
iii trigger devin::api --json '{"method":"GET","path":"sessions","query":{"limit":5}}'

# v3 (service key) paths are org-scoped:
iii trigger devin::api --json '{"method":"GET","path":"organizations/org_.../sessions"}'
```

Ask the engine for any function's contract:

```bash
iii trigger devin::session::create --help
```

## Functions

The worker follows the same base surface as the grok, codex, claude-code, and opencode agent workers (`run` / `start` / `stop` / `status` / `sessions::list` / `events`), then adds the functions unique to Devin's cloud (session lifecycle, PR review, code scan, and a passthrough).

| Function | Surface | Purpose |
| --- | --- | --- |
| `devin::run` | CLI | Run one local CLI turn, wait, return the result |
| `devin::start` | CLI | Start a turn and return immediately; progress on the streams |
| `devin::stop` | CLI | Interrupt a live CLI run |
| `devin::status` | CLI | A recorded run's state: live flag, status, linked Devin session id |
| `devin::sessions::list` | CLI | Every run this worker has recorded (each linked to its Devin session) |
| `devin::session::create` | Cloud | Start a Devin cloud session from a prompt |
| `devin::session::get` | Cloud | Fetch one session (status, messages, output) |
| `devin::session::message` | Cloud | Send a follow-up message to a running session |
| `devin::pr-review::trigger` | Cloud | Start a Devin review for a pull/merge request |
| `devin::pr-review::status` | Cloud | Latest Devin review for a pull/merge request |
| `devin::code-scan::findings` | Cloud | List enterprise code scan findings (enterprise-gated) |
| `devin::code-scan::metrics` | Cloud | Enterprise code scan metrics (enterprise-gated) |
| `devin::code-scan::remediate` | Cloud | Launch a session to fix a finding (enterprise-gated) |
| `devin::api` | Cloud | Raw authenticated call to any v1/v3 endpoint |

To list all Devin cloud sessions org-wide, use `devin::api {method: GET, path: sessions}` (v1) or `{path: organizations/{org_id}/sessions}` (v3); `devin::sessions::list` is scoped to the runs this worker made, matching the family convention.

`devin::run` / `devin::start` accept either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), the same input contract as the claude-code and grok workers, so the acp worker can drive it with `--brain-fn devin::run`.

`devin::session::create` accepts `prompt` plus the union of the v1 and v3 create fields (`title`, `tags`, `playbook_id`, `knowledge_ids`, `secret_ids`, `max_acu_limit`; v3 `devin_mode`, `repos`, `attachment_urls`, `resumable`, `bypass_approval`; v1 `snapshot_id`, `unlisted`, `idempotent`); each is omitted from the body when not supplied, so populate the ones your token's API version accepts. `devin::session::*` follow the v1 flat paths by default and switch to v3 org-scoped paths when `org_id` is set.

### PR review and code scan

`devin::pr-review::trigger` starts a Devin review for a `pr_url`; `devin::pr-review::status` returns the latest review for that PR. `devin::code-scan::{findings,metrics,remediate}` cover Devin's enterprise code scanning: list and measure findings, then launch a remediation session that opens a fix PR. Code scanning is enterprise-gated, so those three return the API's authorization error on a key without the enterprise permission.

### The passthrough

`devin::api` is the escape hatch for the full v3 surface (roughly 250 endpoints: knowledge, playbooks, secrets, repos, PR review, code scan, org and usage admin). It takes `{ method, path, query?, body? }`, adds the bearer token and organization header, and returns the parsed response. Reach for it whenever a capability is not one of the typed wrappers above; graduate a wrapper only when a call proves common.

### Streams

`devin::run` mirrors every stdout line from the CLI verbatim onto `devin::events` (group_id = session_id) and emits a terminal AgentEvent frame onto `agent::events`, so the iii console renders a Devin CLI turn like any other agent worker. The cloud functions return their JSON directly and do not stream; poll `devin::session::get` for progress, or bind a `cron` trigger to poll on a schedule instead of looping.

## Configuration

Managed by the `configuration` worker; `config.yaml` is the seed installed on first registration and the live value hot-reloads.

```yaml
api_key: "${DEVIN_API_KEY}"       # env-expanded on load; empty disables the API surface
org_id: "${DEVIN_ORG_ID}"         # empty = v1 personal mode; set = v3 org-scoped mode
base_url: https://api.devin.ai/v1 # set to .../v3 alongside org_id for a service key
request_timeout_secs: 120
devin_executable: ""              # path to the devin CLI; empty = PATH
cli_extra_args: []                # args inserted before `-- <prompt>`
events_stream: agent::events      # AgentEvent frames
raw_events_stream: devin::events  # verbatim CLI stdout
iii_context: true                 # prepend iii runtime context to a CLI prompt
```

`api_key` and `org_id` are referenced as `${DEVIN_API_KEY}` and `${DEVIN_ORG_ID}` and expanded from the environment on load (an unset var becomes empty), so neither secret lives in the repo. An empty `api_key` disables the API surface while the CLI surface still works if the local `devin` binary is authenticated. `org_id` selects the API shape: empty uses the flat v1 session paths (personal tokens), set uses the v3 org-scoped paths (service keys) and becomes the required path segment for pr-review and code-scan remediation. `iii_context` defaults on, the same as the grok and codex workers, so a `devin::run` turn is prepended with the iii runtime context and the agent discovers engine functions through the `iii` CLI; the context is most useful when the CLI can reach the engine, and you can turn it off per turn with `iii_context: false`.

## Dependent workers

- `configuration` (required): holds the API key, base URL, and stream names; hot-reloads changes.
- `cron` (optional): schedule `devin::session::create` or poll `devin::session::get` without a polling loop.
- `harness` (optional): fan multiple Devin runs out as sub-agents with `harness::spawn`.

## Permissions

`devin::run`, `devin::start`, `devin::session::create`, `devin::session::message`, and `devin::api` drive or mutate a Devin agent and spend ACUs, so they stay at the `needs_approval` default; an agent invoking them without human approval is a privilege escalation. The read-only introspection functions (`devin::status`, `devin::sessions::list`, `devin::session::get`, `devin::pr-review::status`, `devin::code-scan::findings`/`metrics`) and `devin::stop` are allow-listed in `iii-permissions.yaml`.

## How it maps

| Devin | iii |
| --- | --- |
| one local `devin -- "<prompt>"` turn | `devin::run` invocation |
| every CLI stdout line, verbatim | `devin::events` stream frame |
| a Devin cloud session | `devin::session::create` / `::get` / `::list` / `::message` |
| a Devin PR review | `devin::pr-review::trigger` / `::status` |
| enterprise code scanning | `devin::code-scan::findings` / `::metrics` / `::remediate` |
| any other v3 endpoint | `devin::api` passthrough |
| scheduling a run | `cron` worker trigger, not a worker feature |
| fanning runs out | `harness::spawn`, not a worker feature |
