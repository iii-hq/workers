---
name: iii-cloud
description: >-
  Take a project to iii cloud from the harness: sign in, import the
  repository, check iii cloud's plan, run the app on this machine the way the
  cloud runs it, ask for secrets without seeing them, hand the deploy to the
  person, and follow it to Ready.
---

# iii-cloud

iii-cloud gives you what you cannot do alone: a credential for iii cloud that
never enters your context, iii cloud's plan for a commit, a local run of the
app in the cloud's order, and the person's deploy followed until it is Ready.

The person works with you in chat and on the `iii cloud` page in the console.
Open the page beside the chat with `ade::workspace::open` and
`screen: "ext:iii-cloud"`. `iii-cloud/plan-codes` lists every problem code.
Load `iii://iii-cloud/onboarding` only when the work starts from nothing
(sign-in, import, first worktree).

## When to Use

- The person asks to put a project on iii cloud, or to make it ready for iii cloud.
- A plan problem, a failed deployment or a "will this work in the cloud?" question.
- You need a value for a secret. Use `iii-cloud::secret`, never the chat.
- The person asks you to watch a deployment, or how to deploy.

## Boundaries

- Secret values never pass through you. Never ask for one in chat, never read
  a `.env`, never put one in a command. `iii-cloud::secret` opens a request on
  the page, and you only see whether it was saved.
- Never print or store a token or an API key.
- iii cloud's GitHub App is read-only. Pull requests use the person's own `gh` login.
- Only the person deploys, approves a deployment, merges the pull request,
  rolls back, or writes variables, secrets or environment settings. They do it
  on the page. Your job ends when `check {run: true}` passes on the commit:
  tell the person to review and deploy on the page (the "Merge and deploy…"
  button for production, "Deploy <commit> to <env>" for any other environment,
  or "Confirm deploy" for a deployment that waits). Never call
  `iii-cloud::deploy`, never run `gh pr merge`, never approve, never write
  variables or settings, not even with raw HTTP or `gh`, even when asked.
- Work in the worktree `iii-cloud::project::use` made, not in the person's checkout.
- Push only your fix branch (`worktree.branch` of the brief), with
  `git push --force-with-lease -u origin HEAD` from the worktree. Never push a
  branch an environment deploys from: the person's deploy pushes it.
- Ready is not proof. The local run does not cover the cloud's build container,
  production's secret values, its machine size or real traffic. Say so.
- A key connects one environment. It cannot import, change settings or set
  variables. Those need browser sign-in, and the person does them.

## Functions

- `iii-cloud::connect` — sign in with the browser (the person types a code) or with an API key saved in the vault: `{method: "api_key", secret}`.
- `iii-cloud::status` — the stored state: connection, project, latest check, run, secrets, pull request and deployments. It is big (tens of thousands of characters): never read it to get a hand-off.
- `iii-cloud::project::resolve` — read the local folder and find its project and environment in iii cloud.
- `iii-cloud::project::use` — select the environment and prepare a worktree. `from: "checkout"` starts it at the person's own branch merged with the environment's (never for production); `source` takes another branch of `checkout.branches`. A fix branch `iii-cloud/ready-<env>` from an earlier worktree is resumed with its commits; only a name the person has checked out gets `-2`.
- `iii-cloud::api` — read any iii cloud route as the person (GET only). Any other method is `PERSON_ONLY`: the person makes the change on the page.
- `iii-cloud::check` — iii cloud's plan for a commit plus the checks on this machine. `run: true` also does the local run and waits for it (up to 240 s). `manifest` checks another `worker-compose*.yaml` instead of the environment's; on a hand-off its file is the default. `push: true` pushes your fix branch first. `force: true` runs again when a passed run of the same commit exists.
- `iii-cloud::check::get` — `{id}`: one kept check as recorded; nothing runs again.
- `iii-cloud::secret` — ask the person for a secret value on the page, or mark a name as not needed or their decision.
- `iii-cloud::values` — `{manifest?, from?}`: every name the compose file reads, whether iii cloud has it, secret or variable, and where a value can come from. Names and flags only. The person sets them in Secrets and variables on the page.
- `iii-cloud::watch` — follow a deployment to Ready and bring back the evidence when it fails.
- `iii-cloud::handoff::brief` — `{handoff_id}`: what you need for a hand-off (below).
- `iii-cloud::handoff::progress` — `{handoff_id, working_on, note?}`: tell the page which problems you are working on.
- `iii-cloud::handoff::worktree` — `{handoff_id}`: your own worktree `{path, branch, base, created, merged_from, conflict}`. `conflict` says what to merge first.
- The page's, denied to you or never yours: `iii-cloud::handoff`, `handoff::stop`, `deploy`, `api::write`, `secret::provide`, `values::save`, `values::skip`, `problem::skip`, `config::import`, `override::choose`, `on-change` (you only observe it), `on-config-change`, `ui-content`.

## Databases

