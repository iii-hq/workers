# tower

The tower worker owns the control tower page, read model and summary schedule.
Open the page at `#/worker/tower` or from the console navigation. It
aggregates harness roots and asks, kanban projects, cards, releases and
questions, and pull requests from repositories declared on projects. All
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
`valid_hash` and `checked_at`, it persists `pending_turns`, `covered_at`,
`failures`, `last_attempt_at` and `last_refresh_at` so result ownership,
backoff and refresh throttling survive a restart.

## Development

Run from this directory:

```sh
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm install
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm typecheck
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm lint
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm test
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm build:bundle
```

The worker and `ui/` share this directory's pnpm workspace and lockfile. The UI
uses the local `packages/console-ui` package without changing the repository UI
workspace. `build:bundle` builds the UI with strict design lint, then copies
`page.js` and `styles.css` into `dist/bundle/` beside the worker. All assets have
explicit deploy includes. A source install in the monorepo builds them; a
published artifact starts its existing bundle directly. Like onboarding, a
source-only copy of `tower/` cannot build the UI without the repository's
`packages/console-ui` directory. Use the published artifact for that layout.

Run UI checks from `ui/` with the same environment:

```sh
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm build
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 pnpm test
```

The page reads held calls directly through `approval::list-pending`; they never
enter a `tower::*` response. Actions are console calls to the target workers.
Decision side chats use the optional host capabilities, and are hidden when the
host lacks them. Overview keeps the last visit per browser via `usePaneState`.
Run usage metrics were removed because the read model does not supply them.
The worker UI has no Storybook setup; fixture tests and captures replace the
old ADE-only stories.
