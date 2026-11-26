---
name: iii-cloud/plan-codes
description: >-
  Every problem code of `iii-cloud::check`: severity, kind, who fixes it, and
  the fix in one line. Plus the three registry decisions.
---

# Problem codes

`iii-cloud::check` returns problems from two sources. A `cloud_plan` problem
comes from the cloud's plan for the commit. A `check_here` problem comes from
the fast checks on this machine.

- `severity`: `block` stops a deploy. `warn` and `info` do not. Bring each
  warning to the person anyway, with a recommendation.
- `kind`: `fix_in_repo` is a change in the files. `resolve_here` is a choice.
- `owner`: `agent` means you fix it. `person` means you explain it in one
  sentence, recommend, and let them decide. `none` means nothing to do.
- A problem's `code` is the cloud's `context.iii_code` when there is one, else
  the cloud's `code`. The plan issue's own `message` and `fix` are the detail.
- The cloud plan stops at its first blocking step. Check again after each round.

The owner column follows `kind`, except these, which always go to the person:
`RUNTIME_UNSUPPORTED`, `LITERAL_CREDENTIAL`,
`SECRET_MISSING`, `MULTIPLE_HTTP_GROUPS`, every `WEB_*` code,
`LOCAL_CONFIG_NOT_SET`, `ENV_FILE_MISSING` and `EMPTY_DEFAULT_VARIABLE`.
A warning with no kind goes to the person too. Except: `SECRET_MISSING`,
`EMPTY_DEFAULT_VARIABLE` and `LOCAL_VARIABLE_UNSET` about a database URL
(`DATABASE_URL`, `POSTGRES_URL`, ...) are yours: point the name at the managed
database's binding (`${DATABASE_<ALIAS>_URL}`) in the compose file; iii cloud
makes the database.

## Cloud plan codes

`MISSING_WORKER_DIRECTORY` is not a code of its own. The cloud reports it as
`INVALID_COMPOSE_FILE` with `context.iii_code`, and `iii-cloud::check` shows it
as `MISSING_WORKER_DIRECTORY`. Its owner is `agent`: the agent finds the real worker
directory and fixes the path, and asks the person only when no such directory exists.