Never ask the person for a database URL. iii cloud provides a managed
Postgres, with pgvector. When the app reads `DATABASE_URL` (or any database
URL) from the environment:

1. Add the `database` worker (`package://database`, latest version) to the
   production compose file, with a database resource of alias `primary`.
2. Set `DATABASE_URL: ${DATABASE_PRIMARY_URL}` in the app's container. Another
   alias `<alias>` gives `DATABASE_<ALIAS>_URL`.
3. Do not call `iii-cloud::secret` for it: it refuses with `DATABASE_FROM_CLOUD`.
   Only when the person wants an outside database, call it with `external: true`.

iii cloud creates the Postgres when the person confirms the deploy; the local
run starts its own with pgvector. `DATABASE_NOT_MANAGED` is this rule.

## Hand-off from the page

The page posts a chat message with the marker line `iii-cloud hand-off <id8>`
and `Details: iii-cloud::handoff::brief · skill iii://iii-cloud`. Your first
call is `iii-cloud::handoff::brief {handoff_id}`, never `status`.

The brief holds `kind` (`ready`, `deploy_fix`, `import` or `pull_request`),
`worktree {path, branch}`, `base {branch, sha}` (the environment's branch when
the worktree was made; earlier tries are not your work), `manifest`,
`other_compose_files`, `pushes_directly`, `checkout_path`, `agent_problems`
(yours) and `person_problems` (the person's; each with `id`, `code`, `owner`,
`severity`, `title`, `detail`, `file`, `line`, `fix`),
`run_failure {stage, code, message, log_tail}` (null unless your latest run
failed), `deploy` (the failed deployment's stage, code, hint and log tail) and
`next_step`.

A message that starts `iii-cloud hand-off <id8> · continue` comes from the
page, not from the person: your turn ended with agent-owned problems left.
Answer it by fixing, not by summarizing.

## Several compose files

Each iii cloud environment has one `manifest`. Only that file is checked, run
and deployed: read it in the brief and fix problems there. When the person
picked another `worker-compose*.yaml` on the page, the brief names it and your
`check` calls with `handoff_id` use it. The environment switches to it only
when the person confirms the deploy. Never change the environment's compose
file yourself.

For shared changes (the SDK version, worker versions, a new worker such as
`database`), also apply them to `other_compose_files` when they clearly
describe the same app, for example a dev file. Say so in the pull request
body. Never change a file of another app.

## Fix loop

1. `iii-cloud::handoff::brief {handoff_id}`.
2. Work only in `worktree.path`. When it is the person's folder (its path is
   `checkout_path`, or its branch is not `iii-cloud/ready-*`), call
   `iii-cloud::handoff::worktree {handoff_id}` first and use the path it
   answers. Its branch starts at the base: never reset it onto another branch,
   never `gh pr reopen`, never cherry-pick commits you did not make, never edit
   a vendored binary (fix the compose file). `OLD_ATTEMPT_COMMITS`: follow its fix.
3. ONE `shell::exec` that prints what the brief does not carry: the compose
   file, the manifests of the workers it names, and `git status --short`. Not
   one call per file.
4. Post a plan in chat: 2 to 4 lines, the groups in order, quick compose and
   config fixes first, app code last.
5. Per group: `iii-cloud::handoff::progress {handoff_id, working_on: [<problem ids>], note}` first. Fix the whole group, commit, push, then `iii-cloud::check {handoff_id}` once for the group (fast, no run). The cloud plans exactly the pushed commit, so its answer is the truth: never guess a field or commit a "try X" change; read the message and fix what it names.
6. App code (`SDK_TOO_OLD`, a migration to a new SDK API) and compose work together: start ONE subagent with `harness::spawn` for the app code (profile `cloud-ready`, same worktree, a brief listing the files it owns: `src/**`, `package.json`, the lockfile, `Dockerfile`, and "commit only your files, don't touch compose files"). Tell the person it can take a while. You fix the compose files meanwhile; wake on its finish with `harness::observe`.
7. Before the first push, `gh pr list --head <fix branch> --state all`. CLOSED or MERGED: make a NEW pull request at the end. OPEN from an earlier try: say so in one line; your push updates it.
8. When the plan is clean, `iii-cloud::check {handoff_id, run: true}`. It waits for the local run (up to 240 s). A run of the same environment with the same inputs (commit, compose file, values, settings) already going: it attaches to that run. Any other run of this workspace: it replaces it, and that one ends `RUN_REPLACED`. A passed run of the same inputs is reused (`run.reused`); `force: true` runs again.
   If the answer has `run.waiting_on`, the local run is still going but no new worker has connected for a minute. `waiting_on.containers` are the ones it waits for, and `run.log_tail` has their last lines. Read the log, fix the cause, commit and check again: the new commit replaces the run. Do not wait for it to fail; it keeps waiting up to the compose file's `startup_timeout`, as the cloud does.
