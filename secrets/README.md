# secrets

`secrets` keeps credentials such as provider API keys out of the files you
commit. Configuration stores a reference, which is safe to version, and the
value lives in one of two stores:

- **vault** — `secret://ANTHROPIC_API_KEY`: encrypted in a local vault. A
  pasted key goes here by default.
- **env** — `env://ANTHROPIC_API_KEY`: an environment variable, read from the
  project's env file — `.env` unless `env_file` names another, which this
  worker can also write — or, when it is not there, from this worker's own
  environment. For people who prefer keeping
  keys in `.env`, and for deploys that inject them as variables.

Either way the value is handed only to the workers you allow, such as
[`llm-router`](https://github.com/iii-hq/workers/tree/main/llm-router).
Listing, inspecting and detecting secrets returns masked hints and
fingerprints, never a value.

## Install

```bash
iii trigger compose::add worker=secrets
```

Compose starts the worker with its `configuration` dependency. `llm-router`
depends on `secrets`, so adding the router from the registry adds it too.
Wait for `secrets::status` to register before calling it.

## Quickstart

Find a key you already have. `secrets::detect` looks in the worker's own
environment, the project's `.env` and your login shell, and reports where it
found each name:

```bash
iii trigger secrets::detect --json '{"names":["ANTHROPIC_API_KEY"]}'
```

```json
{"results": [{"name": "ANTHROPIC_API_KEY", "stored": false, "sources": [
  {"kind": "dotenv", "location": "/home/me/project/.env",
   "hint": "sk-ant…9f2c", "matches_stored": false}
]}]}
```

Import it and allow the router to read it. The worker re-reads the value from
the source itself; it never passes through the caller or your shell history:

```bash
iii trigger secrets::import --json '{
  "name": "ANTHROPIC_API_KEY", "source": "dotenv", "consumers": ["llm-router"]
}'
```

```json
{"name": "ANTHROPIC_API_KEY", "ref": "secret://ANTHROPIC_API_KEY",
 "consumers": ["llm-router"], "hint": "sk-ant…9f2c",
 "fingerprint": "3f9a0c1d2e4b5a67",
 "created_at": "2026-10-02T18:04:11.532Z", "updated_at": "2026-10-02T18:04:11.532Z"}
```

Then put the reference, not the key, in the provider's configuration
(**Settings → Workers → llm-router**, stored in `./config`):

```yaml
providers:
  anthropic:
    api_key: secret://ANTHROPIC_API_KEY
```

To keep the key in `.env` instead, pass `"store": "env"`. With a value,
`secrets::set` writes `ANTHROPIC_API_KEY=…` to the project's `.env`, replacing
an existing definition in place (or an empty `# ANTHROPIC_API_KEY=`
placeholder) and leaving every other line as it was. A variable already in
`.env` is shared as it is with `secrets::access`:

```bash
iii trigger secrets::access --json '{
  "name": "ANTHROPIC_API_KEY", "consumers": ["llm-router"], "store": "env"
}'
```

```json
{"name": "ANTHROPIC_API_KEY", "ref": "env://ANTHROPIC_API_KEY", "store": "env",
 "consumers": ["llm-router"], "hint": "sk-ant…9f2c",
 "location": "/home/me/project/.env",
 "created_at": "2026-10-06T12:04:11.532Z", "updated_at": "2026-10-06T12:04:11.532Z"}
```

and the provider's configuration gets `api_key: env://ANTHROPIC_API_KEY`.

The Console's onboarding wizard and every key field are built on the same
calls: pasting a key calls `secrets::set`, picking a detected one calls
`secrets::import`, and keeping a variable as it is calls `secrets::access`
with `store: "env"`. Fingerprints and timestamps above are illustrative.

## References

A reference is `secret://NAME` (the vault) or `env://NAME` (the env store),
where `NAME` matches `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`; the scheme is
case-insensitive. Provider keys reuse their environment variable name:
`secret://OPENAI_API_KEY`, `env://TYPESAFE_API_KEY`. `secrets::resolve` also
accepts the bare `NAME`, as a vault secret. The two stores are separate: the
same `NAME` can be in the vault and in `.env`, and each reference reads its
own. `III_SECRETS_KEY` is reserved and cannot be stored.

## Where values and the key live

| What | Where | Why it is not committed |
|---|---|---|
| Encrypted values | `<data_dir>/vault.json`, default `data/secrets/vault.json` in the project | `data/` is gitignored by the project template; even if copied, every value is XChaCha20-Poly1305 ciphertext |
| Master key | `III_SECRETS_KEY` (base64, 32 bytes) when set; otherwise `${XDG_CONFIG_HOME:-~/.config}/iii/secrets/<vault_id>.key` | It never lives in the project. A `key_file` inside the project directory is refused |
| Env store values | The configured `env_file` (default `<III_COMPOSE_DIR>/.env`), else this worker's environment | `.env` is gitignored by the project template. It is plain text: the vault records only who may read each variable |
| References and settings | `./config/<entry>.yaml` | Paths and `secret://` / `env://` references only |

The vault and the key file are written atomically, owner-only (files `0600`,
directories `0700`). The key file is created the first time a value is stored.
Each value is sealed with a fresh nonce and bound to its name, so a ciphertext
moved to another record fails to open. The vault header records a check of the
key that sealed it: a different key is refused with `KEY_MISMATCH`, and a
missing key file is reported, never silently replaced.

Back up the key file, or use `III_SECRETS_KEY` from your CI or container
secret store. Without the key, stored values cannot be recovered and must be
entered again.

The env store reads `.env` each time a variable is resolved, so an edit made
by hand applies at once. It never polls the file: the operating system
reports changes to the project directory (inotify, FSEvents or
ReadDirectoryChangesW), and an event naming `.env` re-reads the shared
variables and reports what changed on `secrets::changed`. Where the platform
cannot watch, an edit made by hand still applies on the next resolution but
is not announced. A value it writes must fit on one line; `.env` is written
atomically and left owner-only (`0600`).

## Functions

| Function | Purpose | Agents |
|---|---|---|
| `secrets::set` | Create or rotate `{name, value, consumers?, description?, store?}`; omitted `consumers` keep the current list. `store: "env"` writes the value to `.env` | denied |
| `secrets::import` | Store a value from `process_env`, `dotenv` or `login_shell`, re-read by the worker. With `store: "env"`, a value already in `.env` or the worker's environment is shared as it is, and one from the login shell is copied into `.env` | denied |
| `secrets::access` | Replace a secret's `consumers`; `[]` revokes everyone. With `store: "env"`, shares the variable, whether or not it is set yet | denied |
| `secrets::delete` | Delete a secret and revoke all access. With `store: "env"`, stops sharing the variable; it stays in `.env` | denied |
| `secrets::resolve` | Return `{name, value}` for `secret://NAME` or `env://NAME` to a listed consumer | denied |
| `secrets::get` / `secrets::list` | Metadata: `ref`, `store`, masked `hint`, `fingerprint` (vault), `location` (env), `consumers`, timestamps, last resolver. `get` takes `store` too | allowed |
| `secrets::status` | Vault path, key source (`env` or `file`) and key path, count, vault format version, the `.env` the env store uses and how many variables it shares | allowed |
| `secrets::detect` | Where existing values of the given names are, with masked hints and whether each equals the stored value | approval |

`secrets::resolve` errors with `INVALID_REFERENCE`, `SECRET_NOT_FOUND` or
`SECRET_FORBIDDEN`. For `env://NAME`, a variable shared with nobody (or not
with the caller) is `SECRET_FORBIDDEN`, and one that is shared but not set is
`SECRET_NOT_FOUND`. Other codes are `INVALID_REQUEST`, `SOURCE_NOT_FOUND`
(import), `KEY_UNAVAILABLE`, `KEY_MISMATCH`, `VAULT_ERROR`, `DECRYPT_FAILED` and
`CALLER_LOOKUP_FAILED`. No error message contains a value. Schemas are
available through `iii trigger <function> --help`.

The "Agents" column is the default in
[`iii-permissions.yaml`](https://github.com/iii-hq/workers/blob/main/secrets/iii-permissions.yaml).
The Console calls these functions on the person's behalf.

## Access scope

`consumers` lists the worker names allowed to resolve a secret. An empty list
admits nobody, which is the default for a new secret. The same holds for the
env store: an `env://NAME` reference resolves only for the workers its grant
lists, never for every worker that asks. The caller is the
connection the engine stamps on the invocation (`_caller_worker_id`, which the
engine overwrites on every call), mapped to its worker name through
`engine::workers::list`. A worker registered in another namespace never
matches, even with the same name. Each successful resolution records
`last_resolved_at` and `last_resolved_by`.

## Detection sources

| `kind` | Reads |
|---|---|
| `process_env` | The `secrets` worker's own environment |
| `dotenv` | `<III_COMPOSE_DIR>/.env` (`KEY=VALUE`, comments, `export`, single or double quotes, inline ` #` comments) |
| `login_shell` | `$SHELL -ilc` printing a marker and `env -0`, with stdin closed, stderr discarded and a 4 s limit. The shell runs in its own session, with `III_RESOLVING_ENVIRONMENT=1` set so a shell profile can skip slow setup |

A source that is missing, broken or slow contributes nothing; it never fails
the call. Only the requested names are kept from a source, and only masked
hints leave the worker. Login-shell detection is Unix-only.

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `secrets::changed` | A secret is created, rotated, deleted or its consumers change; for the env store, also when an edit to `.env` sets, changes or removes a shared variable | `{name, ref, action: "created" \| "rotated" \| "deleted" \| "access_changed", fingerprint?, updated_at}` |

`ref` names the store (`secret://NAME` or `env://NAME`); env store events
carry no `fingerprint`. The binding config `{names?: string[]}` narrows
delivery to some secrets: a bare name for both stores, a `secret://` or
`env://` reference for one; omit it for every secret. Delivery is
fire-and-forget and never carries a value: a subscriber that needs the new
value calls `secrets::resolve`.

## How llm-router uses it

`llm-router` depends on this worker. A provider slice whose `api_key` is
`secret://NAME` or `env://NAME` makes the router resolve it through
`secrets::resolve` instead of reading a literal key. Add `llm-router` to that
secret's consumers. The router caches the value, binds `secrets::changed` to
refetch it and refresh that provider's model list (an edit to `.env` included),
and reports `credential_source: "secret"` with `credential_ref` (and
`credential_error` when it cannot resolve) in `router::provider::list`.
Other workers can adopt the same pattern, for example `judge-typesafe` with
`secret://TYPESAFE_API_KEY` (it reads `secret://` references only for now).

## Configuration

```yaml
data_dir: data/secrets   # vault folder; relative paths resolve against the project
key_file: null           # master key file; absolute or ~/, outside the project
env_file: .env           # the env file env:// references read and the console writes
```

Every Compose namespace has its own `secrets` entry (`<namespace>-secrets`),
so each environment or namespace can keep its keys in its own env file. Set it
in that entry's file, `./config/<namespace>-secrets.yaml` (an edit applies
without a restart), with `iii trigger configuration::set`, or per container in
`worker-compose.yaml` (the ADE has no settings form for this worker yet):

```yaml
containers:
  secrets:
    worker: package://secrets
    config_override:
      env_file: .env.staging
```

`env_file` is relative to the project unless it is absolute or starts with
`~/`. A missing file is not an error: references then read this worker's
environment, and the first key saved through the console creates the file
(owner-only). Pointing `env_file` at another file takes effect at once: the
worker follows the new file, and every shared variable whose value differs
there is reported on `secrets::changed`, so consumers stop using the old
file's values. Keep every env file out of Git (the project template ignores
`.env` and `.env.*`).

`III_SECRETS_KEY` overrides `key_file`. The worker reads it once at startup
and removes it from its own environment, so shells started by `secrets::detect`
do not inherit it.

## Limits

This first version is a local protected store, not a hardware-backed vault.

- The env store is as protected as `.env` is: plain text, readable by your
  user account. Its grants decide who may resolve a variable over the engine,
  not who can read the file. Use the vault for keys that should be encrypted
  at rest.

- Anyone who can read both the vault and the key file can decrypt every
  value. By default that is your own user account and root. File permissions
  are the only barrier, and they are not narrowed on Windows.
- Consumer identity is a worker name. Any process that can connect to the
  engine and register as `llm-router` in this namespace while the real router
  is not connected is treated as the router.
- `III_SECRETS_KEY` stays readable in the process table entry for the
  worker (`/proc/<pid>/environ`) by the same user, even after the worker
  removes it from its environment.
- A resolved value is plaintext in the consumer's memory and crosses the
  local engine connection. This worker turns off the SDK's payload capture in
  traces for its own handlers. Each consumer is responsible for not logging
  the value it receives.
- There is no master-key rotation or OS keychain integration yet. Rotate an
  individual secret with `secrets::set`.
