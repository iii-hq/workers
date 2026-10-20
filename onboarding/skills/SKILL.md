---
name: onboarding
description: >-
  The console tour. Its agent steps add a kanban board in the todo-local
  namespace, then build a TODO worker on top of it: a narrow sidebar page left
  of the board that adds items and sets a due date, a due time, a priority, a
  repeat and an alarm on each, shown as pills on the board cards, plus the
  same controls as a web page the agent opens for the operator. The board
  is the list; the sidebar is the controls. Read this before you do either
  step.
---

# learn-iii

`learn-iii` is the guided tour of the console. Two of its steps ask you to
build. This file is the recipe for both. The tour page waits on a `state`
write (`step_<id>_completed` in the `learn-iii` scope) that each prompt asks
for; write it as you end your turn.

## Add the board

The tour wants a kanban board that sits apart from any board the operator
already runs. The `kanban` worker takes one environment variable for that:

| Setting | Value |
| --- | --- |
| `KANBAN_NAMESPACE` | `todo-local` |

With it set, the worker registers as `kanban-todo-local`, and every id
carries that name in place of `kanban`:

- functions `kanban-todo-local::ticket::create`, `kanban-todo-local::ticket::move`, `kanban-todo-local::config::info`, and so on;
- trigger types `kanban-todo-local:change` and `kanban-todo-local:comment`;
- console pages `kanban-todo-local-board` and `kanban-todo-local-ticket`;
- the configuration entry `kanban-todo-local` (same form as `kanban`);
- the data folder `data/kanban-todo-local`.

Steps:

1. Read `worker-compose.yaml`. When a container already sets
   `KANBAN_NAMESPACE: todo-local`, the board is there: go to step 3.
2. When no container is named `kanban`, call `compose::add` with a container
   object:

   ```json
   { "workers": [{ "worker": "kanban", "environment": { "KANBAN_NAMESPACE": "todo-local" } }] }
   ```

   `compose::add` names the container after the worker, so when a `kanban`
   container exists already, write the entry into `worker-compose.yaml`
   yourself and start it with `compose::up { "worker": "kanban-todo-local" }`:

   ```yaml
     kanban-todo-local:
       worker: package://kanban
       environment:
         KANBAN_NAMESPACE: todo-local
   ```

3. Confirm with `engine::functions::list { "prefix": "kanban-todo-local::" }`,
   then open the board beside the chat:
   `console::workspace::open { "screen": "kanban-todo-local-board", "relative_to": "chat" }`.

The board's own doctrine is the `kanban` skill (`directory::skills::get
{ "id": "kanban" }`); swap the `kanban` prefix for `kanban-todo-local`.

## Build the TODO worker

The board is the list: every item is a ticket, and the board shows them. The
TODO worker shows no list of its own. It is a narrow sidebar of controls, open
to the left of the board, that creates items and sets what a to-do needs on
top: a due date, a due time, a priority, a repeat and an alarm. Build it as a
Node worker named `todo` (the `kanban/iii-node/index` skill has the package
shape, the injectable page and the compose wiring) and add it to the project
as a `path://` container.

Board shape:

- A to-do board has two columns. Before anything else, set them:

  ```json
  configuration::set {
    "id": "kanban-todo-local",
    "value": {
      "data_path": "data/kanban-todo-local",
      "columns": [{ "id": "todo", "label": "TODO" }, { "id": "done", "label": "Done" }],
      "default_status": "todo",
      "done_status": "done",
      "canceled_status": null
    }
  }
  ```

  The board reloads on the write; no restart. Read `configuration::get
  { "id": "kanban-todo-local" }` first and keep `id_prefix` and `priorities`
  as they are.

Data:

- Every item is a ticket on the todo-local board:
  `kanban-todo-local::ticket::create { "title", "priority", "actor": "todo" }`.
  The board's own `priority` is the item's priority; do not keep a second one.
- The item's other fields live in the `state` worker, scope `todo`, key = the
  ticket uuid: `{ "due_date": "2026-10-24", "due_time": "17:00", "repeat": "none|daily|weekly|monthly", "alarm": true|false }`.
- After every write, put the fields on the card:
  `kanban-todo-local::ticket::update { "id", "pills": [...] }` with one pill
  per set field, icon then text: `calendar` + due date, `clock` + due time,
  `repeat` + repeat, `alarm-clock` + "alarm". `pills: []` clears them. An
  icon is a lucide icon name.

Done:

