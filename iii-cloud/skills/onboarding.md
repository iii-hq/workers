---
name: iii-cloud/onboarding
description: >-
  The eleven steps from "put this on iii cloud" to a deploy that is Ready, and
  who does each one.
---

# Onboarding a project to iii cloud

Plan first. Before you call anything, tell the person the steps, who does
each, and that you only need them for two browser moments: the device approval
and the GitHub authorization. Then do each step and say when you are done with it.

Keep the page open beside the chat:

```
ade::workspace::open {screen: "ext:iii-cloud"}
```

A chat message with the marker `iii-cloud hand-off <id8>` comes from the page. Call `iii-cloud::handoff::brief {handoff_id}`, read its `kind` (`ready`, `deploy_fix`, `import` or `pull_request`), and continue at the step it names.

## 1. Connect

Call `iii-cloud::status`. When `connection.status` is `connected`, go on.

Otherwise call `iii-cloud::connect {method: "browser"}`. The answer has a
`user_code`. Tell the person to type it at the cloud's device page. Never
build a link with the code in it. Then wait for the sign-in:

```
harness::observe {function_id: "iii-cloud::on-change",
  filters: {"input.kind": {"$eq": "connection"}, "input.terminal": {"$eq": true}}}
```

With `BROWSER_SIGN_IN_UNAVAILABLE`, the cloud is too old for browser sign-in.
Offer an API key. The person saves it on the page. Then call
`iii-cloud::connect {method: "api_key", secret: "<the secret's name>"}`. The
worker finds the team and the environment from the key. A key cannot import a
project or change settings, so tell the person what it cannot do.

## 2. Resolve the project

Call `iii-cloud::project::resolve {path}` with the project's folder. Read
`project`, `environment` and `importable`.

- Found: go to step 3.
- Several teams and no match: `team` is null. Ask the person which team.
- Not imported, `github_access` false: ask the person to connect GitHub to
  the team in the iii cloud console. This is the GitHub authorization. Resolve
  again after. (`iii-cloud::api` only reads: you cannot start it.)
- Not imported, `importable` set: the person imports it on the page, choosing
  the environment name, whether it is production, and one of the listed
  `manifests`. When several were detected, recommend one with the cloud's
  `suggested_environment`. Each environment has one manifest; see "Several
  compose files" in `iii-cloud`.

## 3. Use a worktree

Call `iii-cloud::project::use {path, environment_id, mode: "worktree"}`.
Recommend the worktree. When the checkout has uncommitted changes, ask the
person if they should come along, then pass `bring_changes: true`. Unpushed
commits do not come along. Tell the person.

Then move the session:

```
ade::working-directory::propose {path: "<project.workspace.path>"}
```

Work in that directory from now on. A worktree from an earlier run is reused: it is reset when its commits were merged, and otherwise picks a new branch name.

## 4. Check, and fix

Call `iii-cloud::check`. Read `plan`, `here` and every problem.

- Fix each problem whose `owner` is `agent`. The fix is in `iii-cloud/plan-codes`.
- Bring each `person` problem and each warning to the person. Say what it is
  in one sentence, give your recommendation, and let them choose.
- Work in groups, quick fixes first and app-code changes last, and call `check {}` after each commit. See the "Fix loop" of `iii-cloud`.
- Check again after each round. The cloud plan stops at its first blocking
  step, so more problems can show up.

## 5. Write the `cloud:` block

Put `web`, `build`, `release` and `healthcheck` in the production compose
file, so they are versioned with the code. The block needs iii 0.27.0 on this machine:
iii 0.26 refuses it. `LOCAL_ENGINE_VERSION` says when it is the wrong one.
Only iii cloud's plan checks the block's content.

## 6. The web part

Read `plan.web_candidates`. Explain each option in one sentence, recommend
one, and let the person choose. Write `cloud.web` and `cloud.healthcheck`, then
check again. Every `WEB_*` code is the person's answer, not yours. Also ask
if the environment is public or team only, and which path is the health path.

## 7. Secrets

A database URL is not a secret. Follow the "Databases" rule of `iii-cloud`: add the `database` worker and bind `DATABASE_URL: ${DATABASE_PRIMARY_URL}`. Never ask for it (`DATABASE_FROM_CLOUD`).

For each name the plan or the app needs, call
`iii-cloud::secret {name, description}`. The person types the value on the
page. It goes to iii cloud and to the local `.env`. You only see `set`,
`left_empty` or `failed`. Use `mode: "not_needed"` or `mode: "decide"` to
mark a name that needs no value or the person's choice.

