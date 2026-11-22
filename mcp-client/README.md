# mcp-client

Connects to the MCP servers you configure, over Streamable HTTP or as local stdio processes, and registers each server's tools, resources and prompts as iii functions named `mcp::<server>::<tool>`. A server signs in with an API key, client credentials or an interactive sign-in in the browser. The MCP client page in the console adds, imports, tests and fixes servers, and decides what agents may call without asking. A credential is never written into the configuration: it is always a `secret://` or `env://` reference, read through the secrets worker.

## Install

```bash
iii call compose::add worker=mcp-client
```

It needs the `configuration` and `secrets` workers. Keep the Compose service named `mcp-client`: that is the name secrets are shared with, so under any other name every `secret://` read is refused.

## Quickstart

Linear with an API key.

```bash
# 1. With LINEAR_API_KEY=<your key> in the project's .env, store it for this worker.
iii call secrets::import --json '{"name": "LINEAR_API_KEY", "source": "dotenv", "consumers": ["mcp-client"]}'
```

```json
{"name": "LINEAR_API_KEY", "ref": "secret://LINEAR_API_KEY", "consumers": ["mcp-client"], …}
```

```bash
# 2. Add the server.
iii call configuration::set --json '{"id": "default-mcp-client", "value": {"mcpServers": {"linear": {
  "type": "http", "url": "https://mcp.linear.app/mcp",
  "auth": {"type": "bearer", "token": "secret://LINEAR_API_KEY"}}}}}'
```

```json
{"new_value": {"mcpServers": {"linear": {…}}}, "old_value": {"mcpServers": {}}}
```

Under Compose the configuration id is `<namespace>-mcp-client`, the name Compose puts in `III_CONFIG_NAME` (`default-mcp-client` in the default namespace); `iii call configuration::list` shows it.

```bash
# 3. Approve it: until then nothing connects and the key is not read.
iii call mcp-client::servers::approve --json '{"server": "linear"}'
```

```json
{"approved": true}
```

```bash
# 4. Call a tool.
iii call mcp::linear::get_user --json '{"query": "me"}'
```

```json
{"content": [{"type": "text", "text": "{\"id\": \"…\", \"name\": \"…\", …}"}]}
```

`mcp-client::servers::list` shows each server's `state`, `stage` and `error`.

With an API key Linear offers no `whoami` tool: `get_user` with `"me"` reads the key's own user.

## Configuration

The worker reads one configuration entry: the name in `III_CONFIG_NAME` (under Compose `<namespace>-mcp-client`, e.g. `default-mcp-client`), or `mcp-client` when the worker runs alone. Its value is the map other MCP clients use:

```json
{"mcpServers": {"<name>": {"type": "http", "url": "https://…", "headers": {}, "auth": {}}}}
```

A server name matches `^[a-z0-9][a-z0-9-]{0,23}$`. Each entry is checked on its own: a bad entry stops only that server. The fields this release reads:

| Field | Meaning |
|---|---|
| `type` | `http`; `streamable-http` is read as `http`. Optional when `url` is set. `sse` and `ws` are a config error. |
| `url` | The endpoint: `https`, or `http` on a loopback host. No user or password in it, and no query parameter whose name looks like a secret. When the URL itself carries a key, make the whole URL a reference. |
| `headers` | Header name to value, a literal or a reference. The value of a header whose name looks like a credential (`Authorization`, `Cookie`, or a name with a part such as `KEY`, `TOKEN`, `SECRET`, `PASSWORD` or `AUTH`) must be a reference. `Host`, `Content-Length`, `Content-Type`, `Accept`, `Last-Event-ID` and `Mcp-*` are refused, and `Authorization` cannot be set together with `auth`. |
| `auth` | `bearer`, `client_credentials` or `oauth`: see Authentication below. Without `auth` the worker sends no credential, and asks for a sign-in if the server wants one. |
| `disabled` | `true` keeps the entry but connects nothing and registers nothing. |

An entry with `command` instead of `url` runs a local process: see stdio servers below.

**References.** `secret://NAME` reads the vault; `env://NAME` reads the secrets worker's env store (the project's `.env`, or the secrets worker's own environment). `NAME` matches `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`. A reference is the whole value, never part of a longer string: `"Bearer secret://X"` is a literal, and a literal is refused where a credential belongs. The secret must be shared with `mcp-client` (`"consumers": ["mcp-client"]`).

