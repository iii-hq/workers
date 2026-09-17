# Download templates

`sync.sh` downloads the contents of `iii/<name>/` from
[`iii-hq/templates`](https://github.com/iii-hq/templates) into `template/<name>/`.
The default name is `harness`.

The sync only downloads files. It does not validate the project's structure,
check whether it is running, configure workers, install dependencies, or execute
anything from the template. Follow the downloaded template's own instructions
to configure and run it.

## Usage

From the repository root:

```bash
cd template

# Download iii/harness/ into template/harness/.
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
bits are preserved. Nothing is rewritten, no local configuration seeds are
added, and no sync manifest is generated inside the download.

Only the selected folder's contents are downloaded: files referenced elsewhere
by an installer manifest are not resolved or added. Git submodules are external
references rather than folder contents; they produce an explicit error instead
of an incomplete download.

There is no requirement for `agents/`, `skills/`, Markdown, Compose, or any other
project file. A template folder only needs to exist at the selected Git ref.

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

Downloading requires only Bash, Git and Python 3.11+ (standard library). The
script tries `python3`, then `python`, and uses the first compatible interpreter.
Keep `scripts/sync_template.py` beside the launcher in its `scripts/` directory.
A missing importer is reported before fetching anything.

From `template/`, run the offline download tests:

```bash
bash -n sync.sh
python3 -m unittest discover -s tests -p test_sync.py -v
# Use python instead of python3 if that is your interpreter's command name.
```

To also run the structural checks for the retained root-level Harness baseline:

```bash
uv run --with PyYAML==6.0.3 python -m unittest discover -s tests -v
```

The download tests use temporary local Git repositories and cover templates
without agents/skills (including a Harness + Kanban fixture), byte-for-byte
copies, executable bits, hidden files, symlinks, repeat downloads, Git isolation,
previews, fetch failures and Python command selection. No template is started.