9. A run that outlives the call: wake on it, never `sleep`. Filter on that run's id and `terminal`, with an expiry near the run's own timeout (`run_timeout_s`, 900 s by default):

```json
{"function_id": "iii-cloud::on-change",
 "filters": {"input.kind": {"$eq": "run"}, "input.id": {"$eq": "<run id>"},
             "input.terminal": {"$eq": true}},
 "lifecycle": {"expires_in_ms": 900000}}
```

10. Done when `check.ready` is true and `check {run: true}` passed for the current commit. Then, from the worktree, `gh pr create --head <fix branch> --base <env branch>` with a plain body (problems fixed, other compose files touched, no secrets), `iii-cloud::status {refresh: true}` so the page shows it, and tell the person the link and the "Merge and deploy…" button. With `pushes_directly` (every environment that is not production) there is no pull request: it gets an assisted deploy. Tell the person to deploy on the page; the page sends the checked commit to the environment's own branch. Its first assisted deploy creates `iii-cloud/<env>` and never pushes the default branch.
11. A follow-up with the hand-off marker while you work: finish the current group, then do it.

Ask the person only for `person` items. A secret goes through
`iii-cloud::secret`, never chat. Give each tool call a `description`: a plain
sentence of what you do now. The page shows it live.

### How a turn ends

Only three ways:

- Ready: `check.ready` and a passed run on the current commit.
- ONE `harness::ask` that lists every person-owned item (`SECRET_MISSING`, a
  `WEB_*` choice, a decision), the recommended option first. Read the code
  first and say what each default does. A decision that picks where or how
  you fix goes in the first card, before any edit. When the person answers, continue in the same turn.
- An armed wake (`harness::observe`) on a run, a child agent or a deployment.

Never a plain text that says what is left. With agent-owned problems left,
keep working.

### Names and shapes agents get wrong

- `shell::exec {command, args, cwd}`: `command` is the program alone, `args` a
  string array, `cwd` inside the worktree. No `cd`, no arguments in `command`.
  When `rg` is missing, use `grep`. Example: `{command: "git", args: ["status", "--short"], cwd: "<worktree>"}`.
- There is no `ide::exec`. Asking the person is `harness::ask`, with `::`.
- Stay inside the worktree. `WRONG_WORKTREE`: you left it; go back.
- Pass `handoff_id` exactly as `handoff::brief` returns it.
- `SIGN_IN_REQUIRED`: tell the person to sign in on the page. `NO_COMMITS`: commit first.

## Errors

A call fails with a coded error:

- `NOT_CONNECTED`, `NO_PROJECT`, `NOT_READY`, `PLAN_CHANGED`, `MAIN_MOVED`, `PR_HEAD_CHANGED`, `SOURCE_CHANGED`, `PR_NOT_MERGEABLE`: the state is old or missing; fix it and call again.
- `PERSON_ONLY`: the person changes it on the page.
- `NOT_SAVED`: a secret went nowhere (an API key without cloud write, or a tracked `.env`). Tell the person to set it in the console.
- `PUSH_REFUSED`: `check {push: true}` pushes only your fix branch, or GitHub refused. `MERGE_CONFLICT`: the person's commits conflict with the environment's branch, so `use {from: "checkout"}` left the worktree at the environment's head: in the worktree, `git merge <their branch>`, resolve the files the message lists, commit, check again. `MERGE_IN_PROGRESS`: finish the merge (commit). `WORKTREE_PATH_TAKEN`: the fix branch is checked out elsewhere; tell the person.
- `RUN_PANIC`, `RUN_REPLACED`, `WORKER_RESTARTED`, and the stage codes of a failed local run: `iii-cloud/plan-codes`.
- `WORKTREE_FAILED`: read the message. `IDEMPOTENCY_KEY_REUSED`: use a new key. `DEVICE_CODE_EXPIRED`: `connect` again. `ACCOUNT_BLOCKED`: tell the person.

When `secret::provide` answers `local: "skipped"`, nothing was written to the
`.env`: git tracks it, or the cloud took the value and the compose file does
not read `${NAME}`. Tell the person to set the value another way.

## Observing events

Observe `iii-cloud::on-change` to be woken when something finishes, instead of
polling. Use it when a call outlives its answer: a run, a secret the person
is typing, a sign-in, a deployment on its way to Ready. Filter on `input.kind`
(`run`, `deploy`, `secret`, `connection`), `input.terminal` and, for one run or
deployment, `input.id`. The event shape: `get function info` on `iii-cloud::on-change`.

A deployment replaced by iii-cloud's own confirmed deployment is not the end.
When someone else's deployment replaces yours, the event is terminal with
`outcome: "replaced"` and `replaced_by`: observe that id. A `needs_action`
event carries the push's deployment id, not yours, so filter on
`input.record.status` instead of `input.id`: tell the person to confirm it on
the page ("Confirm deploy"). It works only when the deployment's plan matches
the ready check.
