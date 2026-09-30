---
name: ide
tags: shell, exec, filesystem, jobs, sandbox
description: >-
  Run Unix commands and structured filesystem ops from the iii engine: exec,
  background jobs, and a structured fs (ls/stat/mkdir/rm/chmod/mv/grep/sed/
  read/write, jailed only if fs.host_roots is configured), all forwardable
  into a sandbox microVM.
---

# shell

The shell worker is the single door every agent uses to touch the OS: run a
build, call a CLI, read a file, list a directory. Routing it all through
`shell::*` and `shell::fs::*` keeps a denylist, timeouts, output
caps, and an optional host-root jail (unjailed by default — see Boundaries)
in one enforceable place. Both surfaces take an
optional `target` field that forwards the call into a live `iii-sandbox`
microVM, so the same denylist gates host and sandbox execution alike.

Host-targeted `shell::exec` is not an isolation boundary. The denylist is a
regex tripwire on `argv.join(" ")`, and any interpreter (`sh`,
`node`, `python3`) can construct any forbidden token at runtime to bypass it.
Run untrusted input with `target: { kind: "sandbox", sandbox_id }`. Prefer the
`shell::fs::*` backends over `exec`-ing `ls`/`stat`/`grep`/`rg`: they stay
in-process, honor `fs.host_roots` when it's configured, and return
structured results.

Sandbox forwarding (and `shell::fs::*` into a VM) requires the `iii-sandbox`
worker; `iii trigger compose::add worker=ide` does not pull it in. To surface `shell::*` to LLM
agents, pair with the `skills` worker.

## When to Use

- Run a one-shot command and block for its full output: `git status`, `wc`,
  `head`, a quick compile probe (`shell::exec`).
- Kick off long work (build, watcher, wide grep) without blocking the turn,
  then poll for completion (`shell::exec_bg` + `shell::status`).
- Survey or terminate in-flight background jobs (`shell::list`, `shell::kill`).
- List, stat, or read files with structured output instead of shelling out to
  `ls`/`stat`/`cat` (`shell::fs::ls`, `shell::fs::stat`, `shell::fs::read`).
- Search or rewrite across a tree without spawning `rg`/`sed`
  (`shell::fs::grep`, `shell::fs::sed`).
- Create, move, remove, or re-permission paths (jailed only if
  `fs.host_roots` is set)
  (`shell::fs::mkdir`, `shell::fs::mv`, `shell::fs::rm`, `shell::fs::chmod`).
- Persist a generated artefact, or bootstrap files into a sandbox, by streaming
  bytes to a path (`shell::fs::write` with a `target`).

## Boundaries

- Host `shell::exec` is not a security sandbox: the denylist is bypassable by
  any interpreter. Run untrusted commands with `target: sandbox`
  (needs `iii-sandbox`).
- `shell::fs::*` honors `fs.host_roots` as a jail WHEN it's set — empty (the
  shipped default) means unjailed, confined only by `fs.denylist_paths` — and
  always refuses denylisted paths regardless; paths must be absolute (unless
  jailed, where a relative path resolves against the primary root) and
  symlinks are never followed.
- In a harness session every call carries a trusted `fs_scope.root`. Under
  the `workspace` boundary (harness `filesystem_boundary`), `coder::*`,
  `shell::fs::*` and an exec `cwd` refuse paths outside that root and the
  session's grants (`C220`/`S220`, `C215`/`S215`); under `configured_roots`
  the root is only the default directory for relative paths.
- With `fs.exec_confinement: landlock`, a session's host `shell::exec` /
  `exec_bg` can write only under its root, its grants, the repository's git
  dir and `fs.exec_writable` (`/tmp`, `/dev`, caches, toolchains); reads stay
  open. Without Landlock such a call fails closed with `S222`.
- Sandbox-backed background jobs cannot be hard-killed: `shell::kill` flips the
  record but the in-VM process runs until its `timeout_ms` (or `sandbox::stop`).
- Not for inlining file bytes into an LLM tool result: `shell::fs::read`/
  `write` move bytes over channels; use the `harness` worker's
  `harness::fs::read_inline` wrapper for inline reads on the web surface.
- No batch or glob form for single-path ops (`mv`, `rm`, `stat`, …); loop in the
  caller.
- Not a package manager, editor, or migration tool; for SQL use the `database`
  worker.

## Functions

- `shell::exec`: run a command in the foreground and return its
  stdout, stderr, exit code, and timing; blocks until exit or timeout. Sandbox
  execution is fully valid (`target: { kind: "sandbox", sandbox_id }`); only the
  host-only override fields — `stdin` (string piped to the program's stdin, then
  EOF), plus `cwd`/`env` — are rejected with `S210` when supplied on a sandbox
  target, because the sandbox exec protocol does not forward them.
- `shell::exec_bg`: spawn a command as a background job and return
  a `job_id` immediately. Host-targeted jobs run until they exit or `shell::kill`
  terminates them — unbounded by default, capped only when the operator sets a
  positive `max_bg_timeout_ms` (default `0` = unbounded), after which a runaway
  job is killed and its status becomes `killed`. Sandbox jobs honor `timeout_ms`.
  Same optional host-only `stdin` as `shell::exec`.
- `shell::status`: fetch one job's full record: state, exit code, and captured
  stdout/stderr. A missing id (never existed or aged out) returns an `S211`
  ("no such job") error.
- `shell::list`: enumerate current jobs as lightweight summaries (no argv,
  stdout, or stderr).
