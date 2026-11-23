# tower

The tower worker owns the control tower page, read model and summary schedule.
Open the page from the console navigation or through the `#/tower` workspace
redirect. It aggregates harness roots and asks, kanban projects, cards, releases
and questions, and pull requests from repositories declared on projects. All
public tower reads are read-only; approval holds remain a console concern.

## Configuration

The worker registers configuration id `tower` with these keys:

| Key | Default |
| --- | --- |
| `stall_after_min` | `30` |
| `summary.model` | unset |
| `summary.interval_min` | `10` |
| `summary.language` | `English` |
| `prs.watched_interval_min` | `5` |
| `prs.idle_interval_min` | `30` |
| `escalate` | `{}` |
| `at_risk_days` | `3` |

An unset project escalation list means `security`, `publish`, `external_cost`
and `data`. Without `summary.model`, the writer is disabled. The model uses
`provider::model` syntax.

## Functions and event

- `tower::overview {scope}` returns the CEO view, latest valid summary and
  `prs_interval_min` (the watched PR interval used by the page freshness label).
- `tower::needs-you::list {scope}` returns questions and stalled roots.
- `tower::runs::list {scope}` returns scoped roots and context sizes.
- `tower::prs::list {scope}` returns cached open pull requests.
- `tower::decided::list {scope, cursor?}` pages settled questions.
- `tower::assumptions::list {scope, since?}` filters recorded assumptions by comment time.
- `tower::watch {scope}` watches the scope for two minutes.
- `tower::summary::refresh {scope}` requests a rate-limited writer turn.
- `tower::on-changed {scope, as_of, parts}` announces changed read-model parts.

`scope` is `all` or a project id. GitHub and approval-gate are optional. When
GitHub is unavailable, pull requests report unavailable; without approval-gate,
writer sessions run without approval setup.

## Source boundaries

Root metadata comes from `session::list`; context size comes from verbose
`harness::status`, with a cached messages-tail estimate when unavailable.
The read model does not call `session::get` or `harness::metrics`: revision 5's
explicit source allowance omits both, so run usage metrics remain absent.

The current GitHub wrappers expose `headRefName` and `updatedAt`, not a head
SHA. The source hash uses those fields plus check state. Releases without an
observed card, session or PR reference retain factual direction but omit
writer prose rather than inventing a reference.

Writer state uses scope `tower`, key `summary:<scope>`. Alongside `result`,
`valid_hash` and `checked_at`, it persists `pending_turn`, `send_order`,
`failures`, `last_attempt_at` and `last_send_at`. Scheduled sends and Refresh
reserve the same send budget. On startup, the worker reconciles a pending turn
with the writer session's current Harness status.

## Development

Run from this directory:

```sh
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm install
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm typecheck
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm lint
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm test
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm build:bundle
```

The worker uses the lockfile in this directory. The UI uses the root pnpm
workspace and its `@iii-dev/console-ui` package. `build:bundle` installs those UI
dependencies from the frozen root lockfile, runs the strict design lint and
build, and copies `page.js` and `styles.css` into `dist/bundle/` beside the worker.
The release lists all bundled files explicitly. Build from the monorepo
source or start the published artifact.

Run UI checks from `ui/` with the same environment:

```sh
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm --dir ../.. --filter @iii-workers/tower-ui run typecheck
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm --dir ../.. --filter @iii-workers/tower-ui run build
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm --dir ../.. --filter @iii-workers/tower-ui run test
```

The page reads held calls directly through `approval::list-pending`; they never
enter a `tower::*` response. Actions are console calls to the target workers.
Decision side chats use the optional host capabilities, and are hidden when the
host lacks them. Browsers sharing an engine can create duplicate copies if they
open the same decision concurrently, since the creation lock is browser-local.
Overview keeps the last visit per browser via `usePaneState`.
Run usage metrics were removed because the read model does not supply them.
The worker UI has no Storybook setup; fixture tests and captures replace the
old ADE-only stories.
