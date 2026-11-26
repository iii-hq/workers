# iii-cloud worker

Status: proposed, 2026-11-25. Tech spec for the approved design v4.

- Design: [`design/design-page.html`](design/design-page.html) (Portuguese, v4,
  authoritative for behavior) and the 12 console boards in
  [`design/`](design/), in the journey order of `design/canvas.json` `order`.
  The page must match the boards; §14 maps each board to its components.
- Base: workers `origin/main` `e478747d5`, branch `feat/iii-cloud-worker`.
- Cloud facts: iii-cloud `origin/main` `d31ed9457` (the design cited
  `7e0b0553`; the facts below were re-read at `d31ed9457`). Line numbers are
  that commit's.
- Engine: iii `0.27.0` (tag `iii/v0.27.0`, `b6d44e2b6`). It accepts the
  compose `cloud:` block (`crates/iii-compose/src/config.rs:13,1300`); 0.26
  refuses it.
- Split into parallel work: [`tasks.md`](tasks.md).

## Why

A person who wants their project on iii cloud, and wants it to work the first
time, gets there with their own harness agent. The worker gives the agent
what it cannot do alone:

- a credential for iii cloud that never enters the agent's context;
- the cloud's plan for a commit;
- a run of the app on this machine the way the cloud runs it;
- a deploy pinned to the plan the person saw;
- a deploy followed to Ready, with the evidence when it fails.

The agent knows the path, through the `cloud-ready` profile and the skill.
The worker stays thin.

## Goals and non-goals

Goals:

- `iii call compose::add worker=iii-cloud` in the person's **harness** (never
  in the app being deployed) adds the functions, the console page, the skill
  and the `cloud-ready` profile.
- The person signs in with a typed browser code, or saves an
  environment-scoped API key in the harness vault.
- The agent resolves the cloud project and environment from a local folder's
  git remote. It works in a git worktree from `origin/<env branch>`, so the
  person's checkout stays as it is.
- `iii-cloud::check` returns the cloud plan for a commit plus the fast checks
  here, as one list of problems `{code, file, line, excerpt, owner}`.
- Secret values reach iii cloud and the worktree's `.env` from the page. They
  never enter the agent's context, the chat or the traces.
- The run here uses the production compose file, the cloud's engine version,
  the frozen install, the build command, the release command against a local
  Postgres, all workers registered, http routes served and the healthcheck.
- "Merge and deploy" merges the pull request and creates the deployment with
  the confirmations, pinned to the plan the person saw.
- The worker follows a deploy to Ready. An http group that serves no route is
  a failure. On failure the agent gets the stage, the code and the last log
  lines.

Non-goals (v1):

- An ephemeral test environment, a cloud build-check, a local image build or
  an image upload, or a required GitHub check (design: "Para depois").
- Restricting the agent. It has every tool (owner decision, 25/11). Only
  secret values stay out of its context.
- Opening pull requests. The agent does that with the person's own `git` and
  `gh`; the cloud's GitHub App stays read-only.
- Running managed queue, state or storage resources here. Only Postgres runs
  here (§6).
- Rollback. It stays one click away in the cloud console.
- The kind stack in the product. It is only our test bench (§Testing).

## What exists today

| Need | Contract | Where (iii-cloud `d31ed9457`) | Gap |
|---|---|---|---|
| API base | `<cloud_url>/api/v1`, same origin as the console | `contracts/openapi/cloud-v1.yaml:27-28` | — |
| API key | `Authorization: Bearer iii_<16hex>_<64hex>`, one environment, `read` / `deploy` (`mcp` grants nothing today) | `crates/access/src/keys.rs:34-50,92-209,228`; yaml 7618-7623 | 23 routes only. No `/me`, no project or environment list, no variable or settings write. A route outside the list is 401 "This API key can't call this route." (`keys.rs:497-503`) |
| Session writes | Import, settings, variables, GitHub install: session only | yaml 3394, 3216, 3749, 6147 | A browser cookie (`iii_session`) is all a CLI could hold, and it can't |
| Device sign-in | RFC 8628: `POST /auth/device`, `/auth/device/token`, `/lookup`, `/approve`; token `iiia_<64hex>`, 90 days, never extended | `crates/identity/src/http/device.rs:19-28,121-196,206-311,377-434` | `platform` only `ios`, `ipados`, `macos`, `android` (`device.rs:28`). The app session reaches only `/me`, sign-out and `/teams/{t}/computers*` (`crates/access/src/http.rs:273-309`) |
| Teams | `GET /me` → `teams[] {id, name, slug, personal, role}` | yaml 685, 8362-8415 | Session or app session only |
| Projects | `GET /teams/{t}/projects` → `Project {id, name, repository {id, full_name, default_branch}}` | yaml 3363, 10620-10668 | No filter by repository: match `full_name` client side |
| Environments | `GET /teams/{t}/projects/{p}/environments`; `GET /teams/{t}/environments/{e}` (key `read`) → `{id, project_id, name, production, branch, manifest, url, settings}` | yaml 3570, 3703, 10901 | — |
| Import | `GET /teams/{t}/github/repositories` (manifests per repo); `POST /teams/{t}/github/install` → `install_url`; `POST /teams/{t}/projects {repository_id, manifest, environment_name, production}` | yaml 3174, 3216, 3394, 10546-10714 | Session only, Owners and Admins |
| Plan | `GET /teams/{t}/environments/{e}/plan?commit=` (key `read`) → `{can_deploy, plan, fingerprint}` | `crates/deploy/src/api.rs:676-708`; yaml 4624, 11440-11806 | Refuses a commit off the environment's branch with 404 (`api.rs:689-692`). The plan stops at its first blocking step (`crates/plan/src/lib.rs:156-184`) |
| Plan issue | `{code, severity: block\|warn\|info, kind: resolve_here\|fix_in_repo\|null, location {file, line, path, setting, resource}, excerpt {start_line, lines[]}, message, fix}` | yaml 11373; codes `crates/core/src/error.rs:26-82,220-330` | `MISSING_WORKER_DIRECTORY` is not a code: it is `context.iii_code` under `INVALID_COMPOSE_FILE` (`crates/plan/src/validation_tests.rs:244-249`) |
| Fingerprint | SHA-256 of the whole plan, the source commit included; `If-Match` on create; a mismatch is 409 CONFLICT | yaml 11790-11806, 4955-4962; `api.rs:546-552` | A merge on GitHub always makes a new commit, so the reviewed fingerprint never matches the merged commit (D5) |
| Deploy | `POST /teams/{t}/environments/{e}/deployments {commit, confirm_resources, confirm_overrides}` + `Idempotency-Key` (required) + `If-Match` (key `deploy`) | `api.rs:525-600`; yaml 4948, 11957 | Off-branch commit is 422 (`api.rs:541-552`, the deploy lock) |
| Push deploys | A push to the env branch deploys. It waits as `needs_action` while resources are created or overrides apply | `crates/github/src/webhook.rs:229-241`; yaml 4956 | No approve route: a new POST for the same commit with the flags replaces the waiting one (`crates/deploy/src/pg.rs` ~580-592). `needs_action` expires after 24 h |
| Deployment | `{id, number, type, status: queued\|in_progress\|needs_action\|ready\|failed\|replaced\|canceled, status_detail, commit {sha, branch, message}, stages[] {name: validation\|build\|resources\|config_secrets\|workers\|routes_tls, status, detail}, error {stage, worker, code, hint, action}}` | yaml 11818-11875 | — |
| Logs | `GET .../logs?deployment_id&source&q&since&until&cursor`, newest first (key `read`) | yaml 6259-6360 | — |
| No route | An http group that serves no route is a log line (`source: engine`), never a failure | `crates/control-plane/src/workers.rs:104-116,787-802` | The worker must count it as a failure (§9) |
| Healthcheck | Runs only when `settings.healthcheck` is set | `crates/control-plane/src/routes_tls.rs:178,292-308` | — |
| Variables | `GET .../variables` (key `read`) → `{name, type, value: null for secrets, status: set\|missing\|empty_default, required_by}`; `PUT .../variables/{name} {type, value}` | yaml 6104-6155, 12459-12492 | `PUT` is session only; production needs Owner or Admin |
| Support matrix | `GET /support-matrix` (public) → `{engine, minimum_sdk, runtimes, …}`; engine `0.27.0`, minimum SDK `0.26` | yaml 5289, 12022; `crates/plan/src/matrix.rs:178,183` | — |
| Frozen install | npm `npm ci`; pnpm `pnpm install --frozen-lockfile`; yarn classic `yarn install --frozen-lockfile`; bun `bun install --frozen-lockfile`; pip `pip install -r requirements.txt` in a venv; uv `uv sync --frozen --inexact`; go `go mod download`; cargo `cargo fetch --locked` | `crates/build/src/lib.rs:411-430` | — |
| Resources | Plan `resources[] {kind: database\|storage\|queue\|state, alias, mode, action: create\|reuse\|…, via, binding, direct_binding}`; Postgres binds `DATABASE_<ALIAS>_URL` | yaml 11690-11740; `crates/plan/src/resources.rs:390-402` | — |
| Error envelope | `{code, message, fix?, doc_url?}` | yaml 8291-8341 | — |

Workers repo, reused as they are:

| Need | Contract | Where |
|---|---|---|
| Rust worker layout, golden schemas | `functions::catalog()` + `tests/schemas.rs` + `tests/golden/schemas/<id>.json` | `github/` |
| Console UI serving | `iii-console-ui` crate (`ConsoleUi::new(..).script(..).style(..)`), `build.rs` → `crates/console-ui/src/ui_build.rs` | `github/src/ui.rs`, `github/build.rs` |
| UI build and design lint | `buildWorkerUi({scope, keyframePrefixes, lint: {strict: true}})` | `github/ui/build.mjs`, `packages/console-ui/build-worker-ui.mjs` |
| Configuration | register schema with `metadata.ui_form`, `III_CONFIG_NAME`, `configuration::on-updated` hot reload | `github/src/configuration.rs` |
| Coded errors | `Error::Remote` keeps `code` on the wire | `secrets/src/error.rs:7-80` |
| Vault | `secrets::compare-and-set {name, expected, value, consumers}`, `secrets::resolve {ref}` → `{name, value}`; console `secrets::set` | `secrets/src/api.rs:94-162`, `secrets/README.md` |
| Device code + vault precedent | device start and poll, token kept with compare-and-set | `provider-openai-codex/src/session.rs:165-232` |
| Private event function | the worker calls it fire-and-forget (`CallAction::Void`); pages and agents observe it | `github/src/events.rs`, `secrets/src/events.rs` |
| Agent wake | `harness::observe {function_id, filters: {"input.kind": {"$eq": …}}}`, once by default | `harness/src/functions/observe.rs:84-125` |
| Trace hiding | metadata `trace_hidden: true`; `.observable(false)` refuses observers | `docs/sops/trace-hidden-functions.md`, `secrets/src/functions.rs:153` |
| Profiles and skills from a worker | `<worker>/agents/<id>.md` → `agents/<id>.md`; `<worker>/skills/SKILL.md` → id `<worker>/index`; `skills/<rel>.md` → `<worker>/<rel>` | `docs/architecture/skills-and-permissions.md`, `iii-directory/src/fs_source.rs:223-250`, `kanban/agents/` |
| Open a page beside the chat | `ade::workspace::open {screen: "ext:<page>"}`; move the session: `ade::working-directory::propose` | `ade/skills/SKILL.md:47-50,103-105` |
| Page → chat | `host.chat.compose({text, submit, beside})` | `packages/console-ui/index.d.ts:861-905` |
| Key field, folder picker | `host.components.SecretKeyField`, `DirectoryPicker` | `packages/console-ui/index.d.ts:2553-2585,2618-2648` |
| Real engine in tests | `III_ENGINE_BIN`, ephemeral port; skip when unset | `github/tests/support/real_engine.rs` |

## Decisions

These are defaults. Each is open to the owner's review.