`${VAR}` is rejected anywhere in an entry: write `env://VAR` instead. Unknown fields are ignored, and `mcp-client::config::validate` reports each one as a warning, so an entry copied from another client still loads. `headersHelper` is never run.

**States.** Each server reports a `state`, the `stage` it is at or stopped at (`config`, `approval`, `secret`, `connect`, `authorize`, `tools`), and an `error` with a `code` and a `message`.

| State | Meaning | Left when |
|---|---|---|
| `disabled` | `disabled: true` | the entry changes |
| `config_error` | The entry is invalid, or a reference cannot be read | the entry or the secret changes |
| `unapproved` | The entry is not the one an operator approved | `mcp-client::servers::approve`, or the entry changes |
| `connecting` | Connect, handshake or tool listing in progress | by itself |
| `ready` | Tools listed and registered | |
| `unavailable` | Network failure, failed handshake, or the session ended | by itself: retried with backoff |
| `auth_failed` | The server rejected the credential | the secret or the entry changes, or `mcp-client::servers::reconnect` |
| `needs_login` | The server asks for a sign-in, or its stored session ended or was signed out | a sign-in finishes, or the entry changes |

A server that leaves `ready` keeps its functions registered. Until it is back, a call answers `` `<server>` MCP unavailable (`<state>`) `` with how long it has been so.

**Fixed limits**, none configurable: 120 s per call, after which the outcome is unknown; 120 KiB per result, longer text is cut and ends with a hint to narrow the query; reconnect backoff from 1 s, doubling to 60 s.

## Authentication

An http entry authenticates in one of four ways.

**No `auth`.** Requests carry no credential. If the server answers with an OAuth sign-in challenge, as Linear does, the server goes to `needs_login` and its row on the MCP client page shows Needs sign-in.

```json
{"linear-auto": {"type": "http", "url": "https://mcp.linear.app/mcp"}}
```

**`bearer`, and secret headers.** The token is sent as `Authorization: Bearer <token>`, and a header can carry any other credential. Both are references, never literals. A changed secret reconnects the server.

```json
{"linear": {"type": "http", "url": "https://mcp.linear.app/mcp",
  "auth": {"type": "bearer", "token": "secret://LINEAR_API_KEY"}}}
```

```json
{"search": {"type": "http", "url": "https://mcp.example.com/mcp",
  "headers": {"X-API-Key": "secret://SEARCH_X_API_KEY", "X-Workspace": "acme"}}}
```

**`client_credentials`.** The worker posts `client_id`, `client_secret` and `scope` to `token_url` (https) and sends the token it gets back. `client_secret` is a reference. `scope` is one string, sent as written: Linear wants commas. When the server rejects the token, the worker gets a new one and retries once. A refused token request leaves the server `auth_failed` with the provider's reason.

```json
{"linear-app": {"type": "http", "url": "https://mcp.linear.app/mcp",
  "auth": {"type": "client_credentials", "token_url": "https://api.linear.app/oauth/token",
    "client_id": "abc123", "client_secret": "secret://LINEAR_CLIENT_SECRET", "scope": "read,write"}}}
```

**`oauth`.** Interactive sign-in. Every field is optional: `scopes` (an array; by default, what the server asks for), `client_id` and `client_secret` (a reference) for a client you registered with the provider, and `client_metadata_url` (https) for a Client ID Metadata Document.

```json
{"linear-me": {"type": "http", "url": "https://mcp.linear.app/mcp",
  "auth": {"type": "oauth", "scopes": ["read", "write"]}}}
```

**Signing in.** Start the sign-in on the MCP client page, or call `mcp-client::login::start` and open its `authorize_url` in a browser:

```bash
iii call mcp-client::login::start --json '{"server": "linear-me"}'
```

The sign-in waits 10 minutes; `mcp-client::login::cancel` drops it. After you sign in, the browser comes back to the callback route on the http worker, which shows a page you can close, and the server turns `ready`.

