# worktree

Git worktree lifecycle for parallel agents. `worktree::create` mints an
isolated, locked worktree off any ref and registers it in a cross-agent
registry; agents work inside it through their turn's filesystem scope
(`metadata.fs_scope.root`); `worktree::land`
rebases the branch onto a target, runs an optional test gate through the
[shell](https://github.com/iii-hq/workers/tree/main/shell) worker, fast-forwards the target with an atomic
compare-and-swap, and cleans up. Every lifecycle change emits a trigger type
sibling workers can subscribe to, so a Slack bot can announce lands or a
harness can react when a sibling's branch merges.

## Install

```bash
iii worker add worktree
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii`
boot. The land test gate delegates to `shell::exec`, so install the shell
worker alongside it:

```bash
iii worker add shell
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions};
use iii_sdk::protocol::TriggerRequest;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // 1. Mint an isolated worktree and hand its path to an agent.
    let wt = iii.trigger(TriggerRequest {
        function_id: "worktree::create".into(),
        payload: json!({ "repo_path": "/home/me/project", "session_id": "sess_1" }),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;
    println!("agent fs_scope root: {}", wt["path"]);

    // 2. ... the agent commits work inside the worktree ...

    // 3. Land it: rebase onto main, gate on tests, ff-merge, clean up.
    let job = iii.trigger(TriggerRequest {
        function_id: "worktree::land".into(),
        payload: json!({
            "worktree_id": wt["worktree_id"],
            "target_branch": "main",
            "test_cmd": "cargo test",
        }),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;
    println!("queued land job: {}", job["job_id"]);
    Ok(())
}
```

The land completes asynchronously; subscribe to `worktree::landed` /
`worktree::land-blocked` (below) or poll `worktree::get` for the outcome.

## Configuration

Runtime settings live in the `configuration` worker under id `worktree` and
hot-reload on change (a `prune_schedule` change re-binds the cron live).

```yaml
worktree_root: "~/.iii/worktrees"   # where managed worktrees are created
branch_prefix: "iii/"               # auto-minted branch names
branch_naming: "id"                 # or "codename" for <adjective>-<noun>-<4hex> names
prune_schedule: "0 0 * * * *"       # six-field cron for the reconcile sweep
prune_expire_hours: 72              # idle hours before a clean, unclaimed worktree is prunable
land_queue: "worktree-land"         # engine queue name for land jobs
max_land_retries: 3                 # rebase retries when the target branch moves mid-land
git_timeout_ms: 60000               # per-git-subprocess bound
test_timeout_ms: 600000             # land test gate bound
provision:
  copy_ignored: false               # replicate gitignored files into new worktrees
  include: []                       # globs; empty = every ignored path
  exclude: []                       # globs subtracted from the copy
  max_copy_bytes: 2147483648        # total budget; entries beyond it are skipped
gates:
  allow_remove: true                # worktree::remove
  allow_force: false                # remove force, claim/release force, land force_restart
  allow_branch_delete: true         # remove delete_branch and the land finalize cleanup
  allow_land: true                  # worktree::land
  allow_prune: true                 # worktree::prune (including the cron sweep)
  land_targets: ["*"]               # globs; pin to ["main"] to allow only mainline lands
  repos: ["*"]                      # globs over canonical repo paths worktrees may branch from
  max_worktrees_per_repo: 0         # live-worktree budget per repo at create time; 0 = unlimited
```

### Copy-ignored provisioning

With `provision.copy_ignored: true`, every create spawns a background task
that replicates the source repo's gitignored files (.env files, caches,
local settings) into the new worktree: cheapest copy per platform (APFS
clones via `cp -c` with a plain fallback on macOS, `--reflink=auto` on
Linux), VCS metadata directories and the managed worktree root always
excluded, bounded by `max_copy_bytes`. Provisioning is best-effort by
design: the create response never waits on it, failures only log, and a
retried copy skips files that already landed.

### Pull request checkouts, dev ports, integration

`worktree::create` also takes `pr: <number>`: it fetches
`refs/pull/<n>/head` from `origin` (GitHub layout) and branches
`<prefix>pr-<n>` at it, so reviewing a PR gets its own isolated checkout.
Every worktree carries an advisory `dev_port` (10000 + hash of the id mod
10000): deterministic, collision-improbable across parallel worktrees, and
never reserved anywhere; point dev servers at it to avoid port fights.
`worktree::status` reports `integrated` with an `integration_reason`
(`same_commit`, `ancestor`, `no_added_changes`, `trees_match`,
`merge_adds_nothing`, `patch_id_match`) so squash- or rebase-landed branches
read as merged, and `worktree::prune` removes integrated-but-ahead worktrees
instead of holding them forever.

### Removal and trash

