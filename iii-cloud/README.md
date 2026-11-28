# iii-cloud

Take any project to iii cloud from your harness. The worker signs in to iii
cloud, shows the cloud's plan for a commit, runs your app on this machine the
way the cloud runs it, and keeps secret values out of your agent's context.
When you are ready, it merges the pull request (production) or, for any other
environment, sends the checked commit to the environment's own branch (an
assisted deploy), and deploys, pinned to the plan you saw, and follows the deploy to Ready. Your agent knows the path through the
`cloud-ready` profile and the `iii-cloud` skill. The worker adds the
`iii cloud` page to the console.

## Install

Add the worker to your harness, not to the project you are taking to the cloud.

```bash
iii call compose::add worker=iii-cloud
```

`iii call compose::add` resolves the worker and its dependencies, writes exact
declarations to `worker-compose.yaml`, and reconciles the Compose project.

The worker needs these on the machine:

- `git`.
- The [GitHub CLI](https://cli.github.com), logged in with `gh auth login`.
  The worker uses it to read and merge your pull request with your own login.
- Docker, only when your app has a database resource. The run here starts a
  local Postgres in it. Without a database resource, no Docker is needed.
- The same iii version as the cloud's engine (0.27.0 today). The run here
  uses it, and 0.26 refuses the `cloud:` block. `iii --version` shows yours.

## Quickstart

Ask your agent: "Put this project on iii cloud." Pick the `Cloud ready` profile
if you want it from the start. It plans the steps, does them, and calls you for
two things: approving a device code and authorizing GitHub.

To drive it yourself, use the functions in order:

```bash
# 1. Sign in. Type the code it returns at <cloud_url>/device.
iii call iii-cloud::connect method=browser

# 2. Find the project and its environment from the folder's git remote.
iii call iii-cloud::project::resolve path=/path/to/your/project

# 3. Select the environment and create a worktree to work in.
iii call iii-cloud::project::use path=/path/to/your/project \
  environment_id=<environment id> mode=worktree

# 4. Check: the cloud plan plus the fast checks here. Add run=true to run the app.
iii call iii-cloud::check run=true
```

The answer of `check` lists each problem with its file, line and who fixes it,
and a `ready` flag. When it is ready, open a pull request (production) and press "Merge and
deploy" on the `iii cloud` page. Any other environment gets an assisted deploy:
press "Deploy <commit> to <env>".

`manifest=<path>` checks another `worker-compose*.yaml` of the repository
instead of the environment's; iii cloud changes nothing. Deploying a ready
check of that file is what switches the environment to it: the deploy sets
the environment's compose file first, and puts the old one back if the deploy
fails before a deployment exists.

To sign in with an API key instead, save the key in the secrets vault from the
page, then pass only its name:

```bash
iii call iii-cloud::connect method=api_key secret=<secret name>
```

The worker finds the key's team and environment through the cloud. Older
clouds cannot do that: pass `team_id` and `environment_id` too. A key can check
and deploy one environment. It cannot import a project or change settings.

## Configuration

The entry is `iii-cloud`. Changes apply without a restart.

```yaml
cloud_url: https://cloud.iii.dev   # the console; calls go to its functions at <cloud_url>/_iii/ws (<cloud_url>/api/v1 while that socket is down). A change signs you out
iii_bin: iii                       # the iii the run here uses; it must match the cloud's engine
merge_method: squash               # merge, squash or rebase: the gh pr merge flag
postgres_image: pgvector/pgvector:pg17        # the local Postgres for a database resource
run_timeout_s: 900                 # the whole run here, from install to healthcheck
data_dir: data/iii-cloud           # records and run logs, relative to the compose directory
auto_fix: true                     # a check the person starts hands the agent the problems that are its to fix
```

## Event functions

| Function | Args | Called when | Event (what observers receive) |
|---|---|---|---|
| `iii-cloud::on-change` | none; filter on `input.kind`, `input.id`, `input.terminal` | After every change to a record | `{ "kind": "connection" \| "project" \| "check" \| "run" \| "secret" \| "deploy", "id", "terminal", "summary", "record" }` |

`terminal` is true when a run or deploy finished, a secret was answered, or a
sign-in settled. Wake an agent only on the end of something:

```bash
iii call harness::observe function_id=iii-cloud::on-change \
  filters='{"input.kind":{"$eq":"deploy"},"input.terminal":{"$eq":true}}'
```

The page observes the same function to stay live. Filter a deploy on its
`input.id`. A deploy replaced by the worker's own confirmation is not the end, one replaced by someone else's deployment ends with `replaced_by`, and `needs_action` means the
cloud waits for a confirmation.

A failed call returns a code, such as `WORKTREE_FAILED`, `IDEMPOTENCY_KEY_REUSED`,
`WORKER_RESTARTED`, `DEVICE_CODE_EXPIRED` or `ACCOUNT_BLOCKED`. The message
says what to do. When `secret::provide` answers `local: "skipped"`, your `.env`
is tracked by git and the worker did not write it.

## What the run here does not cover

The run here repeats the cloud's steps in the cloud's order: the frozen install,
the build command, the release command, the compose file, every worker, the
routes and the healthcheck. It does not cover:

- The cloud's build container.
- Production's secret values.
- Its machine size.
- Real traffic.
- Queue, state and storage resources. They run with local defaults here.

The run here moves every engine, http and ade port to a free one. It never
uses 3111, 3112, 3113 or 49134, so your harness keeps running.

For these, the worker follows the deploy after the merge. When it fails, the
evidence goes back to your agent.

Secret values never enter your agent's context. You type them on the page. They
go to iii cloud and to the project's local `.env`, and the agent only sees that
they were saved.

The skill and the profile ship in this worker: see the
[skill](https://github.com/iii-hq/workers/blob/main/iii-cloud/skills/SKILL.md)
and the
[`cloud-ready` profile](https://github.com/iii-hq/workers/blob/main/iii-cloud/agents/cloud-ready.md).
