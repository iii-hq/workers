# Harness local development template

Run the local Harness stack with the Harness Medium agent profiles and skills
already available in the console. This directory is a runnable project inside
the workers checkout, not a standalone copy of the Harness source.

## Start

Requirements:

- The `iii` CLI `0.23.0-rc.4` or a compatible build with managed Compose engines.
- The Rust toolchain selected by [`../rust-toolchain.toml`](../rust-toolchain.toml).
- Node.js and the pnpm version declared in [`../package.json`](../package.json).
  The Rust build scripts build the console and worker UIs with pnpm.
- Network access on the first build for Cargo, pnpm, ONNX Runtime and browser
  dependencies. See [`../iii-directory/README.md`](../iii-directory/README.md)
  for offline ONNX Runtime setup.

From the repository root:

```bash
cd template
iii compose --up
```

No file generation, template download or published Harness package is required.
Compose starts its managed engine at `ws://127.0.0.1:49134`, then runs each worker
with `cargo run` in its sibling source directory. The first build can take
several minutes; later runs reuse the Cargo and pnpm caches. The startup timeout
is 15 minutes per worker.

Open **http://127.0.0.1:3113** when the workers are ready. Pick a model you have
authenticated and an agent profile, then start a conversation.

Do not run this and `harness/worker-compose.yaml` on the same ports. Stop the
other stack first, or set `III_ENGINE_PORT` in the Compose terminal and choose a
different `http_port` in `config/console.yaml` before the template's first boot.
Do not start a separate engine or pass `--engine` for this managed project.

## Authentication

The checked-in `workers-dev.env.example` contains no credentials and is the default
`env_file`, so the stack can start immediately. Model calls still need a valid
provider credential. To use an API key:

```bash
cp workers-dev.env.example .env
# Edit .env and set ANTHROPIC_API_KEY or OPENAI_API_KEY.
WORKERS_DEV_ENV_FILE=.env iii compose --up
```

The selected file is relative to this directory and is passed to the workers,
including `llm-router`, where API credentials are resolved. `.env` is ignored by
Git. Keep real keys out of `workers-dev.env.example`, YAML and commits. Restart the Compose
daemon after changing which env file it uses.

The included `provider-openai-codex` can also read an existing Codex CLI login
from `~/.codex/auth.json`; it does not require an API key.

## Layout and configuration

```text
template/
├── worker-compose.yaml        local workers, including path://../harness
├── workers-dev.env.example               credential-free startup defaults
├── config/
│   ├── console.yaml           loopback-only console, port 3113
│   └── iii-directory.yaml     project agents, local skills, download cache
├── agents/                    five Harness Medium profiles
├── skills/harness/            17 skill documents from the reference PR
└── data/                      runtime state and downloaded skills (ignored)
```

The Compose stack follows [`../harness/worker-compose.yaml`](../harness/worker-compose.yaml):
queue, state, session manager, router, OpenAI/Codex/Anthropic providers, context
manager, directory, cron, ADE, IDE and Harness. It adds the local `browser`
worker used by the profiles for verification. All workers run from this checkout;
editing `../harness/src/` changes the Harness used here after a restart.

Compose supplies `III_COMPOSE_DIR` as this directory while each worker's process
runs in its source directory. The directory and console commands explicitly
load the YAML seeds from `template/config/`. Worker-owned relative data paths
resolve under the Compose project rather than polluting the source directories.

Configuration files are **first-registration seeds**, not files reloaded over
saved settings at every restart. After the first boot, use the console's
configuration UI to change the `iii-directory` or `console` entry. Directory
folder changes require restarting `iii-directory`; console bind-address changes
require restarting `ade`. No separate `harness/config.yaml` needs to be created.

The directory uses `skills/` as its local override root and `data/skills/` for
automatically downloaded worker skills. The existing `skills/harness/` namespace
wins and makes automatic reconciliation skip the Harness registry bundle. This
keeps the checked-in Medium instructions intact while allowing other workers'
skills to download normally. Agent profiles are read from `agents/`.

## Included profiles

| Profile | Responsibility |
| --- | --- |
| `ade-worker-builder` | Plans the worker with the user and accepts it in the console. |
| `tech-lead` | Designs the architecture, delegates implementation and verifies the integration. |
| `backend-engineer` | Builds the Node worker, function contracts and UI delivery. |
| `frontend-engineer` | Builds and verifies the injected console UI. |
| `agent-profile-creator` | Plans and writes additional agent profiles. |

Each extends the built-in `iii-minimal` identity and references skills by
`harness/...` id. They inherit the selected model. Orchestrators dispatch through
`harness::spawn`; children report through the `state` worker. The skills cover
orchestration, Node workers, configuration, console design and frontend work.

Edits to profiles and skills are watched by the directory. Start a new session
to use changed instructions: existing sessions retain their frozen prompt.

## Development loop

Edit the sibling worker source, then restart just that container from another
terminal in `template/`, targeting this project's engine and namespace:

```bash
iii trigger compose::restart --namespace my-project container=harness
```

The worker's `cargo run` recompiles the changed source. There is no automatic
Rust source watcher in this Compose file. To restart the whole project, press
`Ctrl-C` in its Compose terminal and run `iii compose --up` again. Avoid stopping
or restarting the project from a Harness session it hosts.

## Validate the template

The structural tests do not start an engine, call a model or require credentials:

```bash
uv run tests/test_template.py
```

They check the local worker paths and binaries, dependency ordering, default env
file, configuration seeds, agent frontmatter, skill references and Git ignore
rules. Runtime acceptance is separate: start Compose, open the console, confirm
the five profiles appear and send a message with an authenticated model.

## Upstream reference

`agents/` and `skills/` are copied verbatim from
[iii-hq/templates PR #84](https://github.com/iii-hq/templates/pull/84),
`iii/harness/` at commit `fab5e83895b599d4522e64fb8e05d76b7568010c`.
They are vendored files, not symlinks or boot-time downloads. Keep the provenance
up to date when importing a newer version. The Compose file, configuration,
environment example and this guide are adapted for local-source development.
