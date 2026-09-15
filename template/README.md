# Harness local development template

Run the Harness from this checkout with the Harness Medium agents and skills
already available. This is a project inside the workers repository, not a
standalone copy of the Harness source.

## Start

Requirements:

- The `iii` CLI `0.23.0-rc.4` or a compatible build with managed Compose engines.
- The Rust toolchain in [`../rust-toolchain.toml`](../rust-toolchain.toml).
- Node.js and the pnpm version in [`../package.json`](../package.json).
- Network access on the first build for Cargo, pnpm and worker dependencies.
  See [`../iii-directory/README.md`](../iii-directory/README.md) for offline
  ONNX Runtime setup.

From the repository root:

```bash
cd template
iii compose --up
```

The checked-in instructions need no download before startup. Compose starts its
managed engine at `ws://127.0.0.1:49134` and runs the sibling workers with
`cargo run`, including `path://../harness`. It follows the
[source-only stack](../harness/worker-compose.yaml) and adds the browser worker
used by the profiles. The first build can take several minutes; later runs reuse
the build caches. Open **http://127.0.0.1:3113** when the stack is ready.

Do not run this and the original stack on the same ports. Stop the other stack,
or set `III_ENGINE_PORT` in the Compose terminal and change the Console port in
`config/console.yaml` before the first boot. Do not start a separate engine or
pass `--engine` for this managed project.

## Authentication

`workers-dev.env.example` contains no credentials and is the default env file.
The stack can start without keys, but model calls require authentication:

```bash
cp workers-dev.env.example .env
# Edit .env and set ANTHROPIC_API_KEY or OPENAI_API_KEY.
WORKERS_DEV_ENV_FILE=.env iii compose --up
```

The env-file path is relative to `template/`. All workers receive it, including
`llm-router`, which resolves provider credentials. `.env` is ignored; never
commit real keys. Restart the Compose daemon after changing its env-file choice.
Alternatively, `provider-openai-codex` can use an existing Codex CLI login in
`~/.codex/auth.json` without an API key.

## Local files and synchronized files

```text
template/
├── worker-compose.yaml        local-source stack; never synchronized
├── config/                    local Console and Directory seeds
├── workers-dev.env.example    credential-free startup defaults
├── sync.sh                    upstream fetch entrypoint
├── scripts/sync_template.py   data-only import, using Python's standard library
├── agents/                    upstream profiles
├── skills/harness/            upstream skill documents
├── upstream/sync.json         imported revision and file checksums
├── upstream/config/           optional upstream configs, for review only
└── data/                      runtime state and downloaded skills (ignored)
```

**Edit local configuration directly in `config/`.** It is not generated, merged
or overwritten by `sync.sh`. The Console and Directory commands explicitly load
these seeds via `III_COMPOSE_DIR`, because their processes run in sibling source
directories. Relative worker data paths resolve under this project.

These files are **first-registration seeds**. After first boot, use the Console
configuration UI for saved settings; seeds do not overwrite them on restart.
Directory folder changes require restarting `iii-directory`; Console bind
changes require restarting `ade`. Additional runtime configuration files are
ignored by Git; only the two local development seeds are versioned.

The Directory reads `agents/` and local `skills/`, while automatic worker-skill
downloads go to `data/skills/`. The existing `skills/harness/` namespace prevents
auto-download from replacing the checked-in Medium instructions.

The five profiles are `ade-worker-builder`, `tech-lead`, `backend-engineer`,
`frontend-engineer` and `agent-profile-creator`. They extend `iii-minimal` and
preload skills covering orchestration, Node workers, configuration and frontend
work. The Directory watches instruction edits; start a new session to use them,
since existing sessions retain their frozen prompts.

## Synchronize upstream instructions

Requires **Bash, Git and Python 3.11+**, with no third-party Python dependencies.
Run from any directory; the destination is always the directory of `sync.sh`:

```bash
# From template/: synchronize the latest merged upstream template.
./sync.sh
./sync.sh --dry-run

# Reproduce a commit, or use another branch/tag.
./sync.sh --ref d9ac5f2d183a6fbf79b3bac445a97a9c3e761118

# Work against a local templates checkout instead of GitHub.
./sync.sh --repo /absolute/path/to/templates --ref main
```

The source is [`iii-hq/templates`](https://github.com/iii-hq/templates),
`iii/harness/`, defaulting to `main`. To preview a pull request, pass
`--ref 'refs/pull/<number>/head' --dry-run`. Each invocation fetches the ref into
a temporary Git repository without checking out or executing upstream code.

Only three directories are managed:

- `agents/` and `skills/`: Markdown instructions copied verbatim.
- `upstream/`: `sync.json` records the repository, ref, commit and SHA-256 file
  checksums. Any upstream YAML/JSON configuration is copied to `upstream/config/`
  **as reference only**, never applied to the local stack. The current snapshot
  has no upstream configs. The upstream README, package Compose and template
  installer manifest are not copied; consult them in the source repository.

Sync removes obsolete files only in those directories. It preserves local
`config/`, Compose, README, environment files and runtime data, even with
`--force`. It refuses edited or newly added managed files by default. Save local
agents/skills before refreshing; use `--force` only to intentionally discard
those edits. Review new instructions and reference configs before committing.

Fetch or validation failures leave the snapshot unchanged. Replacements are
staged with rollback on filesystem errors; a lock prevents concurrent syncs.
After a crash, remove `.sync.lock/` only if no sync process is running.

The checked-in snapshot comes from merged
[PR #84](https://github.com/iii-hq/templates/pull/84).
**`upstream/sync.json` is the authoritative imported revision.** Review
`git diff -- template/` from the repository root after each refresh.

## Development and validation

Edit the sibling worker source, then restart just that container from another
terminal in `template/`:

```bash
iii trigger compose::restart --namespace my-project container=harness
```

`cargo run` recompiles changed source; this Compose has no Rust source watcher.
To restart the whole stack, stop it in its Compose terminal and run
`iii compose --up` again, not from a Harness session that the stack hosts.

Run the tests from `template/`:

```bash
uv run --with PyYAML==6.0.3 python -m unittest discover -s tests -v
```

PyYAML is needed only by the structural tests to read Compose and agent YAML,
not by the synchronization script. The Bash integration tests use temporary local
Git repositories and need no network or credentials. CI runs both suites and
checks Bash syntax. Structural checks cover versioned files, not extra local
profiles. Full runtime acceptance is separate: start Compose, confirm the
profiles in the Console and send a message using an authenticated model.
