---
name: learn-iii
description: >-
  The console tour. Its agent steps add a kanban board in the todo-local
  namespace, then build a TODO worker on top of it: a sidebar-like page beside
  the board with a due date, a due time, a priority, a repeat and an alarm on
  each item, shown as pills on the board cards. Read this before you do either
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

The board keeps the progress. The TODO worker adds what a to-do list needs on
top, and shows it on the board. Build it as a Node worker named `todo`
(the `kanban/iii-node/index` skill has the package shape, the injectable page
and the compose wiring) and add it to the project as a `path://` container.

Data:

- Every item is a ticket on the todo-local board:
  `kanban-todo-local::ticket::create { "title", "actor": "todo" }`.
- The item's own fields live in the `state` worker, scope `todo`, key = the
  ticket uuid: `{ "due_date": "2026-10-24", "due_time": "17:00", "priority": "high", "repeat": "none|daily|weekly|monthly", "alarm": true|false }`.
- After every write, put the fields on the card:
  `kanban-todo-local::ticket::update { "id", "pills": [...] }` with one pill
  per set field, icon then text: `calendar` + due date, `clock` + due time,
  `flag` + priority, `repeat` + repeat, `alarm-clock` + "alarm". `pills: []`
  clears them. An icon is a lucide icon name.

Done:

- `kanban-todo-local::config::info` gives `done_status` and `default_status`.
- Checking Done calls `kanban-todo-local::ticket::move { "id", "status": <done_status> }`;
  unchecking moves it back to `default_status`. The board subscribes to its
  own change trigger, so the card moves at once.
- Bind `kanban-todo-local:change` too, so a move on the board updates the
  checkbox on the TODO page.
- A done item with a repeat creates the next occurrence: a new ticket with the
  same fields and the due date moved one day, week or month on.

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
  (`iii.registerTriggerType`). Fire it on create, edit, done, undone and alarm
  with `{ "event", "ticket_id", "title", "fields" }`. The Reactivity step binds
  it from the chat.

Page:

- One injectable console page, id `todo`, title `TODO`, built as the
  `kanban/iii-node/index` skill shows (`todo::ui-content`,
  `console:script` with `path: "todo/page.js"`, a stylesheet scoped
  `[data-iii-ui="todo"]`).
- It reads like a sidebar: one narrow column; a list of open items, each with
  a checkbox, the title and its pills; the done items under a divider; an
  add form at the top with a title, `<input type="date">`,
  `<input type="time">`, a priority select, a repeat select and an alarm
  checkbox. Keep the page its own; it is not inside the board.
- Open it beside the board when the build is done:
  `console::workspace::open { "screen": "todo", "relative_to": "kanban-todo-local-board" }`.

Checks: call the functions and read the board (`kanban-todo-local::board::get`)
to prove each part; take no screenshots. Do not move the operator to another
workspace.
