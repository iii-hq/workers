---
name: iii-cloud
description: >-
  Take a project to iii cloud from the harness: sign in, import the
  repository, check the cloud plan, run the app here the way the cloud runs
  it, ask for secrets without seeing them, hand the deploy to the person, and
  follow it to Ready.
---

# iii-cloud

The `iii-cloud` worker gives you what you cannot do alone: a credential for
iii cloud that never enters your context, the cloud's plan for a commit, a run
of the app on this machine in the cloud's order, and the person's deploy
followed until it is Ready. You know the path.
The worker holds the keys.

The person works with you in chat and on the `iii cloud` page in the console.
Open the page beside the chat with `ade::workspace::open` and
`screen: "ext:iii-cloud"`. Read `iii-cloud/onboarding` for the steps and
`iii-cloud/plan-codes` for every problem code.

## When to Use

- The person asks to put a project on iii cloud, or to make it ready for iii cloud.
- A plan problem, a failed deploy or a "will this work in the cloud?" question.
- You need a value for a secret. Use `iii-cloud::secret`, never the chat.
- The person asks you to watch a deploy, or how to deploy.

## Boundaries

- Secret values never pass through you. Never ask for one in chat, never read
  a `.env`, never put one in a command. `iii-cloud::secret` opens a request on
  the page, and you only see whether it was saved. `iii-cloud::secret::provide`
  is the page's, not yours.
- Never print or store a token or an API key. They stay in the vault.
- The cloud's GitHub App is read-only. Pull requests use the person's own
  `gh` login.