`worktree::remove` never blocks on a large directory delete: the worktree
is renamed into a `.trash` staging area under `worktree_root` (instant),
the git admin entry is pruned, and a detached task deletes the staged
directory in the background; if the rename fails the removal falls back to
a synchronous `git worktree remove`. Entries stranded by a crash are
cleared by the next prune's trash sweep, which runs even when
`gates.allow_prune` is off (it is remove's crash recovery, not part of
prune's destructive surface). Unforced removals also probe for processes
holding files open under the worktree and refuse with `W222` rather than
deleting a directory out from under a running dev server.

### Gates

Every mutating handler checks its gate before anything else, so an operator
can turn off whole classes of destruction with one config flip and the
change hot-reloads. Denials are structured: `W500` (operation disabled),
`W501` (force paths disabled), `W502` (land target not allowed), and every
message names the exact key to flip, e.g. `worktree::remove is disabled by
configuration; set gates.allow_remove: true to enable`. Force is the one
gate that ships closed: takeovers, forced removals, and land restarts are
opt-in.

Creation is gated too: `gates.repos` pins which repositories worktrees may
branch from (globs over the canonicalized path, `W503` otherwise), and
`gates.max_worktrees_per_repo` caps live worktrees per repository at create
time (`W504` names the current count; orphaned records do not count, and
narrowing either gate later never breaks existing worktrees).

Gates constrain this worker's own surface only. Restricting what an agent
can run in a shell (`rm`, `git branch -D`, ...) is the shell worker's
allowlist/denylist job; pair both for defense in depth.

### Filesystem scope interplay

Agents are scoped through `metadata.fs_scope.root` on the turn: the console
picker (or `harness::send` options) sets it to the worktree path, and the
harness stamps `fs_scope: { root, grants }` onto every scoped `shell::*` /
`coder::*` call the agent makes. The land test gate does the same directly:
it runs `shell::exec` with the worktree as `cwd` and as the `fs_scope` root,
so the shell worker enforces the scope for gated tests exactly as it does
for agent calls.

`worktree_root` must still resolve inside the shell worker's
`fs.host_roots`: a root outside the jail fails with `S215` (a path inside
the jail but outside the scope root fails with `S220`), and the test gate
surfaces the `S215` misconfiguration as a `W411` land block naming it.

### Engine queue config

Land jobs serialize per repository through an engine FIFO queue. Add this
block to the engine config (the `repo_key` group field is carried on every
job payload):

```yaml
queue_configs:
  worktree-land:
    type: fifo
    message_group_field: repo_key
    concurrency: 1
    max_retries: 2
```

Without it the queue still delivers, but ordering falls back to this
worker's in-process per-repo lock, which only holds within a single
instance.

## Functions

- `worktree::create` — mint a locked worktree off a ref; auto-claims when
  `session_id` is given.
- `worktree::list` — registry view with filters and optional git status;
  reports unmanaged worktrees of a repo without adopting them.
- `worktree::get` — one worktree with status.
- `worktree::validate` — picker-style path validation; reconciles records
  whose directories are gone.
- `worktree::claim` / `worktree::release` — session ownership with `force`
  takeover and `W210`/`W211` mismatch errors.
- `worktree::status` — clean, ahead/behind, staged/unstaged/untracked,
  diffstat, rebase-in-progress.
- `worktree::remove` — guarded removal (`W220` dirty, `W221` unmerged,
  `W222` busy), instant trash staging with background delete, optional
  branch deletion.
- `worktree::prune` — reconcile sweep, cron-bound; `{}` payload works.
- `worktree::land` — queue a land job; returns `{ job_id, queued }`.
- `worktree::land-step` — internal queue consumer; not agent-callable.

Errors carry stable `W###` codes (`src/error.rs`). One instance per engine:
iii-state has no compare-and-set, so run a single worktree worker (shard by
repo if you need more).

## Custom trigger types

| Trigger type | Fires when | Payload highlights |
|---|---|---|
| `worktree::created` | A worktree was minted | `worktree_id`, `path`, `branch`, `base_sha` |
| `worktree::claimed` | A session took ownership | `session_id`, `previous_session_id` |
| `worktree::released` | A claim was released | `session_id` |
| `worktree::removed` | A worktree was removed or pruned | `branch_deleted` |
| `worktree::landed` | A land fast-forwarded the target | `target_branch`, `merged_sha` |
| `worktree::land-blocked` | A land ended blocked | `reason`, `code`, `conflict_files`, `exit_code` |

Bindings accept optional equality filters: `repo_path`, `worktree_id`,
`session_id`.

```rust
use iii_sdk::protocol::RegisterTriggerInput;

iii.register_trigger(RegisterTriggerInput {
    trigger_type: "worktree::landed".to_string(),
    function_id: "notify::on-land".to_string(),
    config: serde_json::json!({ "repo_path": "/home/me/project" }),
    metadata: None,
})?;
```

## Conflict resolution flow

When a land's rebase hits conflicts, the rebase is left IN PROGRESS inside
the worktree and a `worktree::land-blocked` event fires with
`reason: "rebase_conflict"` and the conflicted paths. The owning agent
resolves in place (its filesystem scope already roots at the worktree),
finishes with `git rebase --continue`, and reruns `worktree::land`. A rerun
while the rebase is still unresolved returns `W402`; pass
`force_restart: true` to abort the rebase, reset, and start the land over.

## HTTP endpoints

With the `http` worker (or any provider of the `http` trigger type)
installed, the read surface is also served over HTTP: `worktree/list`,
`worktree/get`, `worktree/status`, and `worktree/validate` (POST, JSON
bodies matching the function schemas). Registration is best-effort; without
an http provider the worker logs a warning and keeps serving the bus.

## Local development and testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134
cargo test
```

The end-to-end test boots a real engine and self-skips when `iii` is not on
PATH. Regenerate wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.
