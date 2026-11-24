# control-plane 0.0.1

The `control-plane` worker owns the Control plane page, read model and summary
schedule. Open it from the console navigation, via `#/workspace/control-plane`,
or on its own at `#/worker/control-plane`. It aggregates harness roots and asks,
kanban projects, cards, releases and questions, and pull requests from repositories
declared on projects.

The worker requires configuration, harness, session-manager, kanban, state and
iii-directory. ADE hosts the page but is not required to start the backend.
The optional GitHub worker supplies pull requests for repositories set on a
Kanban project's Overview.

## Configuration

The worker connects to `III_URL`, or `III_ENGINE_URL` if `III_URL` is unset.
If neither is set, it uses the SDK default. For WebSocket URLs with the exact
IPv4 wildcard host `0.0.0.0`, the worker connects to `127.0.0.1` instead,
without changing the port, path or query. This does not change where the
engine listens.

The worker registers the id provided by Compose in `III_CONFIG_NAME`, with
`control-plane` as the default. Open the gear in the Control plane page to set the
summary model and stall timeout.

| Key | Default |
| --- | --- |
| `stall_after_min` | `30` |
| `summary.model` | unset |
| `summary.interval_min` | `10` |
| `summary.reasoning` | unset (the model's default effort) |
| `summary.language` | `English` |
| `summary.guidance` | unset (free text: what the summary should focus on, its tone and length) |
| `prs.watched_interval_min` | `5` |
| `prs.idle_interval_min` | `30` |
| `escalate` | `{}` |
| `needs_you.card_labels` | `human-only`, `só humano`, `decisão` (open cards with one of these labels, or the "needs you" pill, show up in Needs you) |

An unset project escalation list means `security`, `publish`, `external_cost`
and `data`. To enable the summary, apply the `control-plane` kit so iii-directory
can resolve its `control-plane-writer` profile, then set `summary.model` to an
available `provider::model`.

## Functions and event

- `control-plane::overview {scope}` returns the Overview facts (tiles, Needs you, releases, runs, PRs, board), the latest valid summary and
  `prs_interval_min` (the watched PR interval used by the page freshness label).
- `control-plane::needs-you::list {scope}` returns questions and stalled roots.
- `control-plane::runs::list {scope}` returns scoped roots and context sizes.
- `control-plane::prs::list {scope}` returns cached open pull requests.
- `control-plane::decided::list {scope, cursor?}` pages settled questions.
- `control-plane::assumptions::list {scope, since?}` filters recorded assumptions by comment time.
- `control-plane::watch {scope}` watches the scope for two minutes.
- `control-plane::summary::refresh {scope}` requests a rate-limited writer turn.
- `control-plane::configuration-id {}` returns `{id}` for the worker's settings entry.
- `control-plane::on-changed {scope, as_of, parts}` announces changed read-model parts.

`scope` is `all` or a project id. `availability.github` indicates whether the
GitHub pull request functions are available.

## Source boundaries

Root metadata comes from `session::list`; context size comes from verbose
`harness::status`, with a cached messages-tail estimate when unavailable.
The read model does not call `session::get` or `harness::metrics`: revision 5's
explicit source allowance omits both, so run usage metrics remain absent.

The current GitHub wrappers expose `headRefName` and `updatedAt`, not a head
SHA. The source hash uses those fields plus check state.

Writer state uses scope `control-plane`, key `summary:<scope>`. Alongside `result`,
`valid_hash` and `checked_at`, it persists `pending_turn`, `send_order`,
`failures`, `last_attempt_at` and `last_send_at`. Scheduled sends and Refresh
reserve the same send budget. On startup, the worker reconciles a pending turn
with the writer session's current Harness status.

## Development

Run from this directory:

```sh
pnpm install
pnpm typecheck
pnpm lint
pnpm test
pnpm build:bundle
```

The worker uses the lockfile in this directory. The UI uses the root pnpm
workspace and its `@iii-dev/console-ui` package. `build:bundle` installs those UI
dependencies from the frozen root lockfile, runs the strict design lint and
build, and copies `page.js` and `styles.css` into `dist/bundle/` beside the worker.
The release lists all bundled files explicitly. Build from the monorepo
source or start the published artifact.

Run UI checks from `ui/` with the same environment:

```sh
pnpm --dir ../.. --filter @iii-workers/control-plane-ui run typecheck
pnpm --dir ../.. --filter @iii-workers/control-plane-ui run build
pnpm --dir ../.. --filter @iii-workers/control-plane-ui run test
```

Actions are console calls to the target workers.
Decision side chats use the optional host capabilities, and are hidden when the
host lacks them. Browsers sharing an engine can create duplicate copies if they
open the same decision concurrently, since the creation lock is browser-local.
The Overview follows the design canvas; the summary model is chosen with the chat's model picker.
Run usage metrics were removed because the read model does not supply them.
The worker UI has no Storybook setup; fixture tests and captures replace the
old ADE-only stories.