- Only the person deploys, approves a deployment, merges the pull request,
  rolls back, or writes variables, secrets or environment settings. They do it
  on the page. Your job ends when `check {run: true}` passes on the commit:
  then tell the person to review and deploy on the page (the "Merge and
  deploy…" button, or "Confirm deploy" for a deployment that waits). Never
  call `iii-cloud::deploy`, never run `gh pr merge`, never approve, and never
  write variables or settings, not even with raw HTTP or the `gh` CLI, even
  when the person asks you to: point them to the page.
- Work in the worktree `iii-cloud::project::use` made, not in the person's checkout.
- Push only your fix branch (`handoff_brief.worktree.branch`), with
  `git push --force-with-lease -u origin HEAD` from the worktree. Never push a
  branch an environment deploys from: the person's deploy pushes it.
- Ready is not proof. The run here does not cover the cloud's build container,
  production's secret values, its machine size or real traffic. Say so.
- The worker is for the harness, never for the project that goes to the cloud.
- A key connects one environment. It cannot import, change settings or set
  variables. Those need browser sign-in, and the person does them.

## Functions

- `iii-cloud::connect` — sign in with the browser (the person types a code) or with an API key saved in the vault.
- `iii-cloud::status` — the stored state: connection, project, latest check, run, secrets, pull request and deploy.
- `iii-cloud::project::resolve` — read the local folder and find its project and environment in iii cloud.
- `iii-cloud::project::use` — select the environment and prepare a worktree for you to work in. `from: "checkout"` starts it at the person's own branch merged with the environment's, instead of the environment's alone (never for production); `source` takes another branch of `checkout.branches` (`main`, `origin/feature-x`) instead of the person's HEAD. A fix branch `iii-cloud/ready-<env>` left from an earlier worktree (here or on origin) is resumed with its commits, and the environment's branch merged in; only a name the person has checked out gets `-2`. While a project with a workspace is selected, `project::resolve` of another folder leaves the selection alone and fills `status.candidate`; `project::use` selects it.
- `iii-cloud::api` — read any iii cloud route as the person (GET only); temporary-credential routes are refused. Any other method is `PERSON_ONLY`: the person makes the change on the page.
- `iii-cloud::check` — the cloud plan for a commit plus the fast checks here; with `run: true`, also the run here. `manifest` checks another `worker-compose*.yaml` of the repository instead of the environment's (say a new `api/worker-compose-cloud.yaml` beside the file the environment uses now), changing nothing in iii cloud: the environment switches to that file only when the person confirms the deploy. On a hand-off, its file is the default. `push: true` pushes your fix branch first (never a branch an environment deploys).
- `iii-cloud::check::get` — `{id}`: one kept check as it was recorded (`status.checks` lists them in short); nothing runs again.
- `iii-cloud::secret` — ask the person for a secret value on the page, or mark a name as not needed or their decision.
- `iii-cloud::values` — `{manifest?, from?}`: every name the compose file reads, whether iii cloud has it, secret or variable, and where a value can come from (a `.env*` file of the person's folder, the environment's address, a copy from another environment). Names and flags only, never a secret's value. The person sets them in Values (Setup) or 'Set values' in the check; `iii-cloud::values::save` is the page's, and is denied to agents.
- `iii-cloud::watch` — follow a deploy to Ready and bring back the evidence when it fails.
- `iii-cloud::handoff::progress` — tell the page which problems you are working on, `{working_on, note?}`, so it shows a spinner on them.
- `iii-cloud::handoff::worktree` — `{handoff_id?}`: your hand-off's own worktree, `{path, branch, base, created, merged_from, conflict}`. A hand-off that points at the person's folder gets one (the environment's branch, the person's branch merged in, their changed files without secrets) and works there from then on. `conflict` says what to merge first.
- `iii-cloud::handoff` — the page's hand-off to chat; the agent does not call it. `iii-cloud::handoff::stop` is the page's too, and is denied to agents.
- `iii-cloud::deploy` (merge, push to an environment's own branch, deploy, confirm a waiting deployment) and `iii-cloud::api::write` (every change to iii cloud) — the person's, from the page; denied to agents.
- `iii-cloud::secret::provide`, `iii-cloud::values::save`, `iii-cloud::values::skip`, `iii-cloud::problem::skip`, `iii-cloud::config::import`, `iii-cloud::override::choose`, `iii-cloud::on-change`, `iii-cloud::on-config-change`, `iii-cloud::ui-content` — the page's and the worker's own; never call them.

A key connects with the secret's name alone: `{method: "api_key", secret}`.
The worker finds the team and the environment through the cloud. Pass
`team_id` and `environment_id` only for an older cloud that fails without them.

## Databases

Never ask the person for a database URL. iii cloud provides a managed
Postgres, with pgvector. When the app reads `DATABASE_URL` (or any database
URL) from the environment:

1. Add the `database` worker (`package://database`, latest version) to the
   production compose file, with a database resource of alias `primary`.
2. Set `DATABASE_URL: ${DATABASE_PRIMARY_URL}` in the app's container. Another
   alias `<alias>` gives `DATABASE_<ALIAS>_URL`.
3. Do not call `iii-cloud::secret` for it. The cloud refuses it with
   `DATABASE_FROM_CLOUD`. Only when the person says they want an outside
   database, call it with `external: true`.

The cloud creates the Postgres when the person confirms the deploy. The run
here starts a local Postgres with pgvector. `DATABASE_NOT_MANAGED` is this
rule: fix it as above.

## Hand-off from the page

The page posts a one-line message in chat with a marker line
`iii-cloud hand-off <id8>` and `Details: iii-cloud::status · skill
iii://iii-cloud`. When you see the marker, first call `iii-cloud::status`
and read `handoff_brief`. It holds the worktree, the problems with their
fixes, the deploy failure data and the next step. Its `kind` is `ready`,
`deploy_fix`, `import` or `pull_request`. Then follow the Fix loop.

## Several compose files

Each iii cloud environment has one `manifest`. Only one file is checked, run
here and deployed: read it in `handoff_brief.manifest`. Fix problems in that
file.

The person can pick another `worker-compose*.yaml` on the page. Then the
hand-off carries it: `handoff_brief.manifest` names it, and your
`iii-cloud::check` calls with `handoff_id` use it without a `manifest` field.
The environment switches to that file only when the person confirms the
deploy of a ready check made with it; until then, pushes keep the current
file. Never change the environment's compose file yourself.

For shared changes (the SDK version, worker versions, a new worker such as
`database`), also apply them to the files in
`handoff_brief.other_compose_files` when they clearly describe the same app,
for example a dev file. Say so in the pull request body. Never change a file
of another app.

## Fix loop

When the person hands you problems from the page:

Shell: use `shell::exec` with `command` (the program), `args` (a string array)
and `cwd` (the worktree). Example: `{command: "git", args: ["status"], cwd: "<worktree>"}`.
Never put arguments in `command` or use `cd`.

Fresh start: the base is `iii-cloud::status` → `handoff_brief.base`, the env
branch at the moment the worktree was made. Earlier tries are not your work.
Never `gh pr reopen`, and never reset or cherry-pick onto commits you did not
make in this worktree. If `check` reports `OLD_ATTEMPT_COMMITS`, follow its fix.

1. Load `iii://iii-cloud` and `iii://iii-cloud/plan-codes`. Each code has one fix line.
2. Post a plan in chat first: 2 to 4 lines, the groups in order.
3. Work only in your hand-off's worktree, `handoff_brief.worktree` (path and branch), never the person's checkout. If it points at the person's folder (its path is `handoff_brief.checkout_path`, or its branch is not `iii-cloud/ready-*`), call `iii-cloud::handoff::worktree {handoff_id}` first and work in the path it answers; it never changes what the page has selected. Its branch starts at the base: never reset it onto another branch (an older `iii-cloud/*` branch on the remote is replaced, not adopted), and never edit a vendored binary in the repo (fix the manifest instead).
4. Split the work. When app-code work (`SDK_TOO_OLD`, a migration to a new SDK API) and compose or config work both exist, start ONE subagent with `harness::spawn` for the app code. Give it the profile `cloud-ready`, the same worktree as its folder, and a brief that lists the files it owns (`src/**`, `package.json`, the lockfile, the `Dockerfile`) and says: "commit only your files, don't touch compose files". Announce it: "this changes app code and can take a while".
5. Meanwhile you do the quick group in the compose files only: compose, lock, healthcheck, release, database and web. Never two agents on the same file. Wake on the child's finish with `harness::observe` (don't poll it with waits; keep working on your group), then integrate: commit, `iii-cloud::check {}`, then `check {run: true}` when the plan is clean. It is mandatory before you say "ready".
6. The first action of every group is `iii-cloud::handoff::progress {working_on: [<problem ids or codes>], note: "<one plain sentence>"}`, so the page shows a spinner on those problems. After each group, commit in the worktree, push it from the worktree with `git push --force-with-lease -u origin HEAD` (the fix branch is yours, `handoff_brief.worktree.branch`, which can be `iii-cloud/ready-<env>-2`; a leftover from an earlier try is replaced), then call `iii-cloud::check {}` (fast, no run). The cloud plans exactly that pushed commit, so its answer is the truth: never guess a manifest field or commit a "try X" change; read the plan's message and fix what it names.
7. Give each tool call a `description` that is a plain sentence of what you are doing now. The page shows it live.
8. Ask the person only for `person` items. A secret goes through `iii-cloud::secret`, never chat.
9. When a problem is a decision, or you are stuck for more than 5 minutes on one, stop and ask with `harness::ask`. Ask all the person's decisions in ONE ask. Put the recommended option first. Read the code first, and say what each default does (an empty `SESSION_SECRET` only leaves sign-in off).
9a. Before the first push, run `gh pr list --head <fix branch> --state all`. A CLOSED or MERGED pull request there: create a NEW pull request after your work, never reopen. An OPEN one from an earlier try: tell the person in one line and keep going; your push updates it. The first push replaces any remote fix branch (`--force-with-lease`).
9b. When the person answers your `harness::ask`, continue at once in the same turn. Do not end the turn.
10. Repeat until `status.check.ready` is true. Before you say done, make sure `check {run: true}` passed for the current commit; if not, run it. Only then talk about the pull request or a deploy. Your job ends there: the person reviews and deploys on the page with the "Merge and deploy…" button; name it exactly. You never deploy or merge. Then, from the worktree (the branch is already pushed), run `gh pr create --head <your fix branch> --base <env branch>` with a plain body: the problems fixed and any other compose files touched, and no secrets. Call `iii-cloud::status {refresh: true}` so the page shows the pull request, and tell the person the link. With `handoff_brief.pushes_directly` there is no pull request: skip `gh pr create`, and tell the person to deploy on the page.
11. A message that begins with the hand-off marker while you are already working is a follow-up. Finish the current group, then do it.

Newer codes and fields:

- `SIGN_IN_REQUIRED`: a person item. Tell the person to sign in on the page.
- `NO_COMMITS`: commit first.
- `WRONG_WORKTREE`: you are not in your hand-off's worktree. `cd` back to it.
- `handoff_brief.pushes_directly`: when true, the environment deploys from its own branch and there is no pull request. Push only your fix branch, then check, then run, and tell the person to review and deploy on the page: their deploy pushes the checked commit to the environment's branch. Never push the environment's branch yourself.
- Every `iii-cloud::*` call you make passes `handoff_id` (from the marker, or `status.agent.handoff_id`).

## Errors

The worker fails with a coded error. The ones you will meet most:

- `NOT_CONNECTED`, `NO_PROJECT`, `NOT_READY`, `PLAN_CHANGED`, `MAIN_MOVED`, `PR_HEAD_CHANGED`, `PR_NOT_MERGEABLE`, `RUN_BUSY`: the state is old or missing; fix it and call again.
- `PERSON_ONLY`: `iii-cloud::api` only reads. Tell the person what to change and that they do it on the page.
- `NOT_SAVED`: a secret went nowhere (API key without cloud write, and a tracked `.env`). Tell the person to set it in the console.
- `RUN_PANIC`: the run crashed inside the worker. Run again.
- `WORKTREE_FAILED`: `git worktree add` failed; read the message.
- `PUSH_REFUSED`: `check {push: true}` pushes only your fix branch, or GitHub refused the person's push. `MERGE_CONFLICT`: the person's commits conflict with the environment's branch, so `use {from: "checkout"}` left your worktree at the environment's head. In the worktree (never the person's checkout), `git merge <their branch>`, resolve the conflicts in the files the message lists, commit, then check again. `MERGE_IN_PROGRESS`: a merge in the worktree is not committed yet; finish it (commit), then check again. `WORKTREE_PATH_TAKEN`: the fix branch is checked out in another folder; tell the person.
- `IDEMPOTENCY_KEY_REUSED`: that key was used for another request. Retry with a new key.
- `WORKER_RESTARTED`: the worker restarted during a run; run again.
- `DEVICE_CODE_EXPIRED`: the code timed out; call `connect` again.
- `ACCOUNT_BLOCKED`: the account is blocked; tell the person.

When `secret::provide` answers `local: "skipped"`, the worker did not write the
`.env`: git tracks it, or the cloud took the value and the compose file does not
read `${NAME}`. Tell the person to set the value another way.

## Observing events

Observe `iii-cloud::on-change` to be woken when something finishes, instead of
polling `iii-cloud::status`. Reach for it when a call outlives its answer: a
run that passed 240 seconds, a secret the person is typing, a sign-in the
person is approving, a deploy on its way to Ready.

Observe with `harness::observe`, filtering on the event's `kind` and `terminal`. For a deploy, also filter on the id of the person's deploy (`input.id`):

```json
{
  "function_id": "iii-cloud::on-change",
  "filters": {
    "input.kind": { "$eq": "deploy" },
    "input.id": { "$eq": "<deploy id>" },
    "input.terminal": { "$eq": true }
  }
}
```

A deploy replaced by the worker's own confirmed deployment is not the end: keep watching. When someone else's deployment replaces yours, the event is terminal with `outcome: "replaced"` and `replaced_by`; the worker follows the successor, so observe that id.
A deploy event with status `needs_action` also wakes you: the cloud holds the
deployment until someone confirms it. Tell the person to confirm it on the
page ("Confirm deploy"); you never confirm it. It works only when the
deployment's plan matches the ready check.

Do the same for `run`, `secret` and `connection`. Skip it when the call
already returned what you need. Call `get function info` on
`iii-cloud::on-change` for the event shape.
