---
name: editor
description: >-
  A code workspace shared with the user — open buffers, a file tree, diffs,
  fuzzy find, and saves that refuse to clobber. Backed by the shell worker for
  files and the state worker for the workspace record.
---

# editor

The editor worker holds a **workspace**: a folder, the buffers open against it,
and which folders are expanded. That record is shared, not private to you — a
file you open with `editor::open` appears in the user's tabs, and a file they
have open is one you can see with `editor::workspace::get`.

The unit is a folder, not a repository. Everything works in a plain directory;
git only adds a branch and change marks when there is one.

It performs no filesystem access itself: reads, writes, moves and listings are
delegated to `shell`, so anything `shell` refuses, `editor` refuses too.

## The workspace

Everything is relative to one **workspace**: a root folder, the buffers open
against it, and which folders are expanded. It is shared, so it is also how you
tell the user what you are doing.

- `editor::workspace::get` tells you the root and what is already open. Read it
  before assuming anything about where you are.
- `editor::workspace::open` repoints it. That changes what every surface sees,
  including the user's screen, so do not do it casually mid-task.
- `editor::tree` lists a folder and carries the expansion state; passing
  `expand` or `collapse` persists it for both surfaces.
- `editor::buffers::list` and `::close` are the tab set. Closing one closes it
  for the user too.

## When to Use

- You are about to write a file and want to show the change first
  (`editor::diff` — pure, no path required).
- You want the user to see what you are working on: `editor::open` puts it in
  their editor, which is better than pasting the file into the conversation.
- You need to know what they are looking at (`editor::workspace::get`).
- You are editing across several turns and must not clobber a concurrent edit
  (`editor::open` for the `version`, then `editor::save` with
  `expected_version`).
- You are renaming or moving something (`editor::move` — never `shell::fs::mv`
  when buffers may be open; see below).
- You know roughly what a file is called but not where it lives
  (`editor::find`); you want to find it by its *contents* (`editor::search`).
- You are creating or removing files (`editor::create`, `editor::delete` —
  delete closes any buffer beneath the path, which `shell::fs::rm` does not).
- You are committing or syncing (`editor::git::commit`, `::sync`, `::stash`,
  `::undo-commit`).
- You want a file as it was at a revision rather than as it is now
  (`editor::git::show`, HEAD by default). Pair it with `editor::open` to
  diff the two sides yourself rather than parsing a patch.
- You want the working tree as data rather than porcelain text
  (`editor::git::status`, `editor::git::hunks`).

## Boundaries

- `editor::find` matches **paths**; `editor::search` matches **contents**.
  Listing a directory outside the workspace is still `shell::fs::ls`.
- Not a full git client. Status, hunks, a file at a revision, tracked paths,
  commit, fetch/pull/push, stash and undo-last-commit are covered. Anything
  else — branch, checkout,
  rebase, cherry-pick, remote management — goes through `shell::exec`.
  `editor::git::sync` pulls `--ff-only`; a merge is deliberately not offered,
  because a conflicted tree under open buffers is a mess an editor cannot
  usefully show.
- Not a way around the jail. A path `shell` rejects comes back as `shell`'s
  error, unchanged.
- `editor::save` writes the **whole file**. It is not a patch applier — build
  the complete new content, then save it.
- Binary files are refused, not mangled.

## Functions

- `editor::workspace::open` — point the workspace at a folder; returns the
  buffers and expanded folders remembered for it.
- `editor::workspace::get` — the active root, open buffers and expanded
  folders, as every surface sees them.
- `editor::tree` — list a folder, carrying and persisting expansion state.
- `editor::open` — read a text file and record it as an open buffer.
- `editor::save` — whole-file write, guarded against a concurrent change.
- `editor::buffers::list` — the tab set.
- `editor::buffers::close` — close one buffer; the file on disk is untouched.
- `editor::move` — move or rename, rewriting every buffer and expanded folder
  at or under the path.
- `editor::create` — create a file or folder, parents included.
- `editor::delete` — remove a path and close any buffer it held.
- `editor::find` — fuzzy file finder over paths, ranked basename-first.
- `editor::search` — search file contents, grouped by file.
- `editor::diff` — unified patch between two texts; pure, nothing is read.
- `editor::git::status` — branch, upstream, ahead/behind, one row per changed
  path.