`NOT_SAVED` means the secret went nowhere (an API key cannot write to the cloud, and the `.env` is tracked): tell the person to set it in the console. `local: "skipped"` means nothing was written to the `.env`: either git tracks it, or the cloud took the value and the compose file does not read `${NAME}`. Never ask for a value in chat, even when the person offers.

## 8. The local run

Call `iii-cloud::check {run: true}`. It installs, builds, runs the release
command, starts the compose file with the cloud's engine, waits for every
worker and the routes, and calls the healthcheck. It waits for the run itself (up to 240 s). When the call returns with
`run.status: "running"`, wake on that run, never `sleep`, with an expiry near `run_timeout_s`:

```
harness::observe {function_id: "iii-cloud::on-change",
  filters: {"input.kind": {"$eq": "run"}, "input.id": {"$eq": "<run id>"},
            "input.terminal": {"$eq": true}},
  lifecycle: {expires_in_ms: 900000}}
```

On a failed stage, read `run.error` and `run.log_tail`, and the code in `iii-cloud/plan-codes` ("Local run failures"). Fix in the worktree and run again. The run copy moves every engine, http and ade port to a free one, and never uses 3111, 3112, 3113 or 49134. One run per workspace: a new run of another commit replaces the going one, and the same commit with the same compose file attaches to it.

## 9. Commit, push and open the pull request

```
git add -A && git commit -m "<message>"
git push --force-with-lease -u origin HEAD
gh pr create --base <environment branch> --head <fix branch> \
  --title "<title>" --body "<check.report>

Changes: <one line of what you changed>"
```

Run it from the worktree. The fix branch is `worktree.branch` of the brief
(`project.workspace.branch`).

An environment on its own branch (`pushes_directly` in the brief) has no pull
request: push only your fix branch, check and run, and stop there. The
person's deploy on the page pushes the checked commit to the environment's
branch. Never push that branch yourself.

A check is ready only when its base commit is in the branch's history, the worktree is clean, and the run's commit equals the check's commit. A run of uncommitted files has a commit ending in `+dirty`, and it does not count. Commit first, then check and run. Use the `report` of the ready check as the body. `gh` uses the person's
own GitHub login. Run `iii-cloud::status {refresh: true}` to see the pull request.

## 10. Hand over the deploy

Your job ends when `check {run: true}` passed on the commit. Tell the person
to review and deploy on the page with the "Merge and deploy…" button (its
dialog merges and deploys). Only the person deploys, merges, confirms a
waiting deployment, rolls back, or writes variables and settings:
`iii-cloud::deploy` and `iii-cloud::api::write` are denied to you, and you
never do it another way (`gh pr merge`, raw HTTP), even when asked. The merge
is pinned to what you checked: the merged tree must equal the checked commit's
tree, and its first parent must be the base. If the person merges on GitHub
instead, tell them to confirm the waiting deployment on the page ("Confirm
deploy"). When the page reports `MAIN_MOVED`, `PR_HEAD_CHANGED` or
`PLAN_CHANGED`, the plan is old: check again. `PUSH_STILL_RUNNING` means the
plan changed but iii cloud could not cancel the merged commit's deployment:
watch it.

## 11. Watch the deployment

```
harness::observe {function_id: "iii-cloud::on-change",
  filters: {"input.kind": {"$eq": "deploy"}, "input.id": {"$eq": "<deploy id>"},
            "input.terminal": {"$eq": true}}}
```

Use the id of the person's deployment (`status.deploys`). When iii-cloud's own confirmed
deployment replaces it, that is not the end. When a deployment iii-cloud did
not create replaces it (a push by someone else), the deployment ends with
`outcome: "replaced"` and `replaced_by: <id>`. iii-cloud follows the
successor: tell the person, and observe the new id.

A `needs_action` event carries the id of the push's deployment, not yours, so
that filter never sees it. Observe it for the environment instead, without the id:

```
harness::observe {function_id: "iii-cloud::on-change",
  filters: {"input.kind": {"$eq": "deploy"}, "input.record.status": {"$eq": "needs_action"}}}
```

When it fires, tell the person to confirm it on the page ("Confirm deploy").

On `ready`, tell the person the URL and what the local run did not cover. On a
failure, including `no_route`, read the stage, the code and the log lines in
the event. Fix in the worktree, run `check {run: true}`, and open a new pull
request. The previous release keeps serving, and rollback is one click in the
cloud's console.