- Done is the board's: the card's status picker, a drag to the Done column,
  or `kanban-todo-local::ticket::move { "id", "status": "done" }`.
- Bind `kanban-todo-local:change` (`iii.registerTrigger`), so a move to Done
  on the board reaches the worker: a done item with an alarm drops its cron
  trigger. A done item with a repeat is not finished: move the SAME ticket
  back to the TODO column with the due date moved one day, week or month on
  (`ticket::move` to `default_status`, then the pills), and comment on it
  (`Done 2026-10-24; next 2026-10-25`). Never create a second ticket for
  it: one item is one card.

Alarm:

- The `cron` worker is in the project. For an item with an alarm, register a
  `cron` trigger (`iii.registerTrigger({ type: "cron", function_id: "todo::alarm", config: { expression } })`)
  for the due date and time: six fields, UTC, for example `0 0 17 24 10 *`.
  Keep the trigger id in the item's state so you can unregister it.
- `todo::alarm` comments on the ticket
  (`kanban-todo-local::comment::create { "id", "body": "Alarm: <title>", "author": "todo" }`)
  and fires the worker's own trigger type.

Triggers:

- Register one trigger type, `todo:changed`, with the SDK
  (`iii.registerTriggerType`). Fire it on create, edit, done and alarm with
  `{ "event", "ticket_id", "title", "fields" }`. The Reactivity step binds it
  from the chat.

Page:

- One injectable console page, id `todo`, title `TODO`, built as the
  `kanban/iii-node/index` skill shows (`todo::ui-content`,
  `console:script` with `path: "todo/page.js"`, a stylesheet scoped
  `[data-iii-ui="todo"]`).
- It reads like a sidebar: one narrow column, built from the console-ui
  components, nothing else. At the top, an add form: a title, `<input type="date">`,
  `<input type="time">`, a priority select (the board's `priorities`, from
  `kanban-todo-local::config::info`), a repeat select and an alarm checkbox;
  Add creates the ticket and its pills. Under it, an edit form for one item:
  a select of the open tickets by title (`kanban-todo-local::board::get`,
  `statuses: ["todo"]`, `compact: true`), the same fields, and Save. No
  list of items, no checkboxes: the board next to it is the list, and Done
  is set there.
- Keep the page its own; it is not inside the board. Open it to the left of
  the board when the build is done, and narrow: a sidebar takes the
  console's smallest column, 12% of the tab (`MIN_COLUMN_FRACTION`), and
  the other columns share the rest. `console::workspace::list` gives the
  tab that holds `kanban-todo-local-board` and its `screens`; with `n`
  columns there now, pass `n + 1` widths, `0.12` for the TODO column:

  ```json
  console::workspace::open {
    "screen": "todo",
    "relative_to": "kanban-todo-local-board",
    "direction": "left",
    "sizes": [0.44, 0.12, 0.44]
  }
  ```

  (that example is chat, TODO, board: the TODO column sits where
  `direction: "left"` puts it, just before the board). Style the page for
  that width: one column, fields full width, no table.

Web page:

- The same controls as a web page, outside the console, so the operator can
  use the list from any browser. The `http` worker serves it: when
  `worker-compose.yaml` has no `http` container, `compose::add
  { "workers": [{ "worker": "http" }] }` first. Bind two of the worker's
  functions to the `http` trigger type (`iii.registerTrigger({ type: "http", function_id, config: { api_path, http_method } })`):
  `GET /todo` returns `{ status_code: 200, headers: { "content-type": "text/html" }, body }`
  with one HTML page (plain HTML, a little inline script, no framework), and
  `POST /todo/api/:fn` takes the page's form posts and calls the same code
  the console page calls. `http::status` gives the port (`3111` unless
  changed), so the page is `http://localhost:<port>/todo`. When the `http`
  container fails with "Address already in use", another process holds
  3111: `configuration::set { "id": "http", "value": { "port": 3121 } }`
  and start it again.
- Open it for the operator when it works, in a new console tab, not the
  preview window: `browser::sessions::start { "url": "http://localhost:<port>/todo", "preview": false }`
  (no preview pop), then `console::workspace::open { "screen": "browser", "placement": "new-tab", "activate": false }`.
  `activate: false` keeps the operator on the tab with the TODO page and
  the board; never switch them to the browser tab, and never put the
  browser in their tab. Tell the operator the address too.

Checks: call the functions and read the board (`kanban-todo-local::board::get`)
to prove each part; `browser::fetch` the web page and check it answers 200;
take no screenshots. Do not move the operator to another workspace.
