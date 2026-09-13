# kanban

A file-backed kanban board for an iii project. Tickets, threaded comments,
an activity timeline and agent assignment live in one `board.json`; every
mutation is a `kanban::*` function, and two trigger types (`kanban:change`,
`kanban:comment`) let agents wake on what happens to a ticket instead of
polling. The worker injects a board page and a ticket screen into the
console, and ships five agent profiles — Product Manager, Tech Lead, Backend
Engineer, Frontend Engineer, iii ADE Worker Designer — that plan, dispatch,
build and review work as tickets on this board.

## Table of contents

- [Install](#install)
- [Quickstart](#quickstart)
- [Configuration](#configuration)
- [Custom trigger types](#custom-trigger-types)
- [Console pages](#console-pages)
- [Agent profiles](#agent-profiles)
- [Skills](#skills)
- [Development](#development)

## Install

```bash
iii trigger compose::add worker=kanban
```

`iii trigger compose::add` declares the worker in `worker-compose.yaml` and
starts it as part of the Compose project. The `configuration` worker (built
into the engine) stores the board settings; `iii-directory` is optional and
supplies the agent profiles the board can assign to.

## Quickstart

Create a ticket, claim it, comment, and hand it off. Every ticket has an
internal uuid and a human key (`KAN-1`); both work everywhere a ticket is
addressed, case-insensitively.

```bash
iii trigger kanban::ticket::create title="Search filters persist" priority=high
iii trigger kanban::ticket::update id=KAN-1 assignee=backend-engineer status=in_progress actor=backend-engineer
iii trigger kanban::comment::create ticket_id=KAN-1 body="Pushed the fix; ready for review." author=backend-engineer
iii trigger kanban::ticket::move id=KAN-1 status=in_review actor=backend-engineer
iii trigger kanban::ticket::get id=kan-1
```

From a worker, the same calls go through the SDK:

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{InitOptions, register_worker};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let ticket = iii
        .trigger(TriggerRequest {
            function_id: "kanban::ticket::create".into(),
            payload: json!({ "title": "Search filters persist", "priority": "high" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;
    println!("{}", ticket["key"]); // KAN-1

    let board = iii
        .trigger(TriggerRequest {
            function_id: "kanban::board::get".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;
    println!("{}", board["columns"]);
    Ok(())
}
```

`kanban::ticket::get` returns the ticket with its `comments` and `activity`
inline; `kanban::board::get` returns every configured column with its ticket
summaries (no description or thread). Both list reads take `updated_since`
and `compact`, and the board takes `statuses` and a per-column `limit`, so an
agent reads the active lanes in one small call and the `done` column only for
what changed:

```bash
iii trigger kanban::board::get --json '{"statuses":["todo","in_progress","in_review"],"compact":true}'
iii trigger kanban::ticket::list --json '{"status":"done","updated_since":"2026-09-10T00:00:00.000Z","compact":true}'
```
 Pass `actor` on writes and `author` on comments: the timeline
reads honestly, and the comment filters key on exactly that string. Deleting
a ticket (`kanban::ticket::delete`) is a soft delete — the row stays in the
board file with `deleted_at` set and `kanban::ticket::restore` undoes it —
and it is reserved for a person on the console's ticket screen; agents move
a finished ticket to `in_review` or `done` instead.

The full function list is in
[`skills/SKILL.md`](skills/SKILL.md); request and response schemas come from
`engine::functions::info`.

## Configuration

Settings live in the `configuration` worker under the id `kanban` and are
edited from the console's Kanban settings form (columns, priorities, prefix,
paths) or by writing the YAML file the configuration worker persists. The
worker hot-reloads on `configuration:updated`; no restart is needed.

```yaml
data_path: data/kanban        # folder holding board.json; relative to III_COMPOSE_DIR (or the process cwd)
id_prefix: KAN                # human keys become KAN-1, KAN-2, …  (^[A-Z][A-Z0-9]*$)
default_status: backlog       # column a new ticket lands in when none is given
columns:                      # left to right; `id` is what tickets store, `label` is what the board shows
  - { id: backlog, label: Backlog }
  - { id: todo, label: To do }
  - { id: in_progress, label: In progress }
  - { id: in_review, label: In review }
  - { id: done, label: Done }
priorities: [low, medium, high, urgent]   # offered on a ticket, lowest first; the first one is the default
agents_path: agents           # fallback folder of <id>.md agent profiles when iii-directory is not installed
```

A stored value is repaired rather than refused: an unknown `default_status`
falls to the first column, an invalid prefix falls back to `KAN`, duplicate
column ids collapse, and an empty list falls back to the default. Pass
`--config <file>.yaml` to seed the entry on first boot only; afterwards the
configuration worker is the source of truth. `kanban::config::info` prints
the resolved absolute paths.

## Custom trigger types

| Trigger type | Config | Fires when | Payload to subscribers |
|---|---|---|---|
| `kanban:change` | `{ ticket_id?, events?, ticket?, metadata? }` | After every mutation. `ticket_id` (uuid or key) and `events` narrow it; an empty config fires for everything. | `{ event, ticket_id, ticket_key, ticket, comment, activity, at }` — `event` is one of `ticket.created`, `ticket.updated`, `ticket.moved`, `ticket.deleted`, `ticket.restored`, `comment.created`. |
| `kanban:comment` | `{ ticket_id, author?, exclude_author?, root_only?, ticket? }` | A comment is created on that ticket. `ticket_id` is required; `author` keeps one author, `exclude_author` drops one (your own id), `root_only` ignores replies. | `{ event: "comment.created", ticket_id, ticket_key, ticket, comment, author, at }` |

`ticket` on either type is `full` (default: the ticket with comments and
activity inline) or `summary` (the board-card fields only); an agent that
re-reads the ticket on a wake should pass `summary` so each wake costs a card,
not a thread. `kanban:change` has no author filter, so a feedback loop that
must not wake on its own comments uses `kanban:comment` with `exclude_author`:

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": { "ticket_id": "KAN-7", "exclude_author": "backend-engineer", "ticket": "summary" },
  "once": false,
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

Bindings live in the worker's in-process map and die with the process;
`kanban::config::info` reports how many are held under `subscribers`. Pair a
standing wake with a `cron` check-in over `kanban::comment::list`, which
takes the same filters plus `since`.

## Console pages

While the worker is connected it injects two pages into the console:

- **Kanban** (`#/ext/kanban-board`) — the board: one lane per configured
  column, drag and drop between lanes, a new-ticket dialog, and a lane picker
  when the pane is narrow. Clicking a card opens the ticket beside the
  board; the card's **Open in this tab** action keeps it in the current pane
  instead (the console reuses any tab that already shows a ticket pane), and
  the inline view offers **Open in its own pane** to go the other way.
- **Ticket** (`#/ext/kanban-ticket`) — one ticket: status, priority and
  assignee (from `kanban::agent::list`), a Markdown description with an
  editor, and the activity timeline with threaded comments. Delete and
  restore live here, behind a confirmation.

Both pages stay live over a `kanban:change` binding, and the Kanban
settings form edits the configuration above. Chat renders `kanban::*`
results as ticket cards and `kanban:*` trigger activity with the ticket and
comment it carried. Command palette rows: New ticket, Refresh board, Refresh
ticket, Edit description, Write a comment, Back to the board.

## Agent profiles

`agents/` ships five profiles. Each is a system prompt with preloaded
`skills` and `functions`; the harness freezes both into every session that
runs as that profile, and the board is the only channel between them.

| Profile id | Role | Preloaded skills |
|---|---|---|
| `product-manager` | Interviews the user, writes tickets whose descriptions carry observable acceptance criteria, and gates `in_review` → `done` (any caveat goes back to `in_progress`). | `kanban/tickets/feature-planning`, `kanban/tickets/acceptance-review`, `kanban/tickets/kanban-tickets` |
| `tech-lead` | Splits one feature on the seam between backend and frontend, dispatches each ticket with `harness::spawn`, stays reachable on comment wakes, verifies the seam. | `kanban/tickets/ticket-orchestration`, `kanban/tickets/kanban-tickets`, `kanban/tickets/acceptance-review` |
| `backend-engineer` | Builds workers, functions, triggers and configuration; verifies with a real call; reports on the ticket. | `kanban/iii-node/index`, `kanban/iii-node/configuration`, `kanban/tickets/ticket-worker` |
| `frontend-engineer` | Builds the browser app (Vite, React, TanStack, iii-browser-sdk); verifies in a real browser; reports with screenshots on the ticket. | `kanban/frontend/iii-browser-sdk`, `kanban/frontend/react`, `kanban/frontend/vite`, `kanban/frontend/tanstack-router`, `kanban/frontend/tanstack-query`, `kanban/frontend/web-accessibility`, `kanban/frontend/web-performance`, `kanban/frontend/frontend-testing`, `kanban/tickets/ticket-worker` |
| `ade-worker-designer` | Designs and builds the UI a worker injects into the ADE console — pages, renderers, configuration forms, scoped styles — against `@iii-dev/console-ui` and the iii Schematic design system; verifies in the running console. | `kanban/ade-worker-design/index`, `kanban/ade-worker-design/console-injectable-ui`, `kanban/ade-worker-design/patterns`, `kanban/ade-worker-design/console-design`, `kanban/iii-node/configuration`, `kanban/tickets/ticket-worker` |

All four extend the bundled `iii-minimal` identity and use their profile id
as `actor` / `author` / `exclude_author`, so the loop in
[`skills/kanban-tickets.md`](skills/kanban-tickets.md) closes.

### Where they land

- **Registry install.** `iii trigger compose::add worker=kanban` publishes
  the profiles into `<agents_folder>/<id>.md` and the skills under
  `<skills_folder>/kanban/…`, which is why the skill ids above are prefixed
  `kanban/`.
- **Local `path://` install.** Copy `agents/*.md` into the project's
  `agents/` folder and `skills/**` into `skills/kanban/` (keeping the same
  relative layout) so the ids the profiles preload resolve.

`directory::agents::list` shows them; `kanban::agent::list` is what the board
assigns from.

### Picking a model per profile

The profiles ship without a `model`, so each session takes the model of the
send (the console's model picker or `harness::send { options: { model } }`).
To pin one, add `model` and optionally `reasoning_effort` to the
frontmatter as `provider::model`. `router::models::list` prints the catalog
for the providers you installed (its ids read `provider/model`; the profile
spells the same key with `::`). The examples below use the `claude-code`
provider; with the API-key provider the same models are `anthropic::…`.

```yaml
---
name: Tech Lead
model: claude-code::claude-fable-5-1
reasoning_effort: high
extends: iii-minimal
---
```

`model` inherits through `extends`, so setting it on a shared base profile
pins every child. A useful split, from most to least demanding:

| Profile | What the work needs | Pick |
|---|---|---|
| `tech-lead` | Decomposition, contract design, judging a seam that "passes" but does not work; the costliest mistakes happen here. | The strongest model you run: `claude-code::claude-fable-5-1` (or `claude-code::claude-opus-5`) with `reasoning_effort: high`. |
| `product-manager` | Interviewing, writing precise acceptance criteria, refusing caveats at the gate. Long context, careful language, few calls. | `claude-code::claude-opus-5` at medium effort; `claude-code::claude-sonnet-5` is the economy choice. |
| `backend-engineer` | Many tool calls, real verification, Rust/Node code. Gains more from a strong coder than from deep deliberation. | `claude-code::claude-sonnet-5` (or an `openai-codex::…` model if Codex is your coding provider); raise to Opus for protocol or migration work. |
| `frontend-engineer` | Same shape as backend plus browser verification and screenshots; visual judgement matters. | `claude-code::claude-sonnet-5`; Opus when the ticket is a design pass. |
| `ade-worker-designer` | Long design-system and contract skills preloaded (about 40k tokens), visual judgement against screenshots, strict adherence to `index.d.ts`. | `claude-code::claude-opus-5`; Sonnet for small renderer or form tickets. |

Edit the file in `agents/` (or `directory::agents::update { id, … }`) and
the next session that runs as that profile picks it up; sessions already
running keep the model they started with.

## Skills

`skills/SKILL.md` is the worker overview agents read first. The rest is the
working doctrine the profiles preload, one folder per concern (each folder's
`index.md` maps its files):

- `tickets/` — how work moves through the board: `kanban-tickets` (the
  shared loop, the comment wake, review vs done, never delete),
  `feature-planning` and `acceptance-review` (the Product Manager's halves),
  `ticket-orchestration` and `ticket-worker` (the dispatcher's and the
  dispatched engineer's halves).
- `iii-node/` — building a Node worker (package shape, functions, triggers,
  asset delivery, compose wiring) with its bundled `configuration` reference.
- `ade-worker-design/` — the designer's half: the injectable UI contract
  (`console-injectable-ui`), the iii Schematic design system
  (`console-design`) and the record-shaped interaction recipes (`patterns`).
- `frontend/` — the Frontend Engineer's browser stack: `iii-browser-sdk`,
  `react`, `vite`, `tanstack-router`, `tanstack-query`, `web-accessibility`,
  `web-performance`, `frontend-testing`.

Fetch any of them with `directory::skills::get { "id": "kanban/<folder>/<name>" }`.

## Development

```bash
# UI (esbuild; react and @iii-dev/console-ui stay external)
pnpm --dir kanban/ui build
# worker
cd kanban
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

`build.rs` runs the UI build when `ui/dist` is missing or stale; set
`SKIP_UI_BUILD=1` to embed the existing output. For the hot-reload loop run
`pnpm --dir kanban/ui watch` in one terminal and
`III_KANBAN_UI_WATCH=1 cargo run --bin kanban` in another; every open
console tab swaps the asset on rebuild. The worker connects to `III_URL`
(default `ws://127.0.0.1:49134`) and resolves `data_path` and `agents_path`
against `III_COMPOSE_DIR`.