- `shell::kill`: terminate a running background job by `job_id`.
- `shell::config-status` *(operator/automation only — not agent-callable)*:
  report the last hot-reload outcome — `last_outcome` (`applied`/`rejected`),
  `last_error`, and `rejected_reloads` (count since boot). A rejected outcome or
  non-zero count means a stored config was refused and shell is enforcing an
  older policy than the central store. Takes no arguments.
- `shell::fs::ls`: list a directory's entries with structured metadata.
- `shell::fs::stat`: read one path's metadata (size, mode, symlink flag).
- `shell::fs::mkdir`: create a directory, optionally with missing parents. Returns `{ created: bool, path: string, already_existed: bool }`.
- `shell::fs::rm`: remove a file or directory, optionally recursive. Returns `{ removed: bool, path: string, was_present: bool }`.
- `shell::fs::chmod`: change a path's mode, and optionally its uid/gid. Returns `{ entries_changed: u64, path: string, recursive: bool }`. **Note**: the field was renamed from `updated` to `entries_changed` — callers relying on `updated` must migrate.
- `shell::fs::mv`: rename or move one path within the jail. Returns `{ moved: bool, src: string, dst: string, overwrote: bool }`.
- `shell::fs::grep`: recursive regex search across a tree, returning structured
  matches.
- `shell::fs::sed`: regex find-and-replace across one file or many.
- `shell::fs::write`: write a file. Simplest form is inline string `content`
  (host target only): `{ path, content: "file text" }`, with `mode` (octal,
  default `"0644"`) and `parents: true`. A `ContentRef` object in `content`
  instead streams large/staged payloads via a channel (temp file + atomic
  rename) and is **required** for sandbox targets. Batch form: pass
  `files: [{ path, content, mode?, parents? }, ...]` to write several files in
  one host call; the response then carries per-file `files: [{ path,
  bytes_written }]` (a single-file write leaves `files` empty).
- `shell::fs::read`: stream a file's bytes out through a channel.

Every `shell::fs::*` call accepts the same optional `target` as `exec`, so host
and sandbox share one wire shape; reads and writes move bytes over SDK channels
rather than inlining them.

## Live change feed (`shell::changed`)

A custom trigger type backed by a system-level directory watch. Bind it
with `config: { path: "/some/dir" }` to stream every change under that
directory — whoever made it: `coder::*` calls, `shell::exec` side
effects, or an editor outside the engine. Payload:
`{ path, kind, root, dir }` with `kind` ∈ `created | modified | deleted`,
`path` relative to `root`, `dir` true for directories (skip those when
opening files). Events coalesce per path in a short window
(create + write = `created`; deletion supersedes), `.git` internals are
filtered, and fan-out is fire-and-forget. Ignored paths (git-ignored, or
`data/`, `config/`, `.iii/`, `node_modules/` outside a repository) are not
delivered unless the binding sets `include_ignored: true` — a wake on a
project root would otherwise fire on the engine's own data writes. Read
content on demand via `coder::read-file` — the event deliberately carries
none.

## Code files (`coder::*`)

The shell worker also serves the `coder::*` code-file surface (formerly a
standalone worker) over the **same jail** (`fs.host_roots`): `coder::info`
(discover roots/caps first), `coder::read-file` (windowed + batch reads),
`coder::search` (add `respect_gitignore: true` to skip ignored files,
`fuzzy_paths: true` to rank path matches like an editor's quick open),
`coder::list-folder`, `coder::tree`, and the batched
`coder::create-file` / `coder::update-file` / `coder::delete-file` /
`coder::move`. Prefer these structured ops over editing files through
`shell::exec`. They return `C2xx` error codes (distinct from `shell::*`'s
`S2xx`); protected paths are the shared `code.non_accessible_globs`.
`coder::list-templates` lists the worker templates (`code.templates`: a local
`dir`, or a cached git clone) with their language and the compose containers
they need.
`coder::scaffold-worker { template, name, directory?, operation_id?, start_after? }`
writes one into a missing or empty folder whose last segment is `name`
(default `workers/<name>`), all or nothing, then adds it to the stack in the
same call (`compose::add` with the worker, its `start_after` and the
`requires` the stack lacks): a name the stack already has fails `C235`
before any write, and the result carries `operation_id` and `started`, or
`start_error` with the files kept. `start: false` writes the files only and
returns `compose_add`, the `compose::add` payload to send whole, adding
`start_after` to its entry and missing `requires` as more entries (never the
bare `worker` string form, which drops the scripts). It adds to the stack
the ide runs in, so a harness session without `compose::add`, or whose
`compose::add` the approval gate would not allow outright, gets the files
only, with a note (an explicit `start: true` is refused there); its own
`compose::add` then goes through approval as usual.

To find where or how something works, ask `coder::find-relevant` with the
question in plain words ("where does the harness stamp the filesystem scope
on coder calls?"). It asks the judge which folders, files and declarations
matter and returns files best first with verbatim excerpts and line ranges.
Use `coder::search` instead for an exact symbol, string or filename. Read
the returned excerpts, and `leads` via `coder::read-file`, before searching
again. Set `path` to the folder of the component the question is about
(`judge/src`, not the repository root): every level is a judge round trip
and the judge bills per token, so a whole-repo ask on a large repository
takes minutes and stops at the judge token budget (reason `token_budget`).
`incomplete` means partial coverage (narrow `path` and retry);
`unavailable` means no judge answered (fall back to `coder::search`). The
query, root-relative paths and file text go to the session's judge
provider, which may be hosted; protected, ignored and secret-looking files
never do, nor hidden entries below `path` (so never point `path` at a
dot-folder holding tokens). An excerpt with `partial` is only a byte span of
its lines: read the file before rewriting those lines.
