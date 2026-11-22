# tower

The tower worker owns the control tower read model and summary schedule. It
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

- `tower::overview {scope}` returns the CEO view and latest valid summary.
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

The backend deploy includes the worker bundle, schema and skill. The build also
copies `ui/dist/page.js` and `ui/dist/styles.css` when they exist, after removing
stale copies. A UI delivery must add its build and explicit deploy includes in
the UI stage; these optional assets are not part of the backend artifact.