- **D1. A Rust worker** in `iii-cloud/`, laid out like `github/` (external
  API, CLI, page, configuration). `iii-sdk = "=0.27.0"`, edition 2024,
  version `0.1.0`.
- **D2. One credential at a time,** in the harness vault. The browser
  session token is the secret `IIICLOUD_SESSION`. An API key is any secret
  the person names. A new `connect` replaces the current credential. There is
  no sign-out function: the person signs a device out in Account → Devices,
  and the next call reports `expired`.
- **D3. The API-key path needs the team and environment ids.** A key cannot
  read `/me` or list projects (see the table above). The page asks for them
  next to the secret name (§14, deviation from board 01). Open decision: a
  key "whoami" route in the cloud would remove the field.
- **D4. The worker's records live in one JSON file,** `<data_dir>/state.json`,
  written to a temp file and renamed. Run logs go next to it. The file holds
  no secret value. No `state` worker dependency.
- **D5. "Pinned to the plan you saw" is a digest, not the fingerprint.** The
  fingerprint covers the commit, and a merge on GitHub always makes a new
  commit, so it never matches. The worker hashes the plan without the commit
  (§8). After the merge it plans the merged commit, compares digests, and
  sends `If-Match` with the merged commit's fingerprint. The cloud then
  guarantees nothing changed between that read and the create.
- **D6. Confirming a push deploy is a new create.** There is no approve
  route. The worker creates a deployment for the merged commit with the
  confirmations, which replaces the waiting one.
- **D7. The profile and the skill ship inside the worker:**
  `iii-cloud/agents/cloud-ready.md` and `iii-cloud/skills/*.md`. The
  registry routes `agents/<id>.md` to the agents folder on download, as for
  `kanban/agents/`. No kit.
- **D8. The worker shells out to `git`, `gh`, `docker` and the configured
  `iii`.** It does not depend on the `github` worker, which the harness
  template does not include. `gh` is used for one thing: reading and merging
  the pull request with the person's login.
- **D9. The run here uses Docker only for Postgres.** Without Docker, a
  project with a database resource fails the run at its release stage with
  `DOCKER_MISSING`. A project without one runs without Docker.
- **D10. Warnings do not gate Ready.** The skill has the agent bring each
  warning to the person, as the design asks.
- **D11. Every `iii-cloud::*` function is open to the agent,** except the one
  that takes a value (`iii-cloud::secret::provide`) and the internal ones.
  `deploy` included. The approval gate can hold it; it ships off.
- **D12. Only the run engine changes ports.** The run copy of the compose
  file moves the engine and every `http` container to free ports, so the
  app never meets the harness's engine (49134) or its http (3111).
- **D13. The two cloud changes are a route gate and a removed check** (§16).
  The device session stays an app session, with a new platform `worker`
  that the app-session gate lets through everywhere except the excluded
  areas. The `Principal` type does not change.

## Design

### 1. The worker

```
iii-cloud/
  Cargo.toml  Cargo.lock  build.rs  iii.worker.yaml  iii-permissions.yaml
  config.yaml  README.md
  src/        main.rs lib.rs config.rs configuration.rs error.rs events.rs
              store.rs cloud.rs connect.rs project.rs check.rs local.rs
              run.rs secret.rs deploy.rs watch.rs git.rs ui.rs
              functions/mod.rs (catalog + registration)
  tests/      schemas.rs contract.rs … golden/schemas/iii-cloud.*.json
  ui/         package.json build.mjs tsconfig.json vitest.config.ts page.tsx
              styles.css src/**
  skills/     SKILL.md onboarding.md plan-codes.md
  agents/     cloud-ready.md
```

The file list in `src/` is a guide, not a contract. Keep files few.

`iii.worker.yaml`:

```yaml
iii: v1
name: iii-cloud
language: rust
deploy: binary
iii_engine_version: ">=0.26.0"
manifest: Cargo.toml
license: Apache-2.0
bin: iii-cloud
tags: [cloud, deploy, iii-cloud, readiness, onboarding]
description: Take any project to iii cloud from your harness — sign in, check the cloud plan and run the app here the way the cloud runs it, keep secret values out of the agent's context, merge and deploy pinned to the plan you saw, and follow the deploy to Ready.
dependencies:
  configuration: "latest"
  secrets: "latest"
config:
  cloud_url: https://cloud.iii.dev
  iii_bin: iii
  merge_method: squash
  postgres_image: postgres:17
  run_timeout_s: 900
  data_dir: data/iii-cloud
```

Configuration (entry `iii-cloud`, or `III_CONFIG_NAME`, hot-reloaded like
`github/src/configuration.rs`):

| Field | Default | Meaning |
|---|---|---|
| `cloud_url` | `https://cloud.iii.dev` | The console. The API is `<cloud_url>/api/v1`, the device page `<cloud_url>/device`. A change signs out (the token belongs to one cloud) |
| `iii_bin` | `iii` | The `iii` the run here uses. It must report the cloud's engine version |
| `merge_method` | `squash` | `merge`, `squash` or `rebase`: the `gh pr merge` flag |
| `postgres_image` | `postgres:17` | The local Postgres for a database resource |
| `run_timeout_s` | `900` | The whole run here, from install to healthcheck |
| `data_dir` | `data/iii-cloud` | Records and run logs, relative to `III_COMPOSE_DIR` |

The worker registers with the name `iii-cloud`: the vault's consumer lists
match on it.

### 2. Credentials: `iii-cloud::connect`

**Browser (the main path).**

1. `POST <cloud_url>/api/v1/auth/device {name: "iii-cloud worker on <hostname>", platform: "worker"}`.
   The name is cut to 64 characters, control characters removed.
2. The call returns at once with `Connection {status: "waiting", device:
   {user_code, verification_uri, expires_at}}`. The worker never builds a
   link with the code in it.
3. A background task polls `POST /auth/device/token {device_code}` every
   `interval` seconds (5), adding 5 s on a 429. It stops on 200,
   `DEVICE_CODE_EXPIRED`, `ACCOUNT_BLOCKED` or after `expires_in` (600 s).
4. On 200 it stores the token with `secrets::compare-and-set {name:
   "IIICLOUD_SESSION", expected: <current value or null>, value: <token>,
   consumers: ["iii-cloud"]}`. It then reads `GET /me`, records the user, the
   teams and `expires_at`, and emits `connection` (§10).
5. Today the cloud answers step 1 with 422 (no platform `worker`). The worker
   returns `BROWSER_SIGN_IN_UNAVAILABLE`, "This iii cloud does not accept
   worker sign-in yet; use an API key." (§16 C1).

**API key.** The person saves the key from the page (§14). The page writes it
with `secrets::set {name, value, consumers: ["iii-cloud"]}` (a console call)
and then calls `connect {method: "api_key", secret, team_id,
environment_id}`. The worker resolves the key with `secrets::resolve {ref:
"secret://<secret>"}`, checks it with `GET
/teams/{team_id}/environments/{environment_id}`, and records it.

