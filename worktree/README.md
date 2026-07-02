# worktree

Git worktree lifecycle for parallel agents. `worktree::create` mints an
isolated, locked worktree off any ref and registers it in a cross-agent
registry; agents work inside it via their `working_dir`; `worktree::land`
rebases the branch onto a target, runs an optional test gate through the
[shell](../shell) worker, fast-forwards the target with an atomic
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
    println!("agent working_dir: {}", wt["path"]);

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
prune_schedule: "0 0 * * * *"       # six-field cron for the reconcile sweep
prune_expire_hours: 72              # idle hours before a clean, unclaimed worktree is prunable
land_queue: "worktree-land"         # engine queue name for land jobs
max_land_retries: 3                 # rebase retries when the target branch moves mid-land
git_timeout_ms: 60000               # per-git-subprocess bound
test_timeout_ms: 600000             # land test gate bound
```

### Shell jail interplay

`worktree_root` must resolve inside the shell worker's `fs.host_roots`.
The land test gate runs `shell::exec` with the worktree as `cwd` and
`base_dir`; a root outside the jail makes every gated call fail with `S215`,
which this worker surfaces as a `W411` land block naming the misconfiguration.
The same applies to harness-stamped `shell::*` and `coder::*` calls agents
make from inside a worktree.

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
- `worktree::remove` — guarded removal (`W220` dirty, `W221` unmerged),
  optional branch deletion.
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
resolves in place (its `working_dir` already points at the worktree),
finishes with `git rebase --continue`, and reruns `worktree::land`. A rerun
while the rebase is still unresolved returns `W402`; pass
`force_restart: true` to abort the rebase, reset, and start the land over.

## Local development and testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134
cargo test
```

The end-to-end test boots a real engine and self-skips when `iii` is not on
PATH. Regenerate wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.