**The redirect URI to register** with a provider that needs a fixed one is `http://127.0.0.1:<http worker port>/mcp-client/oauth/callback`, port 3111 unless the [http worker](https://github.com/iii-hq/workers/blob/main/http/README.md) is configured otherwise. `mcp-client::servers::list` returns the exact value as `callback_url`.

**When the browser is on another machine**, it cannot reach that address. Sign in anyway, then paste the whole address the browser ended on into the MCP client page, or call:

```bash
iii call mcp-client::login::complete --json '{"callback_url": "<the address the browser ended on>"}'
```

The same path works when the http worker is not installed. The worker then logs one warning 30 s after it starts, and the route comes up if the http worker starts later.

**Client-credentials warnings.**

- Keep the `scope` string identical everywhere.
- Never use one Linear OAuth application for both `client_credentials` and interactive sign-in: a client-credentials request replaces that application's authorization-code installation.

**What is stored, and where.**

- A signed-in session is one vault secret per server, `MCP_CLIENT_<SERVER>_OAUTH` (the server name upper-cased, `-` as `_`), readable only by `mcp-client`. A `client_credentials` token is never stored: it stays in the worker's memory.
- Sign out (`mcp-client::login::sign-out`) clears the stored session. It does not revoke it at the provider.
- Removing a server clears its session; disabling it does not.
- Without `client_id` or `client_metadata_url`, each sign-in registers a new client with the provider.

**The callback route is reachable from the network** when the http worker binds `0.0.0.0`, its default. That grants nothing: a request does something only with the `state` of a sign-in this worker started less than 10 minutes ago, and the code it carries is useless without a verifier that never leaves this worker's memory. A wrong or missing `state` consumes nothing, and every request gets the same static page, which repeats nothing from the request and is not cached. The redirect URI always names `127.0.0.1`.

## stdio servers

An entry with `command`, and optionally `args`, `env` and `cwd`, starts a local MCP server and talks to it over its stdin and stdout.

- The command runs directly, never through a shell. It is looked up on the child's `PATH`: the worker's, or the entry's own `env.PATH` when it sets one. A command with a slash is used as a path. An `npx` server needs Node where the worker runs.
- The confirm before a stdio entry runs shows the whole start: its directory, its environment as written, and the command line, quoted as a shell would need it.
- The child's environment is cleared. It holds only `HOME`, `LOGNAME`, `PATH`, `SHELL`, `TERM`, `USER`, `LANG`, `LC_ALL` and `TMPDIR` from the worker (on Windows `APPDATA`, `HOMEDRIVE`, `HOMEPATH`, `LOCALAPPDATA`, `PATH`, `PROCESSOR_ARCHITECTURE`, `SYSTEMDRIVE`, `SYSTEMROOT`, `TEMP`, `USERNAME` and `USERPROFILE`), plus the entry's `env`, whose values may be references.
- An `args` item that is a reference is resolved into the command line, where anyone who can list the host's processes can read it. Pass a secret through `env` instead.
- A failed start (the process cannot be spawned, exits, or fails its handshake or first listing before 60 s of `ready`) is retried after 1 s, doubling to 60 s. After five failed starts in ten minutes the next try waits five minutes; as those failures age past ten minutes the 60 s wait comes back. 60 s of `ready` or `mcp-client::servers::reconnect` resets both.
- The last 500 stderr lines, each cut at 4 KiB, are kept with the entry's secrets replaced by `[redacted]`. `mcp-client::process::logs` returns them, and `mcp-client::servers::list` shows the process under `process`: its pid, its last exit, the failed starts counted against the budget and the time of the next start.
- `III_MCP_CLIENT_STDIO=off` in the worker's environment rejects every stdio entry as `config_error`.
- The children are not killed when the worker itself is SIGKILLed. A server that exits when its stdin closes goes with it; any other keeps running until it is stopped by hand.

## Resources and prompts

- A server that advertises `resources` gets `mcp::<server>::resources::list` and `mcp::<server>::resources::read`, and `mcp::<server>::resources::templates` when it lists a resource template.
- A server that advertises `prompts` gets one `mcp::<server>::prompts::<prompt>` per prompt. Every argument is a string; a number or a boolean is sent as its text.
- `list` and `templates` return one server page per call. To get the next page, pass the `next_cursor` of the last one as `cursor`.
- Results are capped at 120 KiB. A read is cut. A page drops its descriptions first, then its last items, and says `truncated: true`. A prompt drops its later messages and says how many.
- A prompt whose name cannot be a function id is skipped and shown with the reason under `skipped` in `mcp-client::servers::list` and on the MCP client page.
- Subscriptions, argument completion and template expansion are not supported. To read from a template, fill in its variables yourself and read the uri.
- A failed prompt or template listing keeps the functions of that kind as they were, shows the error under `list_errors` and leaves the server `ready`.
- Resource bodies and prompt messages are untrusted text from the server, like tool results.

## The MCP client page

The worker adds an MCP client page to the console (served through the [ade worker](https://github.com/iii-hq/workers/blob/main/ade/README.md)). It edits the same configuration entry as the CLI and saves through the same validate, confirm and approve steps.

The page has three columns, and their heads spell the function id: `mcp::` over the servers, `linear::` over the selected server, `list_issues` over the selected function. The path an operator clicks is the id an agent calls.

| Column | Shows | Answers |
|---|---|---|
| 1, `mcp::` | Every configured server with a state dot, one line of state (`mcp.linear.app · API key`, `stdio · pid 48213`, "Needs sign-in"), and its tool count or the one action that fixes it | Which servers work, and what to do about each one that does not |
| 2, `<server>::` | The pinned rows Connection, Process (stdio only) and Agent access, then the groups Tools, Resources and Prompts with a search field and a count. A tool that writes carries a "Writes" or "Destructive" chip. | What the server offers, under which ids, and what agents may call on it without asking |
| 3, `<function>` | The selected function's description, its inputs and a Test tab, or the selected pinned row's view | What a function does, and what it returns for real inputs |

**Test** calls the real `mcp::*` function. Run tool sends the form's payload; a tool that writes asks first, with the payload shown, and the note under the button says whether the server marked the tool or its name was guessed. The result shows its time, size and blocks, with the raw reply behind a disclosure. Under it, a CLI line with the same payload (`iii call mcp::linear::list_issues limit=5`) copies to the clipboard. `resources::read` replaces the form with a uri field and a picker over `resources::list`; a template puts its text into the field with the first `{variable}` selected for you to fill in. A prompt shows its arguments as a form, and Get prompt shows the messages with their roles. Mod+Enter runs all three.

**When a server is broken**, its row in column 1 carries the one next step: Sign in, Update key, Share, Fix, Retry, Restart, Review or Turn on. Pressing it selects the server and its Connection view in column 3, where the stage track (Config, Approval, Secret, Connect, Authorize, Tools) shows how far the connection got. Column 2 says what is wrong in a panel above Connection, with the worker's own error text. A stdio server's Process view shows the command line, the restarts against the budget, the last exit and the redacted stderr tail.

## Import

Import, in the page header, reads a config pasted or opened from another MCP client. Parsing runs in the page and executes nothing. The source is found by the shape of the text:

| Root shape | Source |
|---|---|
| `mcpServers` object | Claude Code, Claude Desktop, Cursor, Windsurf, Gemini CLI, VS Code portable |
| `servers` (plus `inputs`) | VS Code editor format |
| `context_servers` | Zed |
| `mcpServers` array of entries with `name` | ACP |
| `remotes` / `packages` | MCP registry `server.json`: remotes only |
| One entry with `command` or `url` | a single server; asks for a name |
| A bare name-to-entry map | accepted |
| Text starting with `[mcp_servers.` | Codex TOML, which is not parsed: "Paste JSON, or add the server by hand" |

References are rewritten into the two forms this worker reads, because the configuration worker would expand `${VAR}` into plain text:

| Pasted form | Becomes |
|---|---|
| `${VAR}`, `${env:VAR}`, Gemini `$VAR` and `%VAR%` (env block), a Codex env-var-name field, as the whole value | `env://VAR` |
| `${VAR:-default}` as the whole value | `env://VAR`; the preview says the default was dropped |
| `Authorization: Bearer ${VAR}` (any reference form) | `auth: {type: bearer, token: env://VAR}`; the header is removed |
| `Authorization: Bearer <literal>` | the literal goes to the vault; `auth: {type: bearer, token: secret://<SERVER>_TOKEN}` |
| Any other literal that looks like a secret | the literal goes to the vault; the field holds `secret://<SERVER>_<FIELD>` |
| A reference inside a longer string (`https://x/${TENANT}/mcp`, `--dir=${HOME}/data`) | the row is marked "needs a value" and the preview shows an input for the final text |
| `${input:id}` (VS Code) | an input in the preview, labelled from `inputs[]`; masked and stored in the vault when `password: true` |
| `${workspaceFolder}`, `${userHome}`, `${CLAUDE_PROJECT_DIR}` and similar | "needs a value", an input for the path |

- A literal secret goes to the vault under `<SERVER>_<FIELD>` (upper-cased, `-` as `_`, editable in the preview), shared with `mcp-client`, and the entry holds its `secret://` reference. A name that already holds a secret another worker reads stops that row; one that only mcp-client reads is replaced, and the preview says so. It is never saved as text: if storing it fails, that server is not imported. The preview shows it only masked, as `•••••••• · 21 characters`. Secrets are found by field name and by shape. A credential in a shape the importer does not know, such as `--header=Authorization: Bearer …` or `-e NAME=value`, stays as pasted: read the preview before importing.
- `headersHelper`, `http_headers_helper`, command-type VS Code inputs and `envFile` are listed as ignored, and never run or read.
- A pasted callback port, redirect URI or callback URL belongs to the other client's sign-in and is dropped.
- SSE and WebSocket entries, and a Gemini `url` (which means SSE there), are shown as Skipped with the reason.
- Importing starts nothing by itself: each server goes through the same validate, confirm and approve steps as a save on the page, and a stdio server runs only after the confirm that names its command.

## Security

- An entry runs only after an operator approved exactly that entry. A changed entry is `unapproved` until `mcp-client::servers::approve`: nothing connects and no secret is read. Turning `disabled` on or off needs no new approval. Approvals live in the vault secret `MCP_CLIENT_APPROVALS`.
- Agents cannot call `mcp-client::*`: the harness, `fp::pipe`, `fp::batch` and `code-runner` refuse the prefix, so an agent cannot approve an entry. This is the protection. An external MCP client reaches these functions only as far as `mcp-server`'s access policy allows: keep `mcp-client::*` out of it.
- The hole: nothing refuses an agent's `secrets::set` or `configuration::set` outright. Where the approval gate runs, those calls wait for a person in `manual` and `auto` mode, and run unasked in a `full`-mode session or after someone answered "always" for them. Where no gate runs, the harness default allow `*` lets them through. Such an agent can write the approvals secret and an entry, and the approval gate's own rules. It can also give itself any secret this worker reads, a stored sign-in session among them, and change a value an approved entry points at: a `secret://` url decides where that entry's credentials go, and an args or env reference what runs. That takes effect at once, with no new approval and no warning.
- A change to `MCP_CLIENT_APPROVALS` that the worker did not make puts every server back to `unapproved` and logs a warning. This lasts until the worker restarts: at start it trusts what is stored.
- Inside `fp::pipe`, `fp::batch` and code-runner guest code, the approval gate sees only the outer call. Approving it, or answering "always" for it, approves every MCP write in it.
- `fp::pipe` and `fp::batch` check every `::` suffix of a step id, so an MCP tool is refused as a step when `<server>::<tool>` is itself an id `fp` refuses (the list is `forbidden_unprefixed_step` in [`fp/src/pipe.rs`](https://github.com/iii-hq/workers/blob/main/fp/src/pipe.rs)). Every tool of a server named `harness`, `configuration`, `oauth`, `router`, `provider`, `run`, `approval`, `stream`, `iii`, `hook-fanout`, `mesh-node` or `mcp-client` is refused, and every tool of `session` other than its reads. Single ids are refused too, among them `workflow::start`, `workflow-result::submit`, `state::claim-namespace`, `database` tools other than `query` while the approval gate runs, and any tool named `on-config-change`. Such a server works everywhere else: pick another name to use its tools as steps.
- Secrets never appear in the configuration, in this worker's logs and traces, in `mcp-client::servers::list` or in error texts. A URL that is a reference is shown as the reference.
- Tool descriptions, tool results, resource bodies and prompt messages are untrusted text from a third party. Do not give one agent MCP tools together with shell or secret access without the [approval gate](https://github.com/iii-hq/workers/blob/main/approval-gate/README.md).
- `readOnlyHint` is the server's word. A server can mark a tool that writes as read-only, and the page then counts it among the reads. A rule is written only after a person confirms a list of every function with where its class came from, and a tool that appears later, or that a server newly marks read-only, is held until a person updates the rules.
- An allow rule stays in force until a person presses Update rules, even after the server stops marking that tool read-only. The page shows it as "1 write allowed without asking" and the worker logs one warning, but nobody is told while the page is closed.
- Writes are held for a person only where the approval gate runs; elsewhere the harness default allow `*` leaves them open, and the page says "No approval gate · writes open". `https://mcp.linear.app/mcp/readonly` is the hard switch: Linear itself serves only its read tools there, on every stack.

## Agent access

The MCP client page writes allow rules into the [approval gate](https://github.com/iii-hq/workers/blob/main/approval-gate/README.md)'s own `rules`, in its configuration entry, and only after a person clicks Allow reads, Update rules or Remove rules and confirms. Each rule names one read function by its exact id, `{"function": "mcp::linear::list_issues", "action": "allow", "rule_id": "mcp-client:mcp::linear::list_issues"}`, never a glob, so a tool the server adds later is held until a person updates the rules. Write and destructive functions get no rule: they match nothing and the gate holds them for a person. A function counts as a read when the server marks it `readOnlyHint: true`, when the server sends no annotations and its name starts with `get_`, `list_`, `search_`, `read_`, `find_` or `describe_` or is `whoami`, and for instructions, resources and prompts. The page also removes a server's rules, inside a confirm the person already answers, when the server is removed, when a new entry takes a name that still has rules, and when an entry's `url`, `command` or `args` change. These rules hold only where the approval gate runs (the last point of Security). The [`iii-permissions.yaml`](https://github.com/iii-hq/workers/blob/main/mcp-client/iii-permissions.yaml) files are documentation: no code reads them at runtime.

The rules are not out of an agent's reach. An agent's `configuration::set` on the gate's entry waits for a person in `manual` and `auto` mode, and runs unasked in a `full`-mode session or after someone answered "always" for that function. Such an agent can change these rules and every other rule of the gate. The Agent access view reads the rules the gate holds, and while it is open it asks the gate for a verdict on each function under the page's own probe session, which has no settings of its own. A read rule taken out while others stay shows as new read tools that still ask; the whole block taken out shows as "Every call asks". While the view is open, a write that the rules or the gate's default mode let through shows as an alert. A session a person put in `full` mode, and a function someone answered "always" for, are allowed by the gate's own design: the view never shows them, so a write let through that way goes unseen here.

While the approval gate is restarting, the page reads it as absent, so a save made in that moment keeps the server's rules. Open Agent access after saving during a gate restart.

A child agent that should only read takes allow `mcp::linear::*` with deny globs for the writes, such as `mcp::linear::save_*`. A child narrows its parent's allow list only to an exact id or a `<prefix>::*` glob, so a verb glob such as `mcp::linear::list_*` does not carry over to it, while a deny glob does ([harness](https://github.com/iii-hq/workers/blob/main/harness/README.md)).

By default an agent reaches every function through the harness's one `agent_call` tool, so a server's functions cost no tool tokens however many there are: Linear's 69 included. Do not combine `expose: native` with allow `*`: every allowed function, each MCP tool of each server among them, then goes to the model as a tool with its schema.

## Live tests

`tests/live_linear.rs` runs against Linear's real MCP server. The tests are `#[ignore]` and read four variables from the environment:

| Variable | Carries | Needed by |
|---|---|---|
| `LINEAR_API_KEY` | A personal API key of a member of the test team | Every API-key test |
| `LINEAR_TEST_TEAM` | The key of a Linear team used for nothing else | Every test that writes |
| `LINEAR_TEST_CLIENT_ID` | The client id of a Linear OAuth application with client credentials turned on | The client-credentials tests |
| `LINEAR_TEST_CLIENT_SECRET` | That application's client secret | The client-credentials tests |

```bash
cd mcp-client && cargo test --all-features -- --ignored live_
```

A test whose variable is missing fails at once and names it, so a run that checked nothing never looks green. Every write lands in the test team: a guard resolves the team before the first write, and each test deletes what it created. `LINEAR_TEST_CLIENT_ID` belongs to an application used for nothing else, never the one used for sign-in: a client-credentials request replaces that application's authorization-code installation.
