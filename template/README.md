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
├── sync.sh                    fetch and synchronize upstream template data
├── worker-compose.yaml        local workers, including path://../harness
├── workers-dev.env.example    credential-free startup defaults
├── config-overrides/          maintained local configuration overrides
├── config/                    generated upstream seeds + local overrides
├── agents/                    synchronized Harness Medium profiles
├── skills/harness/            synchronized skill documents
├── upstream/                  source metadata, Compose reference and checksums
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
uv run --with PyYAML==6.0.3 python -m unittest discover -s tests -v
```

They check the local worker paths and binaries, dependency ordering, default env
file, configuration seeds, agent frontmatter, skill references and Git ignore
rules. The synchronization tests run the real Bash script against disposable
local Git repositories: updates, removals, idempotence, pinned revisions,
dry runs, failed fetches, invalid configuration, edit protection and symlink
rejection are exercised without network access. The same suite runs in
`.github/workflows/harness-template.yml` on pull requests and pushes to main.
Runtime acceptance is separate: start Compose, open the console, confirm
the five profiles appear and send a message with an authenticated model.

## Synchronize with upstream

`sync.sh` replaces manual copying with a repeatable fetch-and-sync workflow. It
requires Bash, Git and either **uv**, or Python 3.11+ with PyYAML 6.0.3 installed.
The checked-in generated snapshot still supports starting the stack offline;
you choose when to refresh it. Run the script from any working directory:

```bash
# From template/: fetch the latest merged template from main.
./sync.sh

# Preview changes without replacing generated files.
./sync.sh --dry-run

# Reproduce the merged Harness Medium revision (branches and tags also work).
./sync.sh --ref d9ac5f2d183a6fbf79b3bac445a97a9c3e761118

# Preview another pull request explicitly before it is merged.
./sync.sh --ref refs/pull/<number>/head --dry-run

# Use another checkout instead of GitHub, for local template development.
./sync.sh --repo /absolute/path/to/templates --ref main
```

The source is [`iii-hq/templates`](https://github.com/iii-hq/templates),
`iii/harness/`. The default ref is **main**. The Harness Medium instructions from
[PR #84](https://github.com/iii-hq/templates/pull/84) were merged on September 15,
2026 and are available through the default sync. Use `--ref refs/pull/<number>/head`
explicitly to preview a future pull request. Every invocation fetches the
requested ref; it never relies on an old local checkout. No upstream code or
installer is executed.

The script manages these generated directories:

- `agents/` and `skills/`: upstream Markdown instructions, copied verbatim.
- `config/`: upstream YAML/JSON seeds, if present, recursively merged with
  `config-overrides/*.yaml`. The local overrides win on conflicting values;
  unrelated upstream settings are retained. These files are first-registration
  seeds, not live updates to saved configuration. Additional worker configs are
  copied but require wiring in the local Compose before that worker uses them.
- `upstream/`: the upstream README, package-based Compose and template manifest
  for review, plus `sync.json` recording repository, ref, exact commit and
  SHA-256 checksums of every generated file. The reference Compose is **never**
  used to start the stack. Do not run Compose from this reference directory.

The current upstream has no `config/` directory, so its absence is supported:
our local overrides still produce the Console and Directory seeds. If upstream
adds that directory later, the same script picks it up automatically.

Updates remove obsolete files **only inside those four generated directories**.
The runnable `worker-compose.yaml`, this README, `config-overrides/`, `.env`,
other environment files and runtime data are preserved. Secret files, arbitrary
scripts and other unrecognized upstream paths are not imported. Only request
refs/repositories you trust, and review new configuration before committing it.

The script refuses to overwrite edited or newly added generated files. Keep
custom configuration in `config-overrides/`. Save custom agents/skills before a
refresh; if you intentionally want to discard their local edits, use
`./sync.sh --force`. That flag still cannot replace the local Compose or runtime
data. Fetch and validation failures leave the current snapshot unchanged;
concurrent runs are blocked by `.sync.lock/` (remove a stale lock only after
confirming no synchronization is running).

After syncing, run the validation suite above and review `git diff -- template/`
from the repository root before committing. The checked-in snapshot includes
PR #84's merged revision `d9ac5f2d183a6fbf79b3bac445a97a9c3e761118`;
**`upstream/sync.json` is the authoritative current provenance** after subsequent
synchronizations. The
local-source Compose is maintained separately so updates never switch the
Harness to a published package.