| Code | Severity | Kind | Owner | Fix |
|---|---|---|---|---|
| `MANIFEST_NOT_FOUND` | block | fix_in_repo | agent | Edit: the checked compose file (the environment's, or the one the check named) is not at the commit, or is outside the root directory. Create the file there, or ask the person to pick another file or fix the environment on the page (only the person writes settings). Verify: `check` again. |
| `PLURAL_FILENAME` | block | fix_in_repo | agent | Rename `worker-composes.yaml` and its lock to the singular `worker-compose.yaml` and `worker-compose.lock` with `git mv`. Verify: `check` again. |
| `PATH_OUTSIDE_BUNDLE` | block | fix_in_repo | agent | Edit: a `path://` leaves the repository. Move that folder inside the repository, or use a `package://` worker. Verify: the path stays under the repo root. |
| `INVALID_COMPOSE_FILE` | block | fix_in_repo | agent | Edit the compose file at the issue's file and line, as its `message` says. `MISSING_WORKER_DIRECTORY` is one case. Verify: `iii compose build -f <manifest>`, then `check`. |
| `INVALID_CLOUD_BLOCK` | block | fix_in_repo | agent | Edit the `cloud:` block at the issue's line: keep only `web`, `build`, `release`, `healthcheck` and `runtimes`, with the types the message names. Verify: `check` again. |
| `COMPOSE_LOCK_OUT_OF_DATE` | block | fix_in_repo | agent | Regenerate the lock: run `iii compose build -f <manifest>` with iii 0.27 in the worktree, and commit the `.lock`. Verify: the same command with `--frozen` succeeds. |
| `LOCK_MISSING` | block | fix_in_repo | agent | Create the lock: run `iii compose build -f <manifest>` with iii 0.27 in the worktree, and commit `<manifest stem>.lock` next to the manifest. Verify: the same command with `--frozen` succeeds. |
| `ENGINE_SECTION_UNSUPPORTED` | block | fix_in_repo | agent | Edit the compose file: remove the `engine:` setting the message names (the cloud runs its own engine). Verify: `check` again. |
| `MISSING_DEPENDENCY` | block | fix_in_repo | agent | Edit the compose file: add the worker the message names as a dependency (a `package://` entry), then regenerate the lock with `iii compose build -f <manifest>`. |
| `UNSUPPORTED_ARCH` | block | fix_in_repo | agent | Edit: pin the worker to a version published for linux x86_64 and regenerate the lock with `iii compose build -f <manifest>`. Verify: `check` again. |
| `UNSUPPORTED_PACKAGE_KIND` | block | fix_in_repo | agent | Edit the compose file: the entry uses a package kind the cloud cannot run. Use `path://` or a registry `package://` worker. Verify: `check` again. |
| `RUNTIME_UNSUPPORTED` | block | fix_in_repo | person | Person decides: the worker's runtime is not one the cloud builds (see `runtimes` in `GET /support-matrix`). Offer to port it or run it elsewhere. Verify: `check` again. |
| `UNGROUPED_WORKER` | block | resolve_here | person | Person decides which group the worker joins. Then edit `cloud.groups` (or the group settings) so the worker is in one. Verify: `check` again. |
| `UNKNOWN_GROUP_WORKER` | block | resolve_here | person | Person decides: a group names a worker the file does not have. Fix the name at the issue's line, or remove the group. Verify: `check` again. |
| `CRON_GROUP_SCALED` | block | resolve_here | person | Person decides: a group with a cron worker must run once. Set that group's scale to 1 in the environment settings. Verify: `check` again. |
| `MULTIPLE_HTTP_GROUPS` | block | resolve_here | person | Person decides which group serves http; the cloud allows one. Move the other http worker to the same group or remove it. Verify: `check` again. |
| `INVALID_BRANCH` | block | resolve_here | person | Person decides: the environment's branch name is not valid. The person sets an existing branch in the environment's settings on the page; you never write settings. Verify: `check` again. |
| `INVALID_RELEASE_WORKER` | block | resolve_here | person | Person decides: `cloud.release.worker` names no worker in the file. Set it to a worker that exists. Verify: `check` again. |
| `WEB_CONTAINER_UNKNOWN` | block | resolve_here | person | Person decides: `cloud.web.container` names no container in the file. Set it to a real container name. Verify: `check` again. |
| `WEB_CONTAINER_NOT_SERVABLE` | block | resolve_here | person | Person decides: that container has no http port to serve. The Web setting may be stored in the environment (an earlier manifest, the console) and not in the compose file: change Web in the environment's settings, or set it to none (the `http` package already serves your routes). Verify: `check` again. |
| `WEB_CONTAINER_OUTSIDE_HTTP_GROUP` | block | resolve_here | person | Person decides: put the web container in the http group in the group settings. Verify: `check` again. |
| `WEB_PORT_RESERVED` | block | resolve_here | person | Person decides: the web port is one the cloud reserves. Choose another port in `cloud.web`. Verify: `check` again. |
| `WEB_CONSOLE_PUBLIC` | block | resolve_here | person | Person decides: the console must not be public. Set `cloud.web` access to team only. Verify: `check` again. |
| `WEB_API_WITHOUT_HTTP` | warn | none | person | Person decides: `cloud.web.api.prefix` has no http worker behind it. Add the http worker or remove the prefix. Verify: `check` again. |
| `WEB_DIRECTORY_UNKNOWN` | block | resolve_here | person | Person decides which folder holds the site; the cloud needs its `package.json` to build it. Set `cloud.web.directory` to that folder. Verify: `check` again. |
| `UNKNOWN_SCALE_QUEUE` | block | resolve_here | person | Person decides: a scale rule names a queue the file does not have. Fix the queue name in the scale rule. Verify: `check` again. |
| `INVALID_RESOURCE_REQUEST` | block | resolve_here | person | Person decides: fix the resource's kind, alias or mode at the issue's line (`database`, `storage`, `queue`, `state`). Verify: `check` again. |
| `SQLITE_UNSUPPORTED` | block | fix_in_repo | agent | Edit the worker's config: replace the SQLite database with a Postgres `database` resource, and read the URL from `DATABASE_URL`. Verify: `check` again. |
| `STORAGE_PROVIDER_UNSUPPORTED` | block | fix_in_repo | agent | Edit the storage worker's config: use a provider the cloud supports (the message names them). Verify: `check` again. |
| `STORAGE_NOTIFICATIONS_UNSUPPORTED` | block | fix_in_repo | agent | Edit the storage worker's config: remove the notifications setting. Verify: `check` again. |
| `QUEUE_ADAPTER_UNSUPPORTED` | block | fix_in_repo | agent | Edit the queue worker's config: use an adapter the cloud supports (the message names them). Verify: `check` again. |
| `QUEUE_ADAPTER_MISSING` | block | fix_in_repo | agent | Edit the queue worker's config: declare the adapter explicitly. Verify: `check` again. |
| `QUEUE_TOPOLOGY_IMMUTABLE` | block | fix_in_repo | agent | Edit: restore the queue names and settings the environment already has. A new topology needs a new environment: ask the person. Verify: `check` again. |
| `STATE_ADAPTER_UNSUPPORTED` | block | fix_in_repo | agent | Edit the state worker's config: use an adapter the cloud supports (the message names them). Verify: `check` again. |
| `LITERAL_CREDENTIAL` | block | fix_in_repo | person | Edit the file at the issue's line: replace the value with `${NAME}`, and ask for it with `iii-cloud::secret`. Tell the person to rotate the value if it was real. Verify: `check` again. |
| `SECRET_MISSING` | block | resolve_here | person | Call `iii-cloud::values` to see them; tell the person to open Values (Setup) or 'Set values' in the check. Add the why with `iii-cloud::secret {name, description}`. Verify: `status` shows the secret as `set`. |
| `LIMIT_EXCEEDED` | block | resolve_here | person | Person decides: a plan limit is reached. Reduce what the file asks for, or the person changes the plan in the console. Verify: `check` again. |
| `LOCAL_CONFIG_NOT_SET` | warn | resolve_here | person | Tell the person: a worker setting has no value in the cloud. Set it in the compose file's `config`, or in the console. Verify: `check` again. |
| `MCP_GLOBAL_MIDDLEWARE` | block | fix_in_repo | agent | Edit the `http` worker's config: remove its global middleware, which would read every MCP client's key. Use route-level middleware. Verify: `check` again. |
| `ENV_FILE_MISSING` | warn | resolve_here | person | Tell the person: the file references an `.env` the repository does not have. Move the values to `variables` and `iii-cloud::secret`. Verify: `check` again. |
| `ENV_FILE_SHADOWED` | warn | none | person | Note only: the environment's variables replace the `.env` values. Tell the person. No fix needed. |
| `PRE_RUN_REPLICATED` | warn | none | person | Note only: a pre-run step runs on every replica. Tell the person; move it to `cloud.release` if it must run once. |
| `CUSTOM_REGISTRY_PACKAGE` | warn | none | person | Note only: the package comes from a custom registry. Tell the person. No fix needed. |
| `SQLITE_DEFAULT_REPLACED` | warn | none | person | Note only: the cloud replaces the SQLite default with Postgres. Tell the person. No fix needed. |
| `STORAGE_DEFAULT_REPLACED` | warn | none | person | Note only: the cloud replaces the local storage default. Tell the person. No fix needed. |
| `QUEUE_SEMANTICS_CHANGED` | warn | none | person | Note only: queue behavior differs in the cloud. Tell the person what the message says. No fix needed. |
| `QUEUE_WORKER_OUTDATED` | warn | fix_in_repo | agent | Edit the compose file: raise the queue worker to the version the message names, then run `iii compose build -f <manifest>` to refresh the lock. |
| `WORKER_OUTDATED` | block | fix_in_repo | agent | Edit the compose file: raise the worker to the version the message names, then run `iii compose build -f <manifest>` to refresh the lock. |
| `WORKER_VERSION_REFUSED` | block | fix_in_repo | agent | Edit the compose file: pick another version of the worker, then run `iii compose build -f <manifest>`. Verify: `check` again. |
| `EMPTY_DEFAULT_VARIABLE` | warn | none | person | `${NAME:-}` defaults to empty. Call `iii-cloud::values` to see them; tell the person to open Values (Setup) or 'Set values' in the check. Add the why with `iii-cloud::secret {name, description}`. "Leave empty" is a real choice there. |
| `COMMAND_VARIABLE_EMPTY` | warn | none | person | A `${NAME}` in a `cloud:` command is empty when it runs. Set the variable with `iii-cloud::secret`, or remove it from the command. |
| `ENGINE_SECTION_IGNORED` | info | none | none | Info only: the cloud ignores the `engine:` section. No fix needed. |
| `NAMESPACE_IGNORED` | info | none | none | Info only: the cloud ignores the namespace. No fix needed. |
| `SANDBOX_ADAPTER_IGNORED` | info | none | none | Info only: the cloud ignores the sandbox adapter. No fix needed. |
| `BUILD_FAILED` | block | fix_in_repo | agent | Read the deploy's build log, fix `cloud.build.command`, and run it here: `check {run: true}`. Verify: the build stage passes. |
| `RELEASE_FAILED` | block | fix_in_repo | agent | Read the release log, fix `cloud.release.command` or the migration, and run it here against local Postgres: `check {run: true}`. Verify: the release stage passes. |
| `ROUTE_OUTSIDE_HTTP_GROUP` | block | resolve_here | person | Person decides: a route is registered by a worker outside the http group. Move that worker into the group, or stop it serving routes. The cloud only routes through `package://http`: add it (pinned version) to the manifest if the app serves routes. Here the run fails this way when routes exist and no `package://http` container does. |
| `HEALTHCHECK_FAILED` | block | fix_in_repo | agent | Fix the path in `cloud.healthcheck`, or the app so the path answers 200. Run it here: `check {run: true}`. Verify: the healthcheck stage passes. |
| `MISSING_WORKER_DIRECTORY` | block | fix_in_repo | agent | A `path://` is relative to the compose file's directory: point it at the real worker directory, version that directory in the repository, or use `package://`. Verify: the folder exists, then `check`. |

## Local codes

These come from the checks on this machine. All block, except where noted. Ready needs none of them, so `BASE_NOT_IN_HISTORY` and `UNCOMMITTED_CHANGES` stop a deploy.

| Code | Owner | Fix |
|---|---|---|
| `LOCAL_ENGINE_VERSION` | person | Install the iii version the cloud runs (`GET /support-matrix`, field `engine`) and set `iii_bin` to it. Verify: `<iii_bin> --version` prints it. iii 0.26 also refuses the `cloud:` block. |
| `SDK_TOO_OLD` | agent | Raise `iii-sdk` to the cloud's `minimum_sdk` or newer in the worker's manifest, refresh its lockfile, and commit both. Verify: `check` again, then `check {run: true}`. |
| `RUNTIME_LOCK_MISSING` | person | Person decides: add the lockfile for the worker's runtime at the root directory (`pnpm-lock.yaml`, `bun.lock`, `package-lock.json`, `uv.lock`, `Cargo.lock`), or change the runtime. Verify: `check` again. |
| `RELEASE_IN_CI` | agent | Edit the compose file: add `cloud.release` with the migration command the workflow runs, naming the worker and a timeout. Verify: `check {run: true}` passes the release stage. Remove the command from the workflow only if the person agrees. |
| `HEALTHCHECK_MISSING` | agent | Edit the compose file: add `cloud.healthcheck` with a path that answers 200 (for example `/health`) and a timeout. Verify: `check {run: true}` passes the healthcheck stage. |
| `LOCAL_VARIABLE_UNSET` | person | Not in the run's `.env` nor in the folder's `.env.<env>`, `.env.local`, `.env` or `.env.production` (the run here takes it from those). Call `iii-cloud::values` to see them; tell the person to open Values (Setup) or 'Set values' in the check. Add the why with `iii-cloud::secret {name, description}`. Or give it a default in the compose file (`${NAME:-default}`). Verify: `check` again. |
| `VARIABLE_SYNTAX_UNSUPPORTED` | agent | iii cloud reads only `${NAME}`, `${NAME:-default}` and, in `config_override`, `${NAME:default}`; `${NAME:?}`, `${NAME?}` and `${NAME-x}` reach the container as literal text. Edit the compose file: write `${NAME}` for a required value or `${NAME:-default}`. Verify: `check` again. |
| `DATABASE_NOT_MANAGED` | agent | Add the `database` worker (package://database, `version: latest`) to the manifest and set `DATABASE_URL: ${DATABASE_PRIMARY_URL}` on the container that reads it. iii cloud creates a managed Postgres (pgvector included) when the deploy is confirmed; the run here starts a local one. Never ask the person for a database URL. |
| `BASE_NOT_IN_HISTORY` | agent | Run `git fetch origin` and rebase on `origin/<branch>` (or merge it) in the worktree, then run the checks again. Verify: `git merge-base --is-ancestor origin/<branch> HEAD`. |
| `UNCOMMITTED_CHANGES` | agent | Commit the worktree's changes with `git add -A && git commit` (the run's own files do not count), then check and run again. Verify: `git status --porcelain` is empty. |
| `OLD_ATTEMPT_COMMITS` | agent | The worktree branch holds commits from an earlier try that are not on the base. Reset to `handoff_brief.base` (`git reset --hard <base sha>`) and redo the fixes. Verify: `check` again. |
| `SIGN_IN_REQUIRED` | person | Not signed in to iii cloud, so only the checks on this machine ran. Sign in (`iii-cloud::connect`), then check again. |
| `HTTP_WORKER_NOT_PACKAGE` | agent | A container named `http` (or whose `iii.worker.yaml` says `http`) is not `package://http`; the cloud treats it as plain code and routes nothing to it. Edit the compose file: use `package://http` with a pinned version, move the route code to a worker that registers on it. Verify: `check {run: true}` passes the routes stage. |
| `PROJECT_DELETED` | person | The selected project was removed in iii cloud (`status.problems`). Tell the person; they choose another with `iii-cloud::project::use`. |
| `ENVIRONMENT_DELETED` | person | The selected environment was removed in iii cloud (`status.problems`). The person picks another environment. |
| `REPOSITORY_MISMATCH` | person | `project::use` refused a project that deploys another repository. Open that repository's folder, or pick a project of this one (`project.cloud_projects`). |
| `LOCK_MISSING` | agent | Create the lock: run `iii compose build -f <manifest>` with iii 0.27 in the worktree, and commit `<manifest stem>.lock` next to the manifest. Verify: the same command with `--frozen` succeeds. |

## Worker errors

Besides problems, a call can fail with a code. New ones:

- `DATABASE_FROM_CLOUD`: `iii-cloud::secret` refused a database URL. iii cloud creates the database: follow `DATABASE_NOT_MANAGED`. Pass `external: true` only when the person chose a database outside iii cloud.
- `NOT_SAVED`: a secret went nowhere. Tell the person to set it in the console.
- `RUN_PANIC`: the run crashed inside the worker. Run again.
- `PR_HEAD_CHANGED`: the pull request's head moved after the check. Check again.
- `WORKTREE_FAILED`: `git worktree add` failed. Read the message.
- `IDEMPOTENCY_KEY_REUSED`: the key was used for another request. Use a new key.
- `WORKER_RESTARTED`: the worker restarted during a run. Run again.
- `DEVICE_CODE_EXPIRED`: the sign-in code timed out. Connect again.
- `ACCOUNT_BLOCKED`: the account is blocked. Tell the person.

## Registry decisions

These three came up in a real repository. Each needs the person's answer.
Give the recommendation, then let them choose.

**`path://` or `package://` for a folder the Dockerfile downloads.**
`MISSING_WORKER_DIRECTORY` means the repository does not have the folder.
Recommend `path://`: version the folders and download each binary by sha256 in
the build, as the Dockerfile does. With `package://`, publishing a fix would
depend on the registry being up.

**A bun project with only a pnpm lock.** `RUNTIME_LOCK_MISSING`. The cloud
installs bun projects from `bun.lock`, so nothing would be installed and
`dist/` would not exist. Recommend switching the runtime to the one the lock
belongs to. Add `bun.lock` only when the person wants bun.

**Migrations run in the CD.** `RELEASE_IN_CI`. A workflow runs `db:migrate`
before each deploy, and the cloud's deploy does not. Without `cloud.release`,
new code meets the old schema. Recommend `cloud.release` with the same
command, and run it here against the local Postgres.
