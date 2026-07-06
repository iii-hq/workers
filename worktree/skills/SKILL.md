---
name: worktree
tags: git, worktree, parallel, land, merge, rebase
description: >-
  Mint isolated git worktrees for parallel agent work, track ownership in a
  cross-agent registry, and land finished branches back onto a target with a
  queued rebase, test gate, and atomic fast-forward merge.
---

# worktree

The worktree worker gives every agent its own isolated checkout of a shared
repository. Instead of two sessions fighting over one working tree,
`worktree::create` mints a locked `git worktree` on a fresh branch, records
who owns it, and returns a path to use as the agent's working directory.
When the work is done, `worktree::land` automates the merge-back: rebase
onto the target branch, run an optional test command, fast-forward the
target atomically, and clean the worktree up. Lands are serialized per
repository through an engine FIFO queue, so parallel agents never race a
merge.

The land test gate delegates to `shell::exec`, so the shell worker must be
installed and `worktree_root` must sit inside its `fs.host_roots` jail.
Landing never pushes to remotes; it moves local branches only.

## When to Use

- A task should run in isolation from the primary checkout or from other
  agents working on the same repo (`worktree::create`, then use the
  returned `path` as the agent's working directory).
- You need to see which worktrees exist, who owns them, and whether they
  carry uncommitted or unlanded work (`worktree::list`,
  `worktree::status`).
- A finished branch should merge back into `main` (or any target) with
  tests enforced first (`worktree::land` with `test_cmd`).
- A land was blocked on conflicts and the agent resolved them in place:
  finish the rebase, then rerun `worktree::land`; pass `force_restart` to
  abort and start over instead.
- Hand a worktree between sessions (`worktree::claim`,
  `worktree::release`) or clean up abandoned ones (`worktree::remove`,
  `worktree::prune`).

## Boundaries

- Not a PR or code-review tool: landing fast-forwards a local branch; it
  never pushes, opens pull requests, or talks to a forge.
- Worktrees created out of band are reported as unmanaged and never
  adopted; only worktrees minted by `worktree::create` are managed.
- One worker instance per engine: the registry has no compare-and-set, so
  multi-instance deployments must shard by repository.
- Merges are fast-forward only (after the rebase); there is no merge-commit
  or squash strategy.
- For running commands or editing files inside a worktree, use the `shell`
  worker with the worktree path as `cwd`; this worker only manages the
  worktrees themselves.

## Functions

- `worktree::create` — mint a locked, isolated worktree off a base ref;
  auto-claims for `session_id` when given.
- `worktree::list` — registry view, filterable by repo or session, with
  optional git status per worktree.
- `worktree::get` — one worktree with status.
- `worktree::validate` — check a path is a live managed worktree;
  reconciles records whose directories were removed by hand.
- `worktree::claim` — take session ownership (`force` to take over).
- `worktree::release` — release ownership (`force` to override).
- `worktree::status` — clean flag, ahead/behind, staged/unstaged/untracked
  counts, diffstat, rebase-in-progress.
- `worktree::remove` — remove a worktree; refuses dirty or unlanded work
  unless forced; can delete the branch.
- `worktree::prune` — sweep: drop records whose directories are gone and
  remove clean, unclaimed, expired worktrees (cron-bound, `{}` payload).
- `worktree::land` — queue the rebase / test / fast-forward / cleanup
  pipeline; returns a `job_id` immediately.
- `worktree::land-step` — internal queue consumer that executes land
  phases; never call it directly (denied to agents).

Errors carry stable `W###` codes; the ones worth branching on are `W210`
(already claimed), `W220` (dirty), `W221` (unmerged work), `W401` (land
already queued), `W402` (unresolved rebase from a previous land), and the
land-block reasons carried on events (`W410` conflict, `W411` tests,
`W412` target kept moving, `W413` target checked out dirty).

## Reactive triggers

Register a `worktree::*` trigger when a different worker should react to
lifecycle changes without polling `worktree::list` — announce lands in
chat, start a follow-up agent when a sibling's branch merges, or alert on
blocked lands.

Reach for it when:

- A land's outcome should drive the next step (`worktree::landed`,
  `worktree::land-blocked` with `reason` and `conflict_files`).
- Ownership changes matter to an orchestrator (`worktree::claimed`,
  `worktree::released`).
- Cleanup should cascade (`worktree::removed`).

The caller of `worktree::land` gets only `{ job_id, queued }` back; the
outcome arrives on these triggers, so a landing workflow should always bind
one instead of polling.

### How to bind

1. Register a handler: `registerFunction('notify::on-land', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'worktree::landed',
  function_id: 'notify::on-land',
  config: {
    // optional equality filters:
    // repo_path, worktree_id, session_id
  },
})
```

All six types accept the same three optional filters; unknown config keys
are rejected at registration. For event payload shapes, call
`get function info` on the trigger type.
