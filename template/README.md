# Download templates for local development

`sync.sh` downloads the contents of `iii/<name>/` from
[`iii-hq/templates`](https://github.com/iii-hq/templates) into `template/<name>/`.
The default name is `harness`.

After downloading, sync updates `worker-compose.yaml` to use matching workers
from this repository. For example, `package://harness` becomes
`path://../../harness`, with `scripts.run: cargo run --bin harness`.
It does not start workers or execute anything from the template. Follow the
downloaded template's instructions to configure authentication and run it.

## Usage

From the repository root:

```bash
cd template

# Download iii/harness/ into template/harness/ and configure local workers.
./sync.sh

# Download iii/harness-kanban/ into template/harness-kanban/.
./sync.sh --template harness-kanban

# Preview without creating or changing the destination.
./sync.sh --template harness-kanban --dry-run

# Select a branch, tag, commit or pull-request ref (default: main).
./sync.sh --template harness-kanban --ref <ref>

# Download from a local Git repository instead of GitHub.
./sync.sh --repo /absolute/path/to/templates --template harness-kanban
```

You can run the script from any working directory. The downloaded folder is
always created beside `sync.sh`, never relative to the caller's directory.
Different templates have independent destinations.

## What is downloaded

Every tracked file inside the selected folder is copied, including hidden files
(such as `.env` and `.gitignore`), README, `template.yaml`, Compose, configuration,
source code, scripts, binary assets and symlinks. File contents and executable
bits are preserved, except for the Compose changes below. No local configuration
seeds are added, and no sync manifest is generated inside the download.

### Local workers

For each active `package://<name>` entry in the downloaded `worker-compose.yaml`,
sync looks for `<repository>/<name>/iii.worker.yaml`. Paths are relative to the
downloaded Compose file, not the shell's working directory. Container aliases
do not change which source directory is selected.

Rust workers with a `Cargo.toml` get `scripts.run: cargo run --bin <bin>`, using
the binary name from the worker manifest. Existing run commands and hooks are
preserved. Other local workers use their manifest's `scripts.start`. Workers
without a local manifest or a usable start command keep their package source.
Existing `path://` entries and custom registry URLs are kept.

The YAML editor preserves comments, including disabled provider examples, and
keeps environment, configuration and dependency settings. The `version` field
is retained with its comments; Compose does not use it for `path://` sources.
If no workers change, the Compose file is copied byte for byte. A Compose symlink
is copied without following or editing its target.

`--dry-run` lists the local worker paths without changing the destination.
Adaptation runs in the staging folder, so invalid YAML fails before installation.

Only the selected folder's contents are downloaded: files referenced elsewhere
by an installer manifest are not resolved or added. Git submodules are external
references rather than folder contents; they produce an explicit error instead
of an incomplete download.

There is no requirement for `agents/`, `skills/`, Markdown, or Compose. Templates
without a `worker-compose.yaml` file are copied without adaptation.

**If the destination folder already exists, sync asks before overwriting files.**
The English prompt warns you to back up your files and asks for `yes` or `no`
(default: no). Only `yes` or `y` (case-insensitive) allows replacement. Answering
`no`, pressing Enter, invalid input, EOF or Ctrl+C cancels without modifying the
destination. Cancellation exits with a nonzero status and no success message.
New folders and `--dry-run` do not prompt. The legacy `--force` flag does not
bypass confirmation.

After confirmation, all files at matching upstream paths are overwritten,
including local changes to `.env`, Compose and configuration. No backup is made
automatically; back up changes you want to retain before answering yes. Local-only files are not deleted,
including files that have disappeared upstream; this is a download, not a mirror
or a project migration. No `--force` or `--stack-stopped` flag is needed. Those
old flags remain accepted as no-ops for compatibility.

Downloads are staged before copying, so fetch failures and missing templates do
not change the destination. Existing folders are updated file by file, without
promising an atomic or crash-safe project update. Filesystem safeguards prevent
writes through local symlink parents or over the versioned tooling directories.
The full upstream tree is checked before staging: ambiguous file/directory names
that collide after Unicode normalization or case folding (for example `Config`
and `config/file`) are rejected, including on case-sensitive hosts.
A lock prevents concurrent syncs; it does not inspect project state. Remove a
stale `.sync.lock/` only after confirming that no sync is running.

## Git isolation

Downloaded folders such as `harness/` and `harness-kanban/` are entirely ignored
by the parent repository, including subsequent local changes. A template's own
`.gitignore` cannot override that parent exclusion. Explicit `git add -f` can
still bypass Git ignore rules.

The versioned `sync.sh`, `scripts/`, tests and previous root-level Harness
baseline remain in place. They are not copied into downloads or modified by the
sync. Template names use lowercase letters, digits, underscores and hyphens,
starting with a letter or digit. Tooling/baseline names (`agents`, `config`,
`data`, `scripts`, `skills`, `tests`, `upstream`) are reserved to avoid overwriting
tracked files.

## Requirements and tests

Downloading requires Bash, Git, Python 3.11+ and `ruamel.yaml`. The script tries
`python3`, then `python`, and uses the first compatible interpreter. If the YAML
library is missing, it uses `uv` to supply the pinned dependency in an isolated
cache. It does not install system packages. Without either the library or `uv`,
use the container below.
Keep `scripts/sync_template.py` beside the launcher in its `scripts/` directory.
A missing importer is reported before fetching anything.

From the repository root, build the container and run the offline tests:

```bash
docker build -t workers-template-tests template
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -e PYTHONDONTWRITEBYTECODE=1 -v "$PWD:/workspace:ro" workers-template-tests
docker run --rm --network none -v "$PWD:/workspace:ro" \
  workers-template-tests bash -n template/sync.sh
```

The same image can run sync with access to the local workers and the destination:

```bash
docker run --rm -it --user "$(id -u):$(id -g)" \
  -e PYTHONDONTWRITEBYTECODE=1 -v "$PWD:/workspace" \
  workers-template-tests bash template/sync.sh
```

The download tests use temporary local Git repositories and cover templates
without agents/skills (including a Harness + Kanban fixture), byte-for-byte
copies when no worker matches, local Compose paths and Cargo commands, preserved
settings and comments, executable bits, hidden files, symlinks, repeat downloads,
Git isolation, previews, fetch failures and Python command selection. The suite
also checks the retained root-level Harness baseline. No template is started.
