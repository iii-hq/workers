---
type: index
name: github
description: >-
  Operate GitHub through the gh CLI — typed github::* functions for pull
  requests, issues, repos, Actions runs/workflows, releases, and search,
  plus github::exec / github::api escape hatches for everything else.
---

# github

The github worker wraps the GitHub CLI (`gh`). Thirty typed functions cover
the high-traffic surface (pr, issue, repo, run, workflow, release, search);
`github::exec` runs any other gh command verbatim, and `github::api` reaches
any GitHub REST endpoint. Auth comes from the worker's GH_TOKEN configuration
or the host's ambient `gh auth login` state. There is no local checkout:
every repo-scoped call takes an explicit `repo: "owner/name"`.

## When to Use

- Check a PR before landing: `github::pr::view`, `github::pr::checks`
  (failing/pending checks are data, not errors), `github::pr::diff`.
- Open or shepherd a PR: `github::pr::create` / `review` / `merge`.
- File and triage issues: `github::issue::create` / `list` / `comment` /
  `close`.
- Watch or poke CI: `github::run::list` / `view` / `rerun` / `cancel`;
  dispatch with `github::workflow::run`.
- Cut or inspect releases: `github::release::create` / `list` / `view`.
- Find things org-wide: `github::search::repos` / `issues` / `prs` / `code`
  (qualifiers like `repo:o/r is:open` go in the query string).
- Anything gh does that has no typed function → `github::exec { args: [...] }`.
- Any REST endpoint → `github::api { path: "repos/o/r/…", jq? }`.

## Boundaries

- Needs the gh CLI on the worker host; a missing binary errors at call time
  with code `gh_not_found` (functions still register without it).
- Mutations (create/edit/merge/comment/review/close/rerun/cancel/workflow
  run/release create) and both escape hatches are approval-gated by default;
  the read-only surface is allowed (see iii-permissions.yaml).
- Curated functions error on a non-zero gh exit (the message carries gh's
  stderr); `github::exec` returns exit_code/stderr/timed_out as data instead.
- Output is capped per stream (default 1 MiB) with `*_truncated` flags;
  per-call `timeout_ms` clamps to `max_timeout_ms` (default 120 s; 30 s when
  omitted).
- `github::api` paths take concrete owner/repo values — `{owner}`/`{repo}`
  placeholders need a checkout the worker does not have.

## Functions

- `github::pr::*` — list, view, create, edit, merge, comment, review, diff,
  checks.
- `github::issue::*` — list, view, create, edit, comment, close.
- `github::repo::*` — view, list.
- `github::run::*` — list, view, rerun, cancel (Actions runs).
- `github::workflow::*` — list, run (workflow_dispatch).
- `github::release::*` — list, view, create.
- `github::search::*` — repos, issues, prs, code.
- `github::exec` — `{ args, stdin?, timeout_ms? }` → the full outcome as data
  (`stdout`, `stderr`, `exit_code`, `timed_out`, truncation flags).
- `github::api` — `{ path, method?, fields?, body?, jq?, paginate?,
  timeout_ms? }` → parsed JSON.
