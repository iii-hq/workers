# mcp-client

Connects to the MCP servers you configure and registers each server's tools as iii functions named `mcp::<server>::<tool>`. This release speaks Streamable HTTP. A credential is never written into the configuration: it is always a `secret://` or `env://` reference, read through the secrets worker.

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
| `auth` | `{"type": "bearer", "token": "secret://NAME"}`. The token must be a reference. Without `auth` the worker sends no credential. |
| `disabled` | `true` keeps the entry but connects nothing and registers nothing. |

The other `auth` types are not run by this release. An entry with `command` instead of `url` runs a local process: see stdio servers below.

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
| `needs_login` | The server asks for an interactive sign-in, which this release cannot do | the entry changes |

A server that leaves `ready` keeps its functions registered. Until it is back, a call answers `` `<server>` MCP unavailable (`<state>`) `` with how long it has been so.

**Fixed limits**, none configurable: 120 s per call, after which the outcome is unknown; 120 KiB per result, longer text is cut and ends with a hint to narrow the query; reconnect backoff from 1 s, doubling to 60 s.

## stdio servers

An entry with `command`, and optionally `args`, `env` and `cwd`, starts a local MCP server and talks to it over its stdin and stdout.

- The command runs directly, never through a shell, and must be on the worker's `PATH`: an `npx` server needs Node where the worker runs.
- The child's environment is cleared. It holds only `HOME`, `LOGNAME`, `PATH`, `SHELL`, `TERM`, `USER`, `LANG`, `LC_ALL` and `TMPDIR` from the worker (on Windows `APPDATA`, `HOMEDRIVE`, `HOMEPATH`, `LOCALAPPDATA`, `PATH`, `PROCESSOR_ARCHITECTURE`, `SYSTEMDRIVE`, `SYSTEMROOT`, `TEMP`, `USERNAME` and `USERPROFILE`), plus the entry's `env`, whose values may be references.
- An `args` item that is a reference is resolved into the command line, where anyone who can list the host's processes can read it. Pass a secret through `env` instead.
- A failed start (the process exits or its handshake fails before 60 s of `ready`) is retried after 1 s, doubling to 60 s, and after five failed starts in ten minutes only one try is made every five minutes, until 60 s of `ready` or `mcp-client::servers::reconnect` resets both.
- The last 500 stderr lines, each cut at 4 KiB, are kept with the entry's secrets replaced by `[redacted]`. `mcp-client::process::logs` returns them, and `mcp-client::servers::list` shows the process under `process`: its pid, its last exit, the failed starts counted against the budget and the time of the next start.
- `III_MCP_CLIENT_STDIO=off` in the worker's environment rejects every stdio entry as `config_error`.
- The children are not killed when the worker itself is SIGKILLed. A server that exits when its stdin closes goes with it; any other keeps running until it is stopped by hand.

## Resources and prompts

- A server that advertises `resources` gets `mcp::<server>::resources::list` and `mcp::<server>::resources::read`, and `mcp::<server>::resources::templates` when it lists a resource template.
- A server that advertises `prompts` gets one `mcp::<server>::prompts::<prompt>` per prompt. Every argument is a string; a number or a boolean is sent as its text.
- `list` and `templates` return one server page per call. To get the next page, pass the `next_cursor` of the last one as `cursor`.
- Results are capped at 120 KiB. A read is cut. A page drops its descriptions first, then its last items, and says `truncated: true`. A prompt drops its later messages and says how many.
- A prompt whose name cannot be a function id is skipped and shown with the reason under `skipped` in `mcp-client::servers::list` and on the MCP servers page.
- Subscriptions, argument completion and template expansion are not supported. To read from a template, fill in its variables yourself and read the uri.
- A failed prompt or template listing keeps the functions of that kind as they were, shows the error under `list_errors` and leaves the server `ready`.
- Resource bodies and prompt messages are untrusted text from the server, like tool results.

## Security

- An entry runs only after an operator approved exactly that entry. A changed entry is `unapproved` until `mcp-client::servers::approve`: nothing connects and no secret is read. Turning `disabled` on or off needs no new approval. Approvals live in the vault secret `MCP_CLIENT_APPROVALS`.
- Agents cannot call `mcp-client::*`: the harness, `fp::pipe`, `fp::batch` and `code-runner` refuse the prefix, so an agent cannot approve an entry. This is the protection.
- The hole: nothing refuses an agent's `secrets::set` or `configuration::set` outright. Where the approval gate runs, those calls wait for a person in `manual` and `auto` mode, and run unasked in a `full`-mode session or after someone answered "always" for them. Where no gate runs, the harness default allow `*` lets them through. Such an agent can write the approvals secret and an entry.
- A change to `MCP_CLIENT_APPROVALS` that the worker did not make puts every server back to `unapproved` and logs a warning. This lasts until the worker restarts: at start it trusts what is stored.
- Inside `fp::pipe`, `fp::batch` and code-runner guest code, the approval gate sees only the outer call. Approving it, or answering "always" for it, approves every MCP write in it.
- `fp::pipe` and `fp::batch` check every `::` suffix of a step id, so an MCP tool is refused as a step when `<server>::<tool>` is itself an id `fp` refuses (the list is `forbidden_unprefixed_step` in [`fp/src/pipe.rs`](https://github.com/iii-hq/workers/blob/main/fp/src/pipe.rs)). Every tool of a server named `harness`, `configuration`, `oauth`, `router`, `provider`, `run`, `approval`, `stream`, `iii`, `hook-fanout`, `mesh-node` or `mcp-client` is refused, and every tool of `session` other than its reads. Single ids are refused too, among them `workflow::start`, `workflow-result::submit`, `state::claim-namespace`, `database` tools other than `query` while the approval gate runs, and any tool named `on-config-change`. Such a server works everywhere else: pick another name to use its tools as steps.
- Secrets never appear in the configuration, in this worker's logs and traces, in `mcp-client::servers::list` or in error texts. A URL that is a reference is shown as the reference.
- Tool descriptions and results are untrusted text from a third party. Do not give one agent MCP tools together with shell or secret access without the [approval gate](https://github.com/iii-hq/workers/blob/main/approval-gate/README.md).
- Writes are held for a person only where the approval gate runs; elsewhere the harness default allow `*` leaves them open. `https://mcp.linear.app/mcp/readonly` is the hard switch: Linear itself serves only its read tools there, on every stack.