- 401 → `KEY_REJECTED`; 404 → `KEY_WRONG_ENVIRONMENT`.
- With a key, every session-only route the agent tries through
  `iii-cloud::api` comes back as `NEEDS_BROWSER_SIGN_IN` (a 401 whose message
  says the key can't call the route).

**Every cloud call.** The token is resolved once per process start and after
each connect, and kept in memory. It never reaches a log, a record, an event,
a function result or an error message. A 401 on either credential (other
than a key's route refusal) sets `status: "expired"`, with the error
`NOT_CONNECTED` (session) or `KEY_REJECTED` (key), and emits `connection`.

`connect` is registered with `.observable(false)` and metadata
`{"trace_hidden": true}`. Its input and output never carry a token; the
`user_code` is shown on screen by design.

### 3. Project and worktree

**`iii-cloud::project::resolve {path, team_id?}`** reads, without changing
anything:

- `git -C <path> rev-parse --show-toplevel`, else `NOT_A_GIT_REPO`;
- the raw `git config --get remote.origin.url` (raw, so an `insteadOf`
  rewrite does not hide the GitHub name), parsed as `owner/name` from the
  `https://github.com/`, `git@github.com:` and `ssh://git@github.com/` forms,
  else `NO_GITHUB_REMOTE`;
- the checkout: current branch, `HEAD`, changed files (`git status
  --porcelain`), commits ahead of its upstream.

With a browser session it then finds the cloud side:

1. Teams from `GET /me`. With `team_id`, only that team.
2. In each team, `GET /teams/{t}/projects`, matching
   `repository.full_name` without case.
3. Found: `GET /teams/{t}/projects/{p}/environments`. The default is the
   production environment, else the one whose branch is the repository's
   default branch.
4. Not found: `GET /teams/{t}/github/repositories`, matched the same way,
   gives `importable {repository_id, manifests[]}`. When the repository is
   not listed, `github_access: false`. The agent then calls `iii-cloud::api
   POST /teams/{t}/github/install` and asks the person to open the
   `install_url` (one of the two browser moments in the design).
5. Several teams and no match: `team: null` with the teams listed; the agent
   asks which one.

With an API key, the environment comes from `GET
/teams/{t}/environments/{e}`. The repository match cannot be checked, so
`cloud_project` stays null and `github_access` null.

**`iii-cloud::project::use {path, environment_id, mode, bring_changes?}`**
selects the environment for every later call and prepares where the agent
works.

- `mode: "worktree"` (recommended):
  - `git -C <path> fetch origin <branch>`;
  - path `<parent of the checkout>/<checkout dir>-iii-cloud`, branch
    `iii-cloud/ready-<environment name>`;
  - `git -C <path> worktree add -b <branch> <worktree> origin/<branch>`;
  - an existing worktree of this repository on that branch is reused; a path
    taken by anything else is `WORKTREE_PATH_TAKEN`;
  - a fix branch left without a worktree (here or on origin) holds the
    agent's earlier work and is resumed under the same name: the local
    branch (fast-forwarded to origin's when origin is ahead), else origin's,
    then `origin/<branch>` comes in (fast-forward, else a merge commit; a
    conflict is `MERGE_CONFLICT` and leaves it as it was). A fix branch with
    nothing beyond the base starts at the base. Nothing is force-pushed or
    deleted;
  - only a name out in the checkout itself, or in a worktree the worker did
    not make, moves on to `ready-<env>-2`, `-3`, ...;
  - `workspace.source_sha` is the commit the source (`source`, else the
    checkout's HEAD) pointed at when the worktree took it: "moved since"
    compares the source's tip with it, never with `from_head`.
- `mode: "checkout"`: the agent works in `<path>`; nothing is created.
- `bring_changes: true` copies the checkout's uncommitted changes into the
  worktree: `git diff HEAD --binary | git apply` in the worktree, then the
  untracked files from `git ls-files --others --exclude-standard`. A patch
  that does not apply is `BRING_CHANGES_CONFLICT`, listing the files, and
  nothing is half-applied (apply with `--check` first). Unpushed commits are
  reported (`checkout.ahead`), not brought.
- The worker adds `/.iii/`, `*.iii-cloud-run.yaml`, `*.iii-cloud-run.lock`
  and `.env` to the repository's `info/exclude`
  (`git rev-parse --git-path info/exclude`, shared with the checkout: these
  patterns only hide the run's own files).

It records `Project.workspace` and emits `project`. The agent then moves its
session there with `ade::working-directory::propose` (skill).

### 4. Readiness: `iii-cloud::check {commit?, run?}`

Requires a selected environment (`NO_PROJECT`) and a connection
(`NOT_CONNECTED`). It runs three parts and records one `Check`.

**The commit.** Default: the workspace's `HEAD`. The cloud can only plan a
commit GitHub has, so the worker runs `git fetch origin` and `git branch -r
--contains <sha>`. When no remote branch contains it, the plan part is
`not_pushed` and the local checks still run. An explicit `commit` is used as
given. The local checks always read the workspace's files.

**The cloud plan.** `GET /teams/{t}/environments/{e}/plan?commit=<sha>`.

| Answer | `plan.status` |
|---|---|
| 200, `can_deploy: true` | `clean` |
| 200, `can_deploy: false` | `blocked` |
| 404 whose message is about the commit | `commit_not_planned` ("iii cloud can't see this commit in the repository yet (push the branch)"); any other 404 (no such environment) is `unavailable` with the cloud's message. The cloud plans any commit of the repository since iii-cloud#546; deploy stays branch-only, and the worker answers `MERGE_FIRST` for a commit off the env branch |
| Any other error | `unavailable`, with the error |

From a 200 the worker keeps:

- `fingerprint`;
- `digest` (§8);
- counts of `block` and `warn` issues;
- `at_least: true` when anything blocks (the plan stops at its first
  blocking step);
- the confirmations a deploy will need: `resources[]` with `action:
  "create"`, and `overrides[]`;
- `web_candidates`, passed through for the web conversation (skill).

Every `issues[]` and `deploy_issues[]` entry becomes a `Problem` with `source:
"cloud_plan"`:

- `code` = `context.iii_code` when present, else `code`, and `cloud_code` =
  `code`;
- `title` = the first sentence of `message`, at most 100 characters;
- `detail` = `<code> · <file>[:<line>]`;
- `file`, `line`, `excerpt` from `location` and `excerpt`;
- `owner` from the table in `skills/plan-codes.md`.

The owner table's default is: `fix_in_repo` → `agent`, `resolve_here` →
`person`. Overrides, all → `person`:

- `MISSING_WORKER_DIRECTORY` (path or package is a choice);
- `RUNTIME_UNSUPPORTED`;
- `LITERAL_CREDENTIAL`;
- `SECRET_MISSING`;
- `MULTIPLE_HTTP_GROUPS`;
- every `WEB_*` code (design: the person answers the web part);
- the warnings `LOCAL_CONFIG_NOT_SET`, `ENV_FILE_MISSING` and
  `EMPTY_DEFAULT_VARIABLE`.

A golden test pins the table.

**The checks here.** Fast, read-only, on the workspace. Each yields a
`Problem` with `source: "check_here"`, `severity: "block"` unless noted, and
fixed English text:

| Code | When | Title / detail | Owner |
|---|---|---|---|
| `LOCAL_ENGINE_VERSION` | `<iii_bin> --version` ≠ `engine` from `GET /support-matrix` | "iii {local} here, the cloud runs {engine}" / "The run here uses the cloud's engine. 0.26 also refuses the `cloud:` block." | person |
| `SDK_TOO_OLD` | A `path://` worker's `iii-sdk` is older than `minimum_sdk` | "iii-sdk {v} is older than {min}" / "The deploy would go green and the http group would serve no route." | agent |
| `RUNTIME_LOCK_MISSING` | A worker's runtime has no matching lockfile at the root directory (the install table above) | "Runs with {runtime}, but the repository has only a {found} lock" / "The cloud installs {runtime} projects from {lock}: nothing would be installed." | person |
| `RELEASE_IN_CI` | No `cloud.release` and no `settings.release`, and a file in `.github/workflows/` runs a migration (`db:migrate`, `migrate deploy`, `migrate up`, `alembic upgrade`, `sqlx migrate run`, `diesel migration run`, `knex migrate`) | "Migrations run in your CD, not in the cloud" / "{file} runs {cmd}; it moves to cloud.release." | agent |
| `HEALTHCHECK_MISSING` | No `cloud.healthcheck` and no `settings.healthcheck` | "No healthcheck" / "Without one, a deploy is Ready even when nothing answers." | agent |
| `LOCAL_VARIABLE_UNSET` | A `${NAME}` without default in the compose file has no value in the workspace `.env` and is not a resource binding | "{NAME} has no value here" / "The run here needs it." | person |
| `LOCK_MISSING` | `<manifest stem>.lock` is absent, only when the plan part is not `clean`/`blocked` (else the plan reports it) | "Lock missing" / "{lock path}" | agent |

Notes for the implementer:

- **Runtime:** use the plan's `workers[].runtime` when the plan answered.
  Otherwise use `cloud.runtimes`, then the worker directory's
  `iii.worker.yaml`.
- **SDK version:** the lockfile's resolved version where it is easy to read
  (`package-lock.json`, `Cargo.lock`, `uv.lock`). Otherwise use the lowest
  version of the range in `package.json` or `pyproject.toml`. Mark that
  heuristic with a `ponytail:` comment.
- **Settings:** `settings.*` comes from `GET /teams/{t}/environments/{e}`.
- **Results:** `Check.here` also carries `iii_version` and
  `accepts_cloud_block` (≥ 0.27.0), for the note on board 04.

**Ready.** `ready = plan.status == "clean"` and no `block` problem from the
checks here and `run.status == "passed"` for this commit.

`report` is the readiness text the agent pastes into the pull request body:

```
## iii cloud readiness
Cloud plan for 3c4d2e0: no blocking issue.
Ran here like the cloud: 3 of 3 workers, 14 routes under /api, healthcheck 200.
Not covered: the cloud's build container, production's secret values, its machine size and real traffic.
```

The agent adds its own "Changes:" line.

**With `run: true`** the run here (§6) starts after the two parts. It starts
even when they found problems: the agent may want the evidence. The call
waits for it at most 240 s, under the harness's 300 s dispatch timeout
(`harness/src/config.rs:346`). It then returns with `run.status: "running"`.
The end of the run is always emitted as `run` (§10), so the agent can
observe it. One run at a time: a second gets `RUN_BUSY`.

### 5. Secrets: `iii-cloud::secret` and `iii-cloud::secret::provide`

The agent asks:

```
iii-cloud::secret {name, description, mode?: "ask" | "not_needed" | "decide"}
```

- `ask` (default) opens a request on the Secrets tab and returns `state:
  "waiting"`.
- `not_needed` and `decide` only annotate the row ("not needed", "your
  decision" on board 05). They return `state: "noted"`.
- `name` must match `^[A-Z_][A-Z0-9_]*$` and must not start with `III_`
  (`INVALID_REQUEST`).
- Asking the same name twice returns the open request.

The person answers on the page, which calls
`iii-cloud::secret::provide {request_id, value}` or `{request_id,
leave_empty: true}`:

- **iii cloud:** `PUT /teams/{t}/environments/{e}/variables/{name} {type:
  "secret", value}` with an `Idempotency-Key`. With an API key this route is
  session only, so `cloud: "skipped"` and the page says to set it in the
  console.
- **Here:** `NAME=value` in `<workspace>/<manifest dir>/.env`. The worker
  replaces the line in place or appends it, keeps the file mode `0600`, and
  writes atomically. A value must fit on one line (`INVALID_REQUEST`).
- `leave_empty` writes nothing anywhere and records `left_empty`.
- The record and the event carry the state only: `set`, `left_empty` or
  `failed`, with `cloud` and `local` outcomes. Never the value.

`secret::provide` is denied to agents (§13), registered `.observable(false)`
with `{"trace_hidden": true}`, and never logs its input. The e2e leak check
proves it (§Testing).

The Secrets tab's second table is the cloud's `GET .../variables` (names,
`type`, `status`, `required_by`) merged with the requests. The chip is:

| Chip | When |
|---|---|
| `asked` | open request |
| `your decision` | `decide` |
| `not needed` | `not_needed` |
| `set` | `set` |
| `empty by default` | `empty_default` |
| `missing` | `missing` |

The faint text is the request's description, else `required_by`.

### 6. Run here like the cloud

The run belongs to a check, `check {run: true}`, and records a `Run` with
seven stages. It uses the workspace's files. Stages run in this order and
stop at the first failure:

| Stage | What it does | Detail on the board |
|---|---|---|
| `install` | For each distinct runtime of the `path://` workers: the cloud's frozen install command (table above) at the root directory (`settings.root_directory`, else the repository root). The container-only parts are dropped (corepack's install directory; the venv goes to `<root>/.venv`). A missing tool is `TOOL_MISSING` | `pnpm install --frozen-lockfile` |
| `build` | `cloud.build.command` from the file, else `settings.build_command`, in `sh -c` at the root directory, with only `PATH`, `HOME` and `TMPDIR` set (the cloud's build has no variables). Skipped when unset | the command |
| `release` | Start the local services for the plan's resources, then `cloud.release` (else `settings.release`) `{worker, command, timeout}`. It runs in that worker's directory, with that container's `environment` (interpolated from the run environment) plus the bindings. Skipped when unset; the services still start | `db:migrate`; "local Postgres" when one started |
| `compose_up` | `env -i PATH HOME TMPDIR <run env> <iii_bin> compose --up -f <run copy> -n iii-cloud-run --frozen`, from the manifest directory, as a child process. Output goes to `<data_dir>/runs/<id>/compose.log` | the container names |
| `workers` | Poll the run engine's `engine::workers::list` every second until every container's worker is connected and stays so for 2 s. Fails if the compose process exits or the compose `startup_timeout` passes | `3 of 3` |
| `routes` | The run engine's registered triggers of type `http`, the list the cloud reads (`registered-triggers::list`, `crates/core/src/engine.rs:16`; confirm the id on 0.27.0). An http group with none is `NO_ROUTE`, as the deploy would be (§9). With `cloud.web.api.prefix` and `strip: false`, every route must start with the prefix | `14 under /api` |
| `healthcheck` | `GET http://127.0.0.1:<http port><path>` until 200 or `timeout` (`cloud.healthcheck`, else `settings.healthcheck`). With `web.api.prefix` and `strip: true`, the prefix is removed from the path first (confirm against the cloud's `cluster.health.check` before coding; the test pins the rule). Skipped when unset | `GET /api/health 200` |

**The run copy.** The worker writes `<manifest stem>.iii-cloud-run.yaml`
next to the manifest, so relative `path://` entries still resolve. It copies
the lock to `<manifest stem>.iii-cloud-run.lock`. A frozen lock checks only
the package declarations (`crates/iii-compose/src/lockfile.rs:321-345`), so
the copy still matches. The copy changes only:

- `engine.url: ws://127.0.0.1:<free port>`, and
  `engine.workers.iii-worker-manager: {host: 127.0.0.1, port: <same>}`, as
  the fabrica stack does (D12);
- for every container whose worker is `package://http`,
  `config_override.port: <free port>` and `config_override.host:
  127.0.0.1`;
- for every resource the run starts, the worker config the cloud writes for
  it. Copy the shape from the cloud (`git grep -n databases origin/main --
  crates/control-plane/src crates/plan/src`) and golden-test it.

Free ports come from binding `127.0.0.1:0`. Never 49134 or 3111.

**The run environment** is the workspace `.env` (parsed `KEY=VALUE`), plus
the resource bindings. Nothing else from the worker's own environment passes
through, except `PATH`, `HOME`, `TMPDIR`.

A `${NAME}` the workspace `.env` lacks is filled from the person's folder
(`project.path`): `.env.<env name>`, `.env.local`, `.env`, then
`.env.production`, each in the compose file's folder, else the root, with
the `values` file rules (plain files inside the repository, at most 1 MiB,
never `.env.example`). Never iii's own names, a binding or a database URL.
The filled values go into the run's process environment only, and into
`known_secrets`. Child output reaches `run.log` and `compose.log` through a
redacting pump, so the log files and `log_tail` never hold a known value.
`LOCAL_VARIABLE_UNSET` (a check here) counts the filled names as set: only a
name with no source anywhere is raised. `values` says per name whether the
run here has it (`here`, `here_file`).

**Resources.**

- `database` resources start one container each: `docker run -d --rm --name
  iii-cloud-run-pg-<run id 8> -e POSTGRES_PASSWORD=<random hex> -e
  POSTGRES_DB=app -p 127.0.0.1:<free>:5432 <postgres_image>`. The worker
  waits for `pg_isready` through `docker exec`, up to 30 s.
- `binding` and `direct_binding` get `postgres://postgres:<pw>@127.0.0.1:<port>/app`.
- `queue`, `state` and `storage` resources do not run here. Their workers
  keep their local defaults, and each is named in `not_covered`.

**Engine version.** When `LOCAL_ENGINE_VERSION` fires, the run does not
start: `install` fails with that code. The guarantee needs the cloud's
version.

**Teardown, always, in this order:**

1. SIGTERM to the compose process (Compose stops the engine it started),
   SIGKILL after 30 s.
2. `docker rm -f` the Postgres containers.
3. Delete the run copy and its lock.
4. Logs stay in `<data_dir>/runs/<id>/`, the newest 20 runs kept.

`Run.log_tail` is the last 40 lines across the stages.

`not_covered` always lists:

- "The cloud's build container";
- "Production's secret values";
- "Its machine size";
- "Real traffic".

It adds one line per resource that did not run here.

### 7. Pull request

The agent opens it (skill):

1. commit in the workspace;
2. `git push -u origin iii-cloud/ready-<env>`;
3. `gh pr create --base <env branch> --head iii-cloud/ready-<env> --title
   … --body <check.report + Changes>`.

The worker only reads it. With `status {refresh: true}` it runs `gh pr view
<workspace branch> --repo <repo> --json
number,url,title,state,body,headRefName,headRefOid,baseRefName,mergeable,statusCheckRollup`.
It also runs `git ls-remote origin refs/heads/<env branch>`, so that
`main_moved` is true when the env branch's head is no longer `check.base`.
No pull request → `pr: null`. A `gh` failure (not installed, not logged in)
sets `pr: null` and `pr_error` (`GH_FAILED`), which the page shows.

### 8. Merge and deploy: `iii-cloud::deploy {check_id, pr?}`

The **plan digest** is SHA-256 over canonical JSON (sorted keys, no
whitespace) of `{overrides, settings, workers, groups, resources, variables,
issues, deploy_issues, manifest_sha256: source.manifest_sha256,
lock_sha256: source.lock_sha256}`. `source.commit`, `source.branch` and
`environment` are left out. A unit test pins it against a recorded plan.

Steps:

1. **The check.** It must exist, be of the selected environment, and be
   `ready`, else `NOT_READY`.
2. **With `pr`:**
   - `gh pr view <pr> --json state,headRefOid,baseRefName,mergeable` must
     show `OPEN`, head = `check.commit` (else `PR_HEAD_CHANGED`) and base =
     the env branch;
   - `git ls-remote` must show the env branch head at `check.base`, else
     `MAIN_MOVED` (the page offers "Check again");
   - then `gh pr merge <pr> --repo <repo> --<merge_method>`, never `--admin`
     or `--auto`. A refusal is `PR_NOT_MERGEABLE` with gh's message;
   - the target is `mergeCommit.oid`, read with `gh pr view` until it
     appears, up to 30 s.
3. **Without `pr`:** the target is `check.commit`, which must already be on
   the env branch. The cloud refuses others with 422.
4. **Pin.** `GET plan?commit=<target>`. A digest other than
   `check.plan.digest` is `PLAN_CHANGED`, with the top-level keys that
   differ. Nothing is deployed.
5. **The push's deployment.** After a merge, poll `GET .../deployments`
   (first page) every 5 s, up to 120 s, for one with `commit.sha = target`:
   - `needs_action` → step 6;
   - `queued`, `in_progress` or `ready` → adopt it: no create (it needed no
     confirmation), go to step 7;
   - not seen in 120 s → step 6. A late push for the same commit can then
     add a second deployment; mark it `ponytail:`, since the webhook is
     normally seconds away.

   Without a merge, an open `needs_action` deployment for the target goes to
   step 6; otherwise step 6 creates one.
6. **Create.** `POST /teams/{t}/environments/{e}/deployments {commit:
   target, confirm_resources: <plan has a create>, confirm_overrides: <plan
   has overrides>}`, with `If-Match: <the target plan's fingerprint>` and
   `Idempotency-Key: iii-cloud:<check_id>:<target>`.
   - 409 → `PLAN_CHANGED`;
   - 403 → `FORBIDDEN` with the cloud's message (resources in production
     need Owner or Admin);
   - 422 → `CLOUD_ERROR`.
7. **Follow.** Record the `Deploy`: `pr`, `approved_by_worker` (true when
   step 6 ran), and `checked_ready: true`. Start following it (§9), emit
   `deploy`, return the record.

`deploy` is open to the agent (D11). The skill tells it to deploy only when
the person asks; the page's "Merge and deploy" is the usual path.

### 9. Watch: `iii-cloud::watch {deployment_id?}`

`watch` returns the current `Deploy` and makes sure a follower runs for it.
Without an id it picks the newest deployment of the selected environment.

A follower polls `GET .../deployments/{id}` every 5 s:

- **Each change** of `status`, `status_detail` or a stage status emits
  `deploy` with `terminal: false`.
- **Terminal** (`ready`, `failed`, `canceled`, `replaced`):
  - it reads the last 20 log lines: `GET .../logs?deployment_id=<id>`,
    newest first, reversed;
  - on `ready` it also asks `GET .../logs?deployment_id=<id>&source=engine&q=serves no route`.
    A line there makes the outcome `no_route`, a failure with code
    `NO_ROUTE` and that line as the hint (design: "um http sem rota conta
    como falha");
  - it records `outcome` and `error`, and emits `deploy` with `terminal:
    true`. The agent observes that.
- **The release row** of board 09 is derived, because the cloud runs the
  release inside the `workers` stage:
  - shown when the plan's settings have a release;
  - `failed` on `RELEASE_FAILED`;
  - `passed` once `workers` passes or reports a worker ready;
  - `running` while `workers` runs;
  - `pending` otherwise.

**Pushes made elsewhere** (a merge straight on GitHub, a push by a person):
while connected with a selected environment, a background task polls `GET
.../deployments` every 30 s and follows every deployment created after the
environment was selected. `checked_ready` is true when a ready check of the
same environment has the deployment's commit, or the same digest as the
deployment's plan.

`checked_ready` lets phase 2 count the deploys that fail after "Ready". On
restart the worker resumes following the non-terminal deploys in its store.


**Deploys of every project.** `Status.deploys` is the selected environment's;
`Status.running_deploys` lists every deploy still moving (`outcome: running`,
`needs_action` included), of every project and environment, newest first,
each with where it was made from (`cloud_project_id`, `project_name`,
`repository`, `environment_name`, `path`), recorded when the worker first
stores it with its project selected. The poller follows every known moving
deploy with its own team and environment, with or without a selection, and
after a restart; the store never prunes a moving one.

### 10. Events: `iii-cloud::on-change`

The worker calls this private function fire-and-forget after every record
change, like `github::on-called` (`github/src/events.rs`).

```ts
type ChangeEvent = {
  kind: 'connection' | 'project' | 'check' | 'run' | 'secret' | 'deploy'
  id: string          // the record's id ('connection' and 'project' use 'current')
  terminal: boolean   // run/deploy finished, secret resolved, connection settled
  summary: string     // one line, e.g. "deploy 7a1f3b2 failed at workers: OOMKilled"
  record: Connection | Project | Check | Run | SecretRequest | Deploy
}
```

- The page observes all of it once per tab, with the hub pattern of
  `console-injectable-ui` ("Live data pattern"), and upserts records.
- The agent is woken with `harness::observe {function_id:
  "iii-cloud::on-change", filters: {"input.kind": {"$eq": "deploy"},
  "input.terminal": {"$eq": true}}}`, and the same for `run`, `secret` and
  `connection`.

### 11. Function contracts

| Function | Caller | Input | Output |
|---|---|---|---|
| `iii-cloud::connect` | agent, page | `{method: "browser"}` \| `{method: "api_key", secret: string, team_id?: uuid, environment_id?: uuid}` (ids only as a fallback for older clouds) | `Connection` |
| `iii-cloud::status` | agent, page | `{refresh?: boolean}` | `Status` |
| `iii-cloud::project::resolve` | agent, page | `{path: string, team_id?: uuid}` | `Project` |
| `iii-cloud::project::use` | agent, page | `{path: string, environment_id: uuid, mode: "worktree" \| "checkout", bring_changes?: boolean, from?: "environment" \| "checkout"}` (`from: "checkout"`: the worktree starts at the checkout's HEAD; never for production) | `Project` |
| `iii-cloud::project::create_environment` | page only | `{cloud_project_id: string, name: string, branch?: string, manifest?: string, copy_from?: string, production?: boolean, start_empty?: boolean}` (`start_empty`: copy nothing; not with `copy_from`) | `CreateResponse` |
| `iii-cloud::api` | agent | `{method: "GET"\|"POST"\|"PUT"\|"PATCH"\|"DELETE", path: string, query?: object, body?: any, if_match?: string, idempotency_key?: string}` | `{status: number, body: any}` |
| `iii-cloud::check` | agent, page | `{commit?: string, run?: boolean, push?: boolean, manifest?: string}` (`push`: push the fix branch first; `manifest`: check another compose file, nothing changes in iii cloud) | `Check` |
| `iii-cloud::check::get` | agent, page | `{id: string}` (`status.checks[].id`, a deploy's `check_id`) | `Check` |
| `iii-cloud::secret` | agent | `{name: string, description: string, mode?: "ask"\|"not_needed"\|"decide"}` | `SecretRequest` |
| `iii-cloud::secret::provide` | page only | `{request_id: string, value?: string, leave_empty?: boolean}` | `SecretRequest` |
| `iii-cloud::values` | agent, page | `{manifest?: string, from?: "workspace" \| "checkout"}` (the names the compose file's containers read, each with `need`, `type`, `in_cloud`, `asked` and its `suggestions`: a `.env*` file of the person's folder, the environment's URL, a copy from another environment; names and flags only, a variable's cloud value at most) | `Values` |
| `iii-cloud::values::save` | page only | `{environment_id: uuid, manifest?: string, items: SaveItem[]}` (`typed`, `env_file`, `env_url`, `leave_empty`, `copy`; an API key (`NEEDS_BROWSER_SIGN_IN`) or production without Owner or Admin (`ROLE_REQUIRED`) is refused whole, before any request; a name iii cloud refuses fails only its own item; a 401 or 403 leaves the rest `NOT_TRIED`; env files that are links or leave the repository are never read; hidden from traces) | `ValuesSaved` |
| `iii-cloud::values::skip` | page only | `{environment_id: uuid, names: string[], skip: boolean}` ("skip for now", per environment and name, in `skipped_values`, names only; `skip: false` undoes it. A skipped name runs here with `""` in the process environment and is no `LOCAL_VARIABLE_UNSET`; `EMPTY_DEFAULT_VARIABLE` gets `skipped` and severity `info`; `SECRET_MISSING` gets `skipped` and stays `block`, since iii cloud needs a value, even an empty one) | `ValuesSkipped` |
| `iii-cloud::problem::skip` | page only | `{environment_id: uuid, code: string, skip: boolean}`: "skip for now" of a cloud warning, per environment and code, in `skipped_problems` (`Status.skipped_problems`); `skip: false` undoes it. Only `LOCAL_CONFIG_NOT_SET`; any other code is `INVALID_REQUEST`. The next check's matching plan problems get `skipped` and severity `info` | `ProblemsSkipped` |
| `iii-cloud::config::import` | page only | `{environment_id: uuid, dry_run: boolean, expected_version?: integer}`. Reads the compose file's `engine.workers.configuration` `fs` adapter `directory` (relative to the compose file, from the person's folder, inside the repository, not a link) and its `*.yaml` entries (`id`, `name`, `description`, `value`; plain files, at most 1 MiB), as the engine's `fs` adapter does. An entry maps to the container whose config it is (its `config_name`, else `<namespace>-<key>`, else the bare key), which must be a worker of `GET …/config`; iii cloud keeps console config by container key, and `LOCAL_CONFIG_NOT_SET` goes once one container key has an entry. `value` is flattened to dotted keys (lists whole, nulls left out). Keys `GET …/config` marks not editable go to `set_elsewhere`; secret-looking keys (SECRET, TOKEN, KEY, PASSWORD, PRIVATE, CREDENTIAL in the last segment) unless the value is a `${NAME}` reference, and credential-looking values, go to `left_out`, by key only. A dry run reads only and returns the values of `fields`; an apply refuses a moved version (`CONFIG_CHANGED`) or an unapplied console draft (`CONFIG_DRAFT_PENDING`), sends `PATCH …/config/draft` with the imported fields alone, then `POST …/config/apply {expected_version}` (409 → `CONFIG_CHANGED`, 403 → `ROLE_REQUIRED`), refreshes status and returns no values | `ConfigImported` |
| `iii-cloud::override::choose` | page only | `{environment_id: uuid, setting: string, choice: "file" \| "console"}`: the person's pick for a console override of the newest check, kept in `override_choices` per environment and setting with the hashes of its console and file values; a changed pair drops out of `Status.override_choices`. `NO_SUCH_OVERRIDE` when the newest check has no such override | `OverrideChosen` |
| `iii-cloud::deploy` | page, agent | `{check_id: string, pr?: number, deployment_id?: string}` | `Deploy` |
| `iii-cloud::watch` | agent, page | `{deployment_id?: uuid}` | `Deploy` |
| `iii-cloud::on-change` | worker only | `ChangeEvent` | `ChangeEvent` |
| `iii-cloud::on-config-change` | engine | — | `{ok}` |
| `iii-cloud::ui-content` | console | (from `iii-console-ui`) | |

How each function behaves:

- **`status`** reads the store only, with no network. That keeps the page
  fast, and lets the UI fixtures be plain `state.json` files (§Testing).
  `refresh: true` first re-reads:
  - the checkout facts;
  - the PR (§7);
  - the variables (`GET .../variables`);
  - the deployments list.

  The page calls it on mount and every 30 s while visible.
- **`api`**:
  - `path` is relative to `/api/v1`, starts with `/` and has no `..`, `//`
    or scheme;
  - writes get an `Idempotency-Key` (a v4 UUID unless given);
  - a `PUT …/variables/{name}` whose body has `type: "secret"` is refused
    with `SECRET_VIA_API`: values go through the page;
  - the response body is passed through, except a 401 on a browser session,
    which behaves as in §2.
- **`project::use`, `check`, `deploy` and `watch`** act on the selected
  environment.

Types, in TypeScript notation. The Rust structs derive them with serde
(`snake_case`) and schemars; the golden files are the source of truth. Task B
copies these into `ui/src/types.ts`.

```ts
type WorkerError = { code: string; message: string; status?: number; fix?: string }

type Status = {
  worker: { version: string; iii_version: string | null; cloud_engine: string | null }
  connection: Connection
  project: Project | null
  check: Check | null              // the selected environment's latest
  secrets: SecretRequest[]         // open first, then the latest 20 resolved
  variables: { name: string; type: 'secret' | 'variable' | 'builtin';
               status: 'set' | 'missing' | 'empty_default'; required_by: string[] }[]
  pr: PullRequest | null
  pr_error: WorkerError | null
  deploys: Deploy[]                // newest first, at most 20
  refreshed_at: string | null
}

type Connection = {
  status: 'signed_out' | 'waiting' | 'connected' | 'expired' | 'error'
  method: 'browser' | 'api_key' | null
  cloud_url: string
  user: { id: string; email: string; name: string | null } | null
  teams: { id: string; name: string; slug: string; role: string }[]
  api_key: { secret: string; team_id: string; environment_id: string } | null
  device: { user_code: string; verification_uri: string; expires_at: string } | null
  expires_at: string | null
  error: WorkerError | null
}

type Project = {
  path: string
  repository: string                                   // owner/name
  checkout: { branch: string | null; head: string; changed_files: string[]; ahead: number }
  team: { id: string; name: string; slug: string } | null
  teams: { id: string; name: string; slug: string }[]  // when team is null
  cloud_project: { id: string; name: string } | null
  environment: { id: string; name: string; production: boolean; branch: string;
                 manifest: string | null; url: string | null } | null
  environments: { id: string; name: string; production: boolean; branch: string }[]
  importable: { team_id: string; repository_id: number;
                manifests: { path: string; choosable: boolean; suggested_environment: string | null;
                             issue: string | null }[] } | null
  github_access: boolean | null
  workspace: { mode: 'worktree' | 'checkout'; path: string; branch: string } | null
}

type Problem = {
  id: string                       // sha1(source, code, file, line), stable across checks
  source: 'cloud_plan' | 'check_here'
  code: string
  cloud_code: string | null
  severity: 'block' | 'warn' | 'info'
  owner: 'agent' | 'person' | 'none'
  title: string
  detail: string | null
  file: string | null
  line: number | null
  excerpt: { start_line: number; lines: string[] } | null
  fix: string | null
}

type Check = {
  id: string
  environment_id: string
  commit: string
  base: string                     // env branch head when checked
  checked_at: string
  duration_ms: number
  plan: {
    status: 'clean' | 'blocked' | 'not_pushed' | 'commit_not_planned' | 'unavailable'
    fingerprint: string | null
    digest: string | null
    blocking: number
    warnings: number
    at_least: boolean
    confirmations: {
      resources: { kind: string; alias: string; binding: string | null; tier: string | null }[]
      overrides: { setting: string; console: unknown; file: unknown }[]
    }
    web_candidates: unknown | null
    error: WorkerError | null
  }
  here: { problems: number; iii_version: string | null; accepts_cloud_block: boolean }
  problems: Problem[]              // cloud plan first, then the checks here
  run: Run | null
  ready: boolean
  report: string
}

type Stage = {
  name: 'install' | 'build' | 'release' | 'compose_up' | 'workers' | 'routes' | 'healthcheck'
  status: 'pending' | 'running' | 'passed' | 'failed' | 'skipped'
  detail: string | null
  duration_ms: number | null
}

type Run = {
  id: string
  check_id: string
  commit: string
  engine: string
  status: 'running' | 'passed' | 'failed'
  started_at: string
  finished_at: string | null
  stages: Stage[]
  workers: { ready: number; total: number }
  routes: { count: number; prefix: string | null }
  healthcheck: { path: string; status: number | null } | null
  not_covered: string[]
  log_tail: string[]
  error: { stage: Stage['name']; code: string; message: string } | null
}

type SecretRequest = {
  id: string
  environment_id: string
  name: string
  description: string
  mode: 'ask' | 'not_needed' | 'decide'
  state: 'waiting' | 'set' | 'left_empty' | 'noted' | 'failed'
  cloud: 'set' | 'skipped' | 'failed' | null
  local: 'set' | 'skipped' | null
  asked_at: string
  resolved_at: string | null
  error: WorkerError | null
}

type PullRequest = {
  number: number; url: string; title: string; state: 'OPEN' | 'MERGED' | 'CLOSED'
  head: string; head_sha: string; base: string; body: string
  checks: 'none' | 'pending' | 'passing' | 'failing'
  mergeable: string | null
  main_moved: boolean
}

type Deploy = {
  id: string; number: number; environment_id: string
  commit: { sha: string; message: string | null }
  type: string
  status: 'queued' | 'in_progress' | 'needs_action' | 'ready' | 'failed' | 'replaced' | 'canceled'
  status_detail: string | null
  stages: { name: string; status: string; detail: string | null;
            started_at: string | null; finished_at: string | null }[]
  release_row: { status: 'pending' | 'running' | 'passed' | 'failed'; duration_ms: number | null } | null
  outcome: 'running' | 'ready' | 'failed' | 'no_route' | 'canceled' | 'replaced'
  error: { stage: string; worker: string | null; code: string; hint: string; action: string | null } | null
  log_tail: string[]
  pr: number | null
  approved_by_worker: boolean
  checked_ready: boolean
  created_at: string
  finished_at: string | null
  console_url: string
}
```

`console_url`: when the team slug, project and environment names are known,
the console's deployment page,
`<cloud_url>/<team slug>/<project>/<environment>/deployments/<number>`.
Confirm the `$project` segment against
`apps/web/src/routes/$team/$project/$environment/deployments/$number.tsx`.
Otherwise it is `<cloud_url>`.

### 12. Errors

Every function fails with `Error::Remote {code, message}`, as
`secrets/src/error.rs` does. No message contains a token or a value.

| Code | Meaning |
|---|---|
| `INVALID_REQUEST` | Bad input |
| `NOT_CONNECTED` | No credential, or the session expired |
| `BROWSER_SIGN_IN_UNAVAILABLE` | The cloud refuses platform `worker` (before §16 C1) |
| `KEY_REJECTED` / `KEY_WRONG_ENVIRONMENT` | The API key's check failed |
| `NEEDS_BROWSER_SIGN_IN` | The route needs a person's session; the key can't call it |
| `NO_PROJECT` | No environment selected |
| `NOT_A_GIT_REPO` / `NO_GITHUB_REMOTE` | The folder |
| `WORKTREE_PATH_TAKEN` / `BRING_CHANGES_CONFLICT` | `project::use` |
| `BEHIND_ENVIRONMENT` / `MERGE_CONFLICT` | `project::use {from: "checkout"}`: the checkout lacks the environment's head; the worktree and the checkout both moved |
| `PUSH_REFUSED` | `check {push}` on a branch an environment tracks or that is not an `iii-cloud/` fix branch; `deploy` when GitHub refuses the push to the environment's branch |
| `PUSH_UNKNOWN` | `deploy`: the push failed and origin can't be read after, so whether it landed is unknown |
| `RUN_BUSY` | A run is going |
| `NOT_READY` / `PR_HEAD_CHANGED` / `MAIN_MOVED` / `PR_NOT_MERGEABLE` / `PLAN_CHANGED` / `FORBIDDEN` | `deploy` |
| `SECRET_VIA_API` | `api` refused a secret value |
| `GH_FAILED` / `GIT_FAILED` / `TOOL_MISSING` / `DOCKER_MISSING` | A local tool failed; the message has its stderr, cut to 2 KB |
| `CLOUD_ERROR` | Any other cloud error: `status`, and the cloud's `code`, `message`, `fix` |

Stage failure codes (in `Run.error`) reuse these plus:

- `LOCAL_ENGINE_VERSION`;
- `INSTALL_FAILED`, `BUILD_FAILED`, `RELEASE_FAILED`;
- `COMPOSE_FAILED`, `WORKERS_NOT_READY`;
- `NO_ROUTE`, `ROUTE_OUTSIDE_PREFIX`;
- `HEALTHCHECK_FAILED`;
- `RUN_TIMEOUT`.

### 13. Permissions

`iii-cloud/iii-permissions.yaml`, with the same block added to the repo-root
`iii-permissions.yaml` (which the checked-in harness loads):

```yaml
version: 1
rules:
  # Takes a secret value: the person types it on the page (a console call
  # bypasses this gate). The agent only sees that it was saved.
  - '!iii-cloud::secret::provide'
  # Writes the values the person chose on the page, secrets too.
  - '!iii-cloud::values::save'
  # The person's "skip for now".
  - '!iii-cloud::values::skip'
  # The person's "skip for now" of a cloud warning.
  - '!iii-cloud::problem::skip'
  # Copies the person's local config folder into iii cloud.
  - '!iii-cloud::config::import'
  # The person's pick for a console override.
  - '!iii-cloud::override::choose'
  # Private and internal: the worker calls on-change, observers read it.
  - '!iii-cloud::on-change'
  - '!iii-cloud::on-config-change'
  - '!iii-cloud::ui-content'
  # The agent is not restricted (owner decision, 25/11). The approval gate
  # can hold iii-cloud::deploy in production; it ships off.
  - 'iii-cloud::*'
```

### 14. Console UI

**Rules.** The page follows the `ade-worker-design` skill
(`console-injectable-ui.md`, `console-design.md`, `patterns.md`,
`design-console-ui.md`), and the "Desvios já cometidos" list in its
`SKILL.md`:

- primary buttons are ink;
- status words stay ink, and color goes on the dot only;
- chips are at least 24 px; heights are 28, 32 or 36, and 44 in a pane under
  720 px;
- warnings are `StatusPanel`, never loose text.

Components come from `@iii-dev/console-ui`'s `index.d.ts` only. Every rule is
scoped under `[data-iii-ui="iii-cloud"]`. The canvas file
`design/harness.css` is reference, never shipped.

**Registration** (`ui/page.tsx`):

- `host.pages.register({id: "iii-cloud", title: "iii cloud",
  configurationId: "iii-cloud", render})`;
- `host.functionCalls.register(...)` for `connect`, `check`, `secret`,
  `deploy` and `watch`;
- `host.configForms.register("iii-cloud", ConfigForm)`;
- a palette command "Open iii cloud".

The agent opens the page with `ade::workspace::open {screen:
"ext:iii-cloud"}`.

**Live data.** One hub per tab:

- `host.iii.on('iii::iii-cloud-ui::events', …)`;
- `host.iii.observe('iii-cloud::on-change', 'iii::iii-cloud-ui::events::' + browserId)`;
- `iii-cloud::status` on mount, and with `refresh: true` every 30 s while
  the page is visible.

Records upsert by `kind` and `id`.

**Page states and boards.** The page follows the v5 canvas ("iii cloud worker
v5", 26 boards, 26/11/2026), which replaces the v4 boards in `design/`. One
page id `iii-cloud`: `CloudPage.tsx` routes by `route.ts` to Setup, Deploy from
this machine, or Manage (home, an environment's four tabs, a new environment).
Files are under `ui/src/` (CSS under `ui/src/css/`); the fixture boards are in
`ui/src/fixtures/<area>.ts`.

| Board | Files | Fixture boards |
|---|---|---|
| Main | the map of the canvas, no screen | — |
| CheckStage (shared by Onb4, Dep2–Dep4, Dep7) | `CheckStage.tsx`, `AgentRow.tsx`, `SecretEntry.tsx`, `check.css` | `check.ts`: `deploy-checking`, `deploy-problems`, `deploy-fixing`, `setup-fixing`, `deploy-passed` |
| Onb1 Start and sign in | `Setup.tsx`, `ConnectView.tsx`, `Steps.tsx`, `setup.css`, `shell.css` | `setup.ts`: `setup-signin` |
| Onb2 Your code | `Setup.tsx` | `setup-code` |
| Onb3 Where it runs | `Setup.tsx`, `CreateForms.tsx` | `setup-target` |
| Onb4 Check before deploy | `CheckStage.tsx` (`scenario="setup"`) | `setup-fixing` |
| Onb6 Review and deploy | `Setup.tsx`, `FailureCard.tsx` | `setup-ship`, `setup-deploying` |
| Onb7 Live | `Setup.tsx` | `setup-live` |
| Dep1 What goes | `DeployFlow.tsx`, `deploy.css` | `deploy.ts`: `deploy-what` |
| Dep2–Dep4 Checking, problems, fixing | `CheckStage.tsx` (`scenario="change"`) | `deploy-checking`, `deploy-problems`, `deploy-fixing` |
| Dep5 Review and deploy | `DeployFlow.tsx` | `deploy-review` |
| Dep6 Follow to live | `DeployFlow.tsx` | `deploy-follow-running`, `deploy-follow-live`, `deploy-follow-failed` |
| Dep7 When the check can't finish | `CheckStage.tsx` (its panels) | `check.ts`: `state-docker`, `state-stalled`, `state-stopped`, `state-busy`, `state-override`, `state-moved`, `state-signedout`, `state-outdated` |
| Mng1 Home | `Manage.tsx`, `manage.css` | `manage.ts`: `manage-home` |
| Mng5 Phone | `Manage.tsx` (narrow) | `manage-home` |
| Mng6 Overview | `EnvOverview.tsx` | `overview.ts`: `manage-overview` |
| Mng2 Deploys | `EnvDeploys.tsx`, `FailureCard.tsx`, `deploys.css` | `deploys.ts`: `manage-deploys-failed` |
| Mng10 Phone failure | `EnvDeploys.tsx` (narrow) | `manage-deploys-phone` |
| Mng7 Rollback | `EnvDeploys.tsx` | `manage-rollback` |
| Mng3 Settings | `EnvSettings.tsx`, `SecretEntry.tsx`, `settings.css` | `settings.ts`: `manage-settings` |
| Mng11 Change file | `EnvSettings.tsx` | `manage-settings` |
| Mng4 Checks | `EnvChecks.tsx`, `checks.css` | `checks.ts`: `manage-checks` |
| Mng8 New environment | `NewEnv.tsx`, `CreateForms.tsx`, `newenv.css` | `new-env.ts`: `manage-new-env` |
| Mng9 States | `Manage.tsx` and the tab each state concerns | `manage-empty`, `manage-stale`, `manage-deleted` |
| Cards in the chat (no v5 board) | `cards.tsx` | `cardRecords` in `base.ts` |

**Values.** The Setup rail is `signin, code, target, values, check, ship,
live`: after `project::use`, Target goes to Values instead of starting the
check, and Values' button saves what is chosen, then runs the check. One
editor (`iii-cloud::values` in, `iii-cloud::values::save` out) is mounted in
four places: that step; a dialog from Dep1's "Values" row (`from:
'checkout'`, the picked file); a dialog from "Set values" on CheckStage's
value problems and waiting asks (replacing the inline `SecretEntry` rows);
and first in EnvSettings, above the rows for names the file doesn't read.
Each row preselects its first suggestion with `preselect: true`. With
`save: 'api_key'` or `'role'` the editor is read-only, with a Notice.

Chat cards open the page at their record's route: a check at its environment's
Checks tab with `check: <id>`, a secret at Settings, a deploy at Deploys with
`deploy: <id>`. The environment header shows the state of its deployments
(`GET /deployments`): "Live", "Deploying", "Needs your action", "Last deploy
failed" or "Nothing live yet".

**The configuration form** (`host.configForms`, using `SettingsSection`,
`SettingsList` and `SettingsField`):

- Cloud URL (`Input`);
- iii binary (`Input`, mono), with the detected version and the cloud's
  engine next to it;
- Merge method (`Select`);
- Postgres image (`Input`, mono);
- Run timeout (`Input`, seconds);
- Data directory (`Input`, mono);
- a `SettingsRow` "Signed in as … · method · expires …", read from
  `status`.

### 15. Skill and agent profile

`skills/SKILL.md` (id `iii-cloud/index`) follows
`DOCUMENTATION_GUIDELINES.md`:

- frontmatter;
- "When to Use";
- "Boundaries": values never pass through you; the cloud's GitHub App is
  read-only; deploy only when the person asks;
- "Functions": one line each;
- "Observing events": the `harness::observe` filters of §10.

`skills/onboarding.md` (id `iii-cloud/onboarding`) is the flow, in order:

1. Connect, and open the page.
2. Resolve the project. Import when needed: GitHub install with the person's
   one browser click, `POST /teams/{t}/projects`.
3. Use a worktree, and propose the session's new working directory.
4. Check. Fix what is yours; bring each "you decide" item and each warning to
   the person with a recommendation.
5. Write the `cloud:` block: `web`, `build`, `release`, `healthcheck`. It
   needs iii ≥ 0.27.0 here.
6. The web conversation. Read `plan.web_candidates`, explain each option in
   one sentence, let the person choose, write `cloud.web` and
   `cloud.healthcheck`, and check again. Every `WEB_*` code is the person's
   answer.
7. Ask for secrets with `iii-cloud::secret`.
8. `check {run: true}`, observing `run` when it outlives the call.
9. Commit, push, and open the PR with `check.report` plus "Changes:".
10. Tell the person to "Merge and deploy" from the page.
11. Observe `deploy`. On a failure, fix in the worktree, check with
    `run: true`, and open a new PR.

`skills/plan-codes.md` (id `iii-cloud/plan-codes`) has:

- every plan issue code from `crates/core/src/error.rs`, with severity,
  kind, owner (the §4 table) and a one-line fix;
- the local codes;
- the registry examples from the design: `path://` versus `package://`, bun
  with a pnpm lock, migrations in CD.

`agents/cloud-ready.md`:

```yaml
---
name: Cloud ready
description: "Takes a project to iii cloud with the iii-cloud worker: sign in, import, readiness, the web part, run here like the cloud, pull request, deploy, and the fix when a deploy fails."
icon: agent
color: blue
extends: default
skills: [iii-cloud/index, iii-cloud/onboarding, iii-cloud/plan-codes]
functions: ["iii-cloud::status", "iii-cloud::connect", "iii-cloud::project::resolve", "iii-cloud::project::use", "iii-cloud::check", "iii-cloud::secret", "iii-cloud::api", "iii-cloud::deploy", "iii-cloud::watch", "harness::observe", "harness::unobserve", "ade::workspace::open", "ade::working-directory::propose", "shell::exec"]
---
```

`functions` preloads contracts; it does not restrict. The body states the
plan-first habit of board 02: list the steps, mark who does each, and call
the person only for the device approval and the GitHub authorization.

### 16. The cloud changes

Two PRs in iii-cloud, from `origin/main`, each labeled `shared` (they touch
`contracts/`, `crates/deploy/CLAUDE.md:6`). They are never merged as part of
this work and never reach production. Titles: `feat(identity): device sign-in
for workers` and `feat(deploy): plan any commit of the repository`.

**C1. Device sign-in for workers.** The session reaches what the person's
role allows, minus account security, members, API keys, billing and team
deletion. The code is always typed.

| File | Change |
|---|---|
| `crates/identity/src/http/device.rs` | `PLATFORMS` (28) gains `worker`, and the message at 146 names it. In `start` (185-194), `verification_uri_complete` is omitted when `platform == "worker"` |
| `crates/identity/migrations/<newer ts>_device_worker_platform.{up,down}.sql` | Drop and recreate the `platform` CHECKs of `sessions` and `device_codes` (from `20261109013310_device_sign_in.up.sql:6,14`) with `worker` added. Confirm the constraint names with `\d` on a migrated database. Add the file to the migration lists in `crates/identity/tests/common/mod.rs` (~503-512) and `crates/access/tests/common/mod.rs` (~253-262) |
| `crates/identity/src/http.rs` | `app_session` (146-160) also returns the session's `platform` |
| `crates/access/src/http.rs` | `app_principal` (273-309): when `platform == "worker"`, allow every route except `WORKER_DENIED` and answer those with 403 FORBIDDEN "A worker session can't <area>; use the console." Other platforms keep the computers-only rule |
| `WORKER_DENIED` | All methods: `/teams/{t}/api-keys*`, `/teams/{t}/invitations*`, `/teams/{t}/team-plan*`, `/teams/{t}/usage*`, `/teams/{t}/prices`, `/teams/{t}/leave`, `/teams/{t}/transfer-ownership`, `/me/two-factor*`, `/me/sessions*`, `/me/deletion`. Non-GET: `/teams/{t}/members*`. `DELETE /teams/{t}` |
| `apps/web/src/device/approval.tsx` | For `platform === "worker"`: title "Connect a device" and board 01's copy ("<name>, from <ip>, wants to act as you on iii. It can do what your role allows: projects, environments, settings, variables, resources and deployments. It can't change your account's security, members, API keys or billing, or delete the team. It lasts 90 days and shows in Account → Devices, where you can sign it out.") |
| `apps/web/src/account/queries.ts:115-121`, `devices.tsx:189,223` | `PLATFORM.worker = "Worker"`, a server icon, and copy that names workers |
| `contracts/openapi/cloud-v1.yaml` | `DevicePlatform` (13193) gains `worker`. `DeviceAuthorization.verification_uri_complete` leaves `required` (13197), described as absent for `worker`. Update `POST /auth/device` (347-354) and the `appSession` scheme (7624-7631): a `worker` session reaches every operation a person's session reaches, except the list above. Regenerate `contracts/openapi/client/schema.d.ts` |

Tests for C1:

- **`crates/identity/tests/device.rs`:**
  - `worker` is accepted, and the start answer has no
    `verification_uri_complete`;
  - the session lasts 90 days, `/me/sessions` lists it with `platform:
    worker`, and revoking it ends it;
  - `wrong_codes_and_bad_requests_are_refused_and_change_nothing` still
    refuses `windows`.
- **`crates/access/tests/app_session.rs`, new
  `a_worker_session_is_the_person_but_not_for_security_members_keys_billing_or_team_deletion`:**
  - as an Owner, it reaches `GET /me`, `GET /teams/{t}/projects`, `POST
    /teams/{t}/projects`, `POST /teams/{t}/github/install`, `PATCH`
    environment, `PUT` variables, `GET` plan and `POST` deployments;
  - it gets 403 on api-keys list and create, `PATCH` member, `POST`
    invitation, team-plan, usage and `DELETE` team;
  - `/me/two-factor` stays refused;
  - the existing `an_app_session_is_the_user_on_the_computers_routes_and_nowhere_else`
    passes unchanged for `ios`.
- **Contracts and web:** the `contracts/openapi/test` suite passes (the
  area-1 count is unchanged, since no operation is added), and
  `apps/web/e2e/device-approval.spec.ts` gains the worker copy case.

**C2. The plan of any commit of the repository.** The deploy lock stays.

| File | Change |
|---|---|
| `crates/deploy/src/api.rs` | Remove the `on_branch` call in `plan` (689-692). Keep it in `launch` (541-552) |
| `crates/deploy/tests/common/api_scenarios.rs` | In the scenario at 1079: the `OFF_BRANCH` plan is now 200 with `source.commit = OFF_BRANCH`, and `POST deployments` with it is still 422 VALIDATION_FAILED. Rename it to `a_commit_off_the_branch_is_planned_but_not_deployed_unless_accepted_before`, and update its registrations in `tests/api.rs:27-30` and `tests/api_pg.rs:49-52`. The scenario at 205 (unknown commits 404) stays |
| `contracts/openapi/cloud-v1.yaml` | `plans.compute` (4628-4635): "a commit of the repository; the branch head when absent". Keep the pattern |
| `crates/deploy/CLAUDE.md:29`, `crates/control-plane/CLAUDE.md:38` | The plan's 404 rule now covers only unknown commits |

Security note for C2's PR:

- `input()` reads the commit through the project's own repository id
  (`crates/control-plane/src/inputs.rs:291-310`,
  `crates/github/src/app.rs:857-875`).
- GitHub may also serve a fork's commit through the parent repository. The
  exposure is a plan, to someone who can already read the environment.
- Deploying it stays refused by `launch`.
- Nothing caches plans by commit (only tokens and registry versions are
  cached).

Commands for both PRs:

- `cargo fmt --all --check`;
- `cargo clippy --locked --workspace --all-targets -- -D warnings`;
- `cargo test --locked --workspace`;
- the database tests: `DATABASE_URL=… .github/scripts/db-tests.sh` against
  our own throwaway Postgres container;
- `.github/scripts/migrate.sh check` and `migrate.sh unchanged origin/main`;
- the openapi lint, generate (the diff is clean), typecheck and test;
- `apps/web` typecheck and the touched Playwright specs.

## Machine rules (every implementer)

- **Worktrees.** Work only in `~/workspaces/worktrees/workers-iii-cloud`
  (branch `feat/iii-cloud-worker`). Never touch `~/workspaces/workers`, the
  main checkout other sessions use, except to read it and to symlink its
  `node_modules` read-only. The iii-cloud changes go in new worktrees of
  their own (tasks.md D). `~/workspaces/iii-cloud` belongs to another
  session: only `git -C ~/workspaces/iii-cloud show|grep|log origin/main …`
  and `git worktree add` outside it. Never checkout, stash or edit there.
- **Build output** never goes in `/tmp` or the scratchpad; `/tmp` is a small
  tmpfs. Set `CARGO_TARGET_DIR=~/tmp/iii-cloud-worker/target` and
  `TMPDIR=~/tmp/iii-cloud-worker/tmp` for every cargo and node command (the
  iii-cloud tasks use `~/tmp/iii-cloud-worker/target-cloud`). Run `df -h ~`
  and `free -g` before a build. Build one crate at a time.
- **pnpm.** Never `pnpm install`, `pnpm exec` or `pnpm run` in a worktree:
  pnpm 11 installs on `exec`/`run` and rewrites main's links. For the UI,
  symlink `iii-cloud/ui/node_modules` →
  `~/workspaces/workers/provider-openai/ui/node_modules`, which has the same
  dependency set, and call `./node_modules/.bin/{tsc,vitest}` and `node
  build.mjs` directly. Remove the link before committing (`node_modules` is
  ignored anyway). Biome: `~/workspaces/workers/node_modules/.pnpm/@biomejs+cli-linux-x64@2.5.4/node_modules/@biomejs/cli-linux-x64/biome`.
- **Cargo and the UI.** `build.rs` runs pnpm when `ui/dist` is missing or
  stale. Always build with `SKIP_UI_BUILD=1`. Before task B lands, task A
  uses ignored stub files: `ui/dist/page.js` = `export default function
  setup() {}`, and `ui/dist/styles.css` = `[data-iii-ui="iii-cloud"] {}`.
- **The running stacks.** Never touch the dev stack in `~/workspaces/dev`
  (engine 49134, ADE 3113), the fabrica stack (49300), or containers you did
  not create. Never `iii call` without `--engine` (it defaults to 49134).
  Stop a stack only with `iii kill --compose <its file>`, never `--port
  49134` or `--orphans`. Never `compose::stop` on the dev engine (it stops
  every project).
- **iii versions.** `~/.local/bin/iii` is 0.26.0 and the dev stack uses it:
  never replace it. The tests use a private 0.27.0 at
  `~/tmp/iii-cloud-worker/bin/iii`, from the `iii/v0.27.0` GitHub release
  for x86_64 Linux, with `iii_bin` set to it.
- **Kubernetes.** Plain `kubectl` defaults to production EKS: never use it.
  Use only the kind stack's guarded wrapper (`k` from
  `tests/e2e/lib/guard.sh`, with `guard_check`), and only in task E.
- **Secrets.** Never print, log or pass as an argument an API key, a session
  token, a cookie or a secret value. Write them to `0600` files and read
  them from there. The Release Control key and cookies in `~/.config/agent-auth/`
  are not for this work.
- **Publishing.** Nothing is pushed to `iii-hq/registry` or deployed to
  production. Commits, pushes and PRs happen only when the owner asks.

## Testing

### Unit and contract (task A, CI)

- **Golden schemas:**
  - `tests/schemas.rs` snapshots each function in `functions::catalog()`
    (id, description, request and response schema) to
    `tests/golden/schemas/iii-cloud.<name>.json`, as `github/tests/schemas.rs`
    does;
  - the catalog test lists the ids of §11 in registration order;
  - regenerate with `UPDATE_GOLDENS=1`.
- **Cloud client, against `wiremock` 0.6** (dev-dependency, as other
  workers use):
  - auth header, Idempotency-Key on writes, If-Match, error envelope →
    `CLOUD_ERROR`;
  - a 401 on a session → `expired`;
  - a key's route refusal → `NEEDS_BROWSER_SIGN_IN`;
  - the device flow: pending, 429 back-off, success, expiry, and the 422 →
    `BROWSER_SIGN_IN_UNAVAILABLE`.
- **Pure rules:**
  - the issue → `Problem` mapping, with `iii_code`, and the owner table
    (golden);
  - the plan digest against a recorded plan: the same plan for another
    commit gives the same digest, and a changed resource does not;
  - the `ready` rule;
  - the run copy rewrite (engine, worker manager, http ports, database
    config) from a fixture compose file;
  - the install table;
  - the healthcheck prefix mapping;
  - remote URL parsing;
  - `.env` editing (in place, append, `0600`, one-line refusal);
  - the `SECRET_VIA_API` refusal;
  - the release-row rule;
  - the no-route outcome.
- **Git:** temp repositories cover `project::use` (worktree created from
  `origin/<branch>`, reused, `WORKTREE_PATH_TAKEN`, bring changes with and
  without conflict, checkout untouched: `git status --porcelain` and `HEAD`
  identical before and after) and the not-pushed detection.
- **Real engine** (`III_ENGINE_BIN`, skip when unset; copy
  `github/tests/support/real_engine.rs`), with `secrets::compare-and-set` and
  `secrets::resolve` faked by the test:
  - connect stores the token, and the token appears in no function output,
    event or `state.json`;
  - `secret::provide` refuses observers (`NOT_OBSERVABLE`);
  - `on-change` reaches an observer with the record.
- **Ignored test** (`--ignored`, needs `~/tmp/iii-cloud-worker/bin/iii`,
  node and npm): the run here on a tiny fixture app (one `path://` node
  worker on `iii-sdk` ≥ 0.26 and `package://http`) passes all seven stages.
  No port 49134 or 3111 is touched: `ss -ltn` before and after.

Commands (from `iii-cloud/`, with the machine rules):

```
SKIP_UI_BUILD=1 cargo fmt --all -- --check
SKIP_UI_BUILD=1 cargo clippy --locked --all-targets --all-features -- -D warnings
SKIP_UI_BUILD=1 cargo test --locked --all-features
python3 ../.github/scripts/validate_worker.py --worker iii-cloud --base-ref origin/main --source-changed '["iii-cloud"]'
```

### UI (task B)

Everything runs from `iii-cloud/ui`, with node_modules symlinked:

- `./node_modules/.bin/tsc --noEmit`;
- `node build.mjs` (the design lint, strict);
- `./node_modules/.bin/vitest run`;
- biome `ci` on `iii-cloud/ui`.

Vitest renders each of the 12 board states from `src/fixtures.ts` with a fake
`host`:

- texts, chips and buttons as in §14;
- the pill rule;
- the phone layout under 640 px;
- the renderers unwrap `details` and fall through on an error envelope;
- `secret::provide` is called with the typed value, and no other call or
  `console.*` carries it.

Visual check: the fixtures are `state.json` files.

1. Start the e2e stack (§E2E, step 1) with `data_dir` pointing at each
   fixture.
2. Open `http://127.0.0.1:3413/#/worker/iii-cloud/iii-cloud` at 1440, 720
   and 390 px, in both themes.
3. Compare with the board using the `design-diff` skill (port 3413 instead
   of 3113).
4. The dialog of board 08 and the chat cards of board 11 are captured by
   clicking (Deploy…) and from a session where the cards were rendered.
5. `ade::ui-manifest` has no `warnings`.

### The cloud PRs (task D)

The commands of §16, in the new iii-cloud worktrees.

### End to end on the kind stack (task E)

The kind stack is our test bench only:

- cluster `iii-cloud-local`;
- console `http://localhost:18080`, API `http://localhost:18080/api/v1`;
- kubeconfig `~/workspaces/iii-cloud/.worktrees/traces-backend/.tools/kube/config`.

The e2e follows the safe reuse of `~/tmp/assistant-real/runbook.md`.
Everything lives in `~/tmp/iii-cloud-worker/e2e/` (not committed: it names
this machine's paths), with its own runbook, scripts and evidence.

**Setup.**

1. **Our iii stack:**
   - `~/tmp/iii-cloud-worker/e2e/stack/worker-compose.yaml`, namespace
     `iiicloud-e2e`, engine `127.0.0.1:49410`, ADE on `3413`;
   - `secrets` and `ade` as `package://` pinned to the versions in
     `~/workspaces/dev/worker-compose.lock` (read only);
   - `iii-cloud` as `path://` to the worktree, with `scripts.run: exec
     ~/tmp/iii-cloud-worker/target/debug/iii-cloud`;
   - config `cloud_url: http://localhost:18080` and `iii_bin:
     ~/tmp/iii-cloud-worker/bin/iii`;
   - started with `env -u III_URL ~/tmp/iii-cloud-worker/bin/iii compose --up
     -f … -n iiicloud-e2e` in a tmux session `iiicloud-e2e`;
   - calls go through `iii call --engine ws://127.0.0.1:49410 -n
     iiicloud-e2e`;
   - check that the dev stack's `iii ps` count on 49134 is the same before
     and after.
2. **Kind, as the runbook does:**
   - copy (never symlink) `.tools` from traces-backend into
     `~/tmp/iii-cloud-worker/e2e/src`, from `git archive origin/main` of
     iii-cloud, then `guard_check`;
   - `k get ns -l iii.cloud/env` must be empty, and the control plane's image
     is the one recorded in `before/`. If either is not true, someone else
     uses the stack: stop and ask;
   - save the control plane deploy and `/repo-rm` to `before/`;
   - create our own database `iii_cloud_worker_e2e`, and point the control
     plane's `DATABASE_URL` at it (same image);
   - run D02-D07 of `tests/e2e/suites/lib/demo.sh` (ada, team, installation
     77, project, environment, plan readable), and keep ada's cookie in
     `secrets/` (`0600`).
3. **The local checkout:**
   - a bare repository `~/tmp/iii-cloud-worker/e2e/origin.git` and a clone
     `apps/hello-iii`;
   - its `remote.origin.url` is `https://github.com/ada/hello-iii.git`,
     with `url.<file:///…/origin.git>.insteadOf` set, so the worker reads the
     cloud's name and git talks to the local bare repository;
   - every content change (the e2e plays the agent) is committed to the bare
     repository's `main`, the worktree is reset to it, and the same files go
     to `/repo-rm` (tar through `k exec`, as runbook step 5). The fake's
     commit id is read from `GET $GITHUB/repositories/9001/commits/main`
     (`recorded_push` in `suites/lib/push.sh`).
4. **The API key:**
   - as ada, `POST /teams/$T/api-keys {name: "iii-cloud e2e",
     environment_id: $E, permissions: ["read", "deploy"]}`, the token
     written by Python straight into the stack's `.env` (`0600`);
   - then `secrets::import {name: "IIICLOUD_E2E", source: "dotenv",
     consumers: ["iii-cloud"]}`. The value never goes through an argument.

**Fixtures** (fake-github serves one repository, `ada/hello-iii`, with one
commit = its files, and no pull requests):

- **F1, a registry copy:** a read-only clone of `iii-hq/registry` at
  `e3d145b`. Nothing is pushed anywhere. The environment's manifest is
  patched as ada to `api/worker-compose-prod.yaml`.
- **F2, the demo app:** `tests/e2e/overlays/P76-control-plane/demo-repo`
  with:
  - a `database` container and `api.environment.DATABASE_URL:
    ${DATABASE_PRIMARY_URL}`, as in
    `tests/e2e/overlays/P54-resources/repo/worker-compose.yaml`, so the plan
    has a Postgres to create;
  - a `cloud:` block: `healthcheck: {path: /hello}`, and `release: {worker:
    api, command: "node -e 'process.exit(process.env.DATABASE_URL ? 0 : 1)'"}`
    (the release sees the variables its worker maps);
  - the lock regenerated with `~/tmp/iii-cloud-worker/bin/iii compose build
    -f worker-compose.yaml`.
- **F3, F2 broken:** `import express from 'express'` added to
  `workers/api/index.mjs` (runbook step 5). The build passes, and `api`
  crash-loops.

**E2E-1: the cloud as deployed today (API-key path).** Each step records the
request, the status, the result and the verdict in `ev/<step>/`.

| # | Step | Pass |
|---|---|---|
| 1 | `connect {method: "browser"}` | `BROWSER_SIGN_IN_UNAVAILABLE`; `status.connection.status` stays `signed_out` |
| 2 | `connect {method: "api_key", secret: "IIICLOUD_E2E", team_id: $T, environment_id: $E}` | `connected`; the token is not in `state.json`, the stack's logs, `engine::traces::*` or any result (`grep -cFf` = 0) |
| 3 | `project::resolve {path: apps/hello-iii}` | `repository: "ada/hello-iii"`, `environment.id: $E`, `cloud_project: null` (key) |
| 4 | Change one file in the checkout, then `project::use {mode: "worktree", bring_changes: true}` | the worktree is `apps/hello-iii-iii-cloud` on `iii-cloud/ready-<env>`, from `origin/main`, with the change; the checkout's `git status` and `HEAD` are unchanged |
| 5 | F1 loaded; `check {commit: <fake sha>}` | plan `blocked`, problems include `LOCK_MISSING` and `MISSING_WORKER_DIRECTORY` (`cloud_code: INVALID_COMPOSE_FILE`, owner `person`); here, `SDK_TOO_OLD`, `RUNTIME_LOCK_MISSING`, `RELEASE_IN_CI`, `HEALTHCHECK_MISSING`; `ready: false` |
| 6 | `check {}` on an unpushed worktree commit | `plan.status: "not_pushed"`; the checks here still run |
| 7 | `secret {name: "SESSION_SECRET", …}`, then `secret::provide {value}` read from a `0600` file by a Python bus client | `waiting`, then `set` with `cloud: "skipped"` (a key can't PUT) and `local: "set"`; the worktree `.env` has the line, mode `0600`; the value is in no log, trace, event or `state.json` |
| 8 | F2 loaded; `check {commit: <fake sha>, run: true}` | plan `clean`; run `passed` with all seven stages (Postgres started and removed, `release` saw the binding, healthcheck 200); `ready: true`; nothing listens on the run's ports afterwards, and `docker ps` has no `iii-cloud-run-*` |
| 9 | The merge, played by the e2e: `deliver` of `tests/e2e/suites/lib/push.sh` for the fake's head (fake-github has no PRs), then `deploy {check_id}` without `pr` | the push's deployment waits `needs_action` (a Postgres to create); `deploy` creates one with `confirm_resources: true` and `If-Match`; the waiting one ends `replaced` (this is the first live proof of D6: record what the cloud did either way); `watch` follows the new one to `ready`; exactly one `deploy` event with `terminal: true` for it |
| 10 | F3 loaded, pushed, `watch {}` | outcome `failed`, `error.stage` workers, `log_tail` has `Cannot find package 'express'`; one terminal event |
| 11 | UI: captures of boards 01-10 and 12 from the live states above, at the three widths | as §14; `ade::ui-manifest` clean |
| 12 | Last: ada revokes the key (`POST /teams/$T/api-keys/{id}/revoke`), then `check {}` | `KEY_REJECTED`; `connection.status` becomes `expired`; one `connection` event |

**E2E-2: with C1 and C2.**

- **Image:** a local branch `e2e/iii-cloud-worker` = iii-cloud `origin/main`
  plus both PR branches, built as `localhost:15000/iii-cloud/control-plane:iii-cloud-worker`
  (our own tag, never `:dev`), with its digest recorded.
- **Deploy:** applied as runbook step 4, with our database.
- **Steps:**
  - repeat 1 → `waiting`. As ada, `POST /api/v1/auth/device/lookup` and
    `/approve` with the `user_code` (the suite plays the browser) →
    `connected`, `teams` listed;
  - `project::resolve` finds the cloud project by repository;
  - `secret::provide` gives `cloud: "set"` (`GET variables` shows it `set`,
    value null);
  - as an Owner, `api GET /teams/$T/api-keys` → 403;
  - `api POST /teams/$T/projects` imports a second environment;
  - the device appears in `GET /me/sessions` (ada's cookie) with platform
    `worker`, and ada's revoke turns the next call into `expired`.
- **Not covered:** the plan of an off-branch commit cannot be shown on kind,
  because fake-github knows one commit only. C2's scenario tests cover it.

**What the e2e cannot cover on kind,** and where it is covered:

- **Pull request merge with `gh`:** fake-github has no PRs. The merge path
  is covered by unit tests with a fake `gh` on `PATH` (a script answering
  `pr view` and `pr merge`), and the push step stands in for the merge.
- **The device approval page in a browser:** the apps/web Playwright spec
  of C1.
- **Real GitHub authorization:** the later proof in a real iii cloud test
  project. That is a production deploy, so it needs the owner's OK (design
  "Decisões que faltam").
- **Production limits (the registry's OOM of board 10):** the same later
  proof. On kind, F3 stands in.

**Restore, always, in this order:**

1. Run the leak check.
2. `DELETE` the environments, and wait until `k get ns -l
   iii.cloud/env=<id>` is empty.
3. Restore `/repo-rm` from `before/`.
4. `rollout undo` the control plane to the recorded revision, and diff it
   against `before/`.
5. Drop `iii_cloud_worker_e2e`.
6. Stop our stack with `~/tmp/iii-cloud-worker/bin/iii kill --compose
   <our compose file>` and kill the tmux session.
7. Remove the worktrees and the bare repository.
8. `rm -rf secrets/ tmp/`.

Leave the cluster running.

## Open decisions

- **D3, the API key's ids.** A key cannot tell its team and environment. The
  page asks for them, and the console does not show them. Recommend a
  read-only `GET /api-keys/self` → `{team_id, environment_id, permissions}`
  in the cloud later. It is not in this work: the owner approved two cloud
  changes.
- **`cloud.build` spelling.** The guide writes `build: {command}`
  (`docs/guides/compose-file.md` ~52) and the settings say `build_command`
  (yaml 10831). The worker reads both. The cloud should settle one.
- **C1's denylist** follows the owner's list. `teams.update` and
  `github.install` stay allowed, because the import needs the install.
- **The final proof** on the real iii cloud (design, phase 1) needs the
  owner's OK, and C1 and C2 deployed.
- **The worker in the harness template** (design: recommended after phase
  1).

## Amendments after review (25/11/2026)

An independent review (cloud refs at iii-cloud origin/main d31ed945) found defects. These override the sections above.

1. **Deploy confirm (§8 step 5).** Decide from the target plan. With creates or overrides: wait until the push deployment is `needs_action`, then create the confirmed deployment (it replaces it). With neither: never create, only adopt the push deployment. No "not seen in 120 s → create" branch.
2. **Recovery after merge (§8).** Save `Deploy {pr, target}` right after the merge; `deploy` with a merged PR resumes from step 4. On PLAN_CHANGED, cancel the push deployment while it is queued or in progress.
3. **`api` passthrough (§11).** Refuse `…/resources/{r}/temporary-credential`; the worker session (C1) denies it too.
4. **Ready pins what is merged (§4, §7, §8).** `ready` needs `base` to be an ancestor of `commit` and a clean worktree (the run's own files excluded); merge with `gh pr merge --match-head-commit <commit>`; after the merge, the merge commit's first parent must be `base`.
5. **No route (§9, §6).** Read deployment events, match "but no function of this release serves a route"; add `GET …/health` to the evidence. The run here skips NO_ROUTE when the plan has `settings.web` or `settings.site`.
6. **Secrets in runs (§5, §6, §10).** Redact `.env` and binding values from log tails, stage details and errors; never write a tracked `.env` (`local: "skipped"`).
7. **Wake-ups (§9, §10).** `replaced` is not terminal; `needs_action` wakes the agent; `deploy {check_id, deployment_id}` confirms a waiting deployment whose plan digest matches (merge done on GitHub). The skill filters on the id of the deploy it started.
8. **Crash recovery (§6).** On start: fail `running` runs, remove their containers, `iii kill --compose` a leftover run copy, delete it.
9. **Excludes (§3).** `.iii/` without a leading slash; remove it on teardown.
10. **Worktree reuse (§3).** Reset a reused clean branch whose commits were merged; otherwise use a new branch name.
11. **Ports (§6).** Move every engine worker port and any `package://ade` port, or fail before start; never 3111/3112/3113/49134.
12. **Idempotency.** `IDEMPOTENCY_KEY_REUSED` has its own code; a retry after a failure is a new key.
13. **Third cloud change, C3 `GET /api/v1/keys/self`** (owner-side decision): API-key auth only, same checks as `key_principal`, returns the calling key's `{team_id, environment_id, project_id, permissions}`; browser and app sessions refused. Board 01's API-key card needs only the secret name.
