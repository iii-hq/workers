# secrets

`secrets` keeps credentials such as provider API keys out of the files you
commit. Configuration stores a reference, `secret://ANTHROPIC_API_KEY`, which
is safe to version; the value lives encrypted in a local vault and is handed
only to the workers you allow, such as
[`llm-router`](https://github.com/iii-hq/workers/tree/main/llm-router).
Listing, inspecting and detecting secrets returns masked hints and
fingerprints, never a value.

## Install

```bash
iii trigger compose::add worker=secrets
```

Compose starts the worker with its `configuration` dependency. Wait for
`secrets::status` to register before calling it.

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

The Console's onboarding wizard is built on the same calls: pasting a key
calls `secrets::set`, picking a detected one calls `secrets::import`.
Fingerprints and timestamps above are illustrative.

## References

A reference is `secret://NAME`, where `NAME` matches
`^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`. Provider keys reuse their environment
variable name: `secret://OPENAI_API_KEY`, `secret://TYPESAFE_API_KEY`.
`secrets::resolve` also accepts the bare `NAME`. `III_SECRETS_KEY` is reserved
and cannot be stored.

## Where values and the key live

| What | Where | Why it is not committed |
|---|---|---|
| Encrypted values | `<data_dir>/vault.json`, default `data/secrets/vault.json` in the project | `data/` is gitignored by the project template; even if copied, every value is XChaCha20-Poly1305 ciphertext |
| Master key | `III_SECRETS_KEY` (base64, 32 bytes) when set; otherwise `${XDG_CONFIG_HOME:-~/.config}/iii/secrets/<vault_id>.key` | It never lives in the project. A `key_file` inside the project directory is refused |
| References and settings | `./config/<entry>.yaml` | Paths and `secret://` references only |

The vault and the key file are written atomically, owner-only (files `0600`,
directories `0700`). The key file is created the first time a value is stored.
Each value is sealed with a fresh nonce and bound to its name, so a ciphertext
moved to another record fails to open. The vault header records a check of the
key that sealed it: a different key is refused with `KEY_MISMATCH`, and a
missing key file is reported, never silently replaced.

Back up the key file, or use `III_SECRETS_KEY` from your CI or container
secret store. Without the key, stored values cannot be recovered and must be
entered again.

## Functions

| Function | Purpose | Agents |
|---|---|---|
| `secrets::set` | Create or rotate `{name, value, consumers?, description?}`; omitted `consumers` keep the current list | denied |
| `secrets::import` | Store a value from `process_env`, `dotenv` or `login_shell`, re-read by the worker | denied |
| `secrets::access` | Replace a secret's `consumers`; `[]` revokes everyone | denied |
| `secrets::delete` | Delete a secret and revoke all access | denied |
| `secrets::resolve` | Return `{name, value}` to a listed consumer | denied |
| `secrets::get` / `secrets::list` | Metadata: `ref`, masked `hint`, `fingerprint`, `consumers`, timestamps, last resolver | allowed |
| `secrets::status` | Vault path, key source (`env` or `file`) and key path, count, vault format version | allowed |
| `secrets::detect` | Where existing values of the given names are, with masked hints and whether each equals the stored value | approval |

`secrets::resolve` errors with `INVALID_REFERENCE`, `SECRET_NOT_FOUND` or
`SECRET_FORBIDDEN`. Other codes are `INVALID_REQUEST`, `SOURCE_NOT_FOUND`
(import), `KEY_UNAVAILABLE`, `KEY_MISMATCH`, `VAULT_ERROR`, `DECRYPT_FAILED` and
`CALLER_LOOKUP_FAILED`. No error message contains a value. Schemas are
available through `iii trigger <function> --help`.

The "Agents" column is the default in
[`iii-permissions.yaml`](https://github.com/iii-hq/workers/blob/main/secrets/iii-permissions.yaml).
The Console calls these functions on the person's behalf.

## Access scope

`consumers` lists the worker names allowed to resolve a secret. An empty list
admits nobody, which is the default for a new secret. The caller is the
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
| `secrets::changed` | A secret is created, rotated, deleted or its consumers change | `{name, ref, action: "created" \| "rotated" \| "deleted" \| "access_changed", fingerprint?, updated_at}` |

The binding config `{names?: string[]}` narrows delivery to some secrets
(bare names or `secret://` references); omit it for every secret. Delivery is
fire-and-forget and never carries a value: a subscriber that needs the new
value calls `secrets::resolve`.

## How llm-router uses it

A provider slice whose `api_key` is `secret://NAME` makes the router resolve
`NAME` through `secrets::resolve` instead of reading a literal key. Add
`llm-router` to that secret's consumers. The router caches the value, binds
`secrets::changed` to refetch it and refresh that provider's model list, and
reports `credential_source: "secret"` with `credential_ref` (and
`credential_error` when it cannot resolve) in `router::provider::list`.
Other workers can adopt the same pattern, for example `judge-typesafe` with
`secret://TYPESAFE_API_KEY`.

## Configuration

```yaml
data_dir: data/secrets   # vault folder; relative paths resolve against the project
key_file: null           # master key file; absolute or ~/, outside the project
```

`III_SECRETS_KEY` overrides `key_file`. The worker reads it once at startup
and removes it from its own environment, so shells started by `secrets::detect`
do not inherit it.

## Limits

This first version is a local protected store, not a hardware-backed vault.

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
