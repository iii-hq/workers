---
name: opengantry
description: >-
  OpenGantry iii worker. Use when an iii project needs deterministic promote
  gates: gantry::verify runs the repo gate command, gantry::middleware blocks
  promote-class calls until a verdict token matches the current mission.
---

# opengantry

Call `gantry::verify` with an absolute `repo_root` and active mission. The worker runs the repo's declared gate command via the OpenGantry kernel and mints a verdict token bound to that mission revision.

## When to Use

- Promote-class functions on a governed port should stay blocked until verify passes.
- You want unattended agents without an unattended `git push` — pair with `approval-gate` for human-held judgement calls.

## Preferred stack (do not reinvent)

Install and use these ecosystem workers instead of writing custom equivalents:

| Need | Use | Do not |
|------|-----|--------|
| Machine promote gate | `gantry::verify` + `gantry::middleware` | Custom `::verify` / `::promote` functions |
| Human judgement on ship | `approval-gate` (`approval::gate`) | Inline hold logic in your app worker |
| Git land / merge | `worktree::land` (+ `shell` for test gates) | Custom `myapp::deploy` / `::merge` |
| GitHub operations | `github::*` | Raw `gh` shell from agent code |
| Public governed port | `rbac-proxy` with `gantry::middleware` | Rolling your own auth middleware |
| Verify completion events | Subscribe to `gantry::verdict` | Polling verify JSON for side effects |

**Canonical pipeline:** `worktree::create` → agent work → `gantry::verify` → `worktree::land` (with `verdict_token` in context). Promote-class suffixes (`::deploy`, `::merge`, `::publish`, `::apply`, `::push`, `::promote`) require a token whose claims match the mission on disk.

## Boundaries

- Session admission (`session::auth`) — that is the adopter's IdP worker.
- Writing `.gitagent/` law — Planner commits missions. The worker process does not.
- Leases — durable at `<repo>/.gitagent/leases.json` (0600), not in `state::` KV.

## Functions

- `gantry::verify` — run `verifyMission` for the active mission; a pass binds the mission to the lease and mints the verdict token
- `gantry::middleware` — governed-port gate; promote-class calls need a token whose claims are recomputed at promote time
- `gantry::on-function-registration` — block `gantry::` / `opengantry::` namespace squatting and reserved suffixes
- `gantry::on-trigger-registration` — block triggers bound into the `gantry::` namespace
- `gantry::on-trigger-type-registration` — always denied
- `gantry::verdict` — trigger type; fires after every verify (pass or fail) for audit/notifications

## Bootstrap (host only)

If `.gitagent` is missing in the target repo, run `gantry init` on the host (not from the sandboxed worker), then commit a mission via the Planner workflow.