- `editor::git::hunks` — what changed in one file, as ranges plus a patch.
- `editor::git::show` — a file's contents at a revision, HEAD by default.
- `editor::git::commit` — stage and commit.
- `editor::git::sync` — fetch, fast-forward pull, or push.
- `editor::git::stash` — stash the working tree, or pop the most recent stash.
- `editor::git::undo-commit` — undo the last commit, keeping its changes
  staged.

Every path is root-relative unless it is absolute. The `editor::git::*`
functions fail outside a repository, which is an absent overlay rather than a
broken workspace.

## The two rules that prevent data loss

**Save against the version you opened at.**

1. `editor::open` returns `version` (an opaque version of the content) and
   `mtime`.
2. Pass `version` back as `expected_version` on `editor::save`. Prefer it to
   `expected_mtime`: mtime resolution is one second, so two writes inside the
   same second are indistinguishable and the later one wins silently.
   `expected_mtime` still works and is honoured when `expected_version` is
   absent; when both are sent, `expected_version` is the guard.
3. If the content changed in between, **nothing is written**: the response
   carries `conflict: true`, the current `disk_version` and `disk_mtime`, and
   `conflict_patch` — a diff from what is on disk now to what you tried to
   write.

Re-open, reconcile against that patch, and save again with the fresh version —
or use the `version` a successful `editor::save` returns as the guard for the
next one, without re-opening. Do not retry with the guard omitted to force it
through; that is exactly the clobber it exists to prevent. Omit it only when
creating a new file.

**Move through `editor::move`, not `shell::fs::mv`.**

`editor::move` rewrites every open buffer and expanded folder at or under the
path. `shell::fs::mv` does not, so buffers keep pointing at the old location
and the next save writes them back there — silently recreating the folder that
was just moved.

## Reading a response

- `editor::diff` — `identical: true` means the texts match. `truncated: true`
  means a side was over `max_diff_bytes` and **no diff was computed**; it does
  not mean "no changes".
- `editor::open` — `truncated: true` means the file was over `max_file_bytes`
  and you hold only its beginning. It is deliberately *not* recorded as a
  buffer, and saving it back is refused, because that would delete the rest.
- `editor::find` — `from_git: false` means the folder is not a repository and
  candidates came from the directory walk. `truncated: true` means only the
  first `max_find_candidates` paths were ranked; narrow the query.
- `editor::git::hunks` — empty `hunks` with `untracked: true` means git has
  never seen the file, so there was nothing to compare against.
- `editor::git::status` failing with "not a git repository" is an absent
  overlay, not a broken workspace. Everything else still works.
- `editor::search` — paths come back root-relative, like every other
  function here. `truncated: true` means the search stopped at
  `search_max_matches`.
- `editor::git::commit` — `committed: false` with a summary is "nothing to
  commit", not a failure. Do not retry it.
- `editor::git::show` — `exists: false` with empty content means the path is
  absent at that revision, which is what a newly added file looks like. It
  is not an error.

## Reactive triggers

The worker publishes one custom trigger type, `editor::changed`, which fires
after a file in the workspace changes — whoever changed it. It does not require
the writer to have called this worker: a `harness::hook::post-trigger` hook on
the `shell::*` and `coder::*` write paths turns any filesystem call into an
event. The hook is advisory and fail-open, so it never delays or denies the
write that produced it.

Bind it when a *different* worker or surface should follow edits as they land:
mirroring the workspace into a viewer, reacting to an agent's writes without
polling `editor::git::status`, or annotating a file the moment it moves.

Do not bind when you made the write yourself — `editor::save` already returns
`added`, `removed` and the new `version`.

### How to bind

1. Register a handler: `registerFunction('my-worker::on-edit', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'editor::changed',
  function_id: 'my-worker::on-edit',
})
```

Bindings take no config, and every subscriber gets every event. Delivery is
fire-and-forget: a slow or absent subscriber is logged and skipped rather than
retried. The event's `patch` is capped and sets `truncated` when it was cut —
call `editor::git::hunks` when you need the whole diff. For the payload shape,
call `get function info` on the trigger type.
