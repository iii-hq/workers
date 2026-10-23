---
name: chat-mentions
description: Make a worker's items mentionable in chat as @<name>(id="<id>") — tickets, sessions, traces, emails, calendar events, users — with autocomplete in the ADE composer, rich pills with a hover preview, and agents receiving each mention already resolved. Covers the two functions and the metadata descriptor a provider registers (Node, Rust, any language), ranking and view rules, the optional preview renderer, permissions, and how to verify it end to end.
---

# Chat mentions

A worker becomes a **mention provider** by registering two functions and
declaring itself in one of them. Nothing else is needed — no console change,
no harness change:

- **The user** types `@<name>` in the ADE composer, presses Tab, searches the
  worker's items and picks one; the message carries `@<name>(id="<id>")`, shown
  as a pill (icon, color, name) that previews on hover and opens on click.
  Typing `@` plus text searches every provider at once, one group each.
- **The agent** gets, with the message, a `<mentions>` block from the judge's
  hook: each item's one-line summary and a pre-verified call that returns the
  full item. It may also write the token in replies; the console renders it.

The contract is `crates/mention-contract` (Rust types, token grammar, shared
fixtures). The kanban (`@kanban`), session-manager (`@session`) and console
(`@trace`) workers are reference providers.

## When to Use

- The user should be able to point the agent at one of the worker's records
  without pasting ids: a ticket, a document, an email, an event, a user.
- The worker's records deserve a recognizable pill in chat (icon, color,
  status) instead of a raw id.
- An agent working with the worker should reference records in its replies
  so the user can open them with one click.

## Boundaries

- Mention functions are console and hook plumbing: register them
  `internal: true` (agents never see them in discovery) and
  `trace_hidden: true` (search runs on every keystroke). Agents read a
  mentioned record through the **details** function you name — an ordinary,
  agent-allowed function such as `<worker>::item::get`.
- Search and get only read. Never mutate state, never return secrets or
  another user's private data (session-manager leaves the parked draft out of
  its view).
- One token name per provider; `fn`, `file` and `skill` are taken. Two
  workers claiming one name: the lexicographically first get-function id
  wins, everywhere.
- The id in the token is the record's canonical, stable id. Human handles
  (`KAN-12`) belong in `hint`; a get may accept them but answers with the
  canonical id.

## The two functions

**Search** — `{ query, limit?, context? }` → `{ items }`, best match first.

```json
{ "query": "fix log", "limit": 8,
  "context": { "session_id": "console-…", "working_dir": "/repo" } }
→ { "items": [ { "id": "6ac4f6df-…", "label": "Fix login redirect", "hint": "KAN-12",
                 "description": "In progress · high", "icon": "ticket", "color": "amber" } ] }
```

- An empty `query` returns the most relevant recent items (the console asks
  for them as soon as the user scopes to `@<name>:`).
- Rank exact handles first (`KAN-12`, `12`), then title prefix, then every
  query word starting a title word, then substring, then body text. Break ties
  by most recently updated. Skip deleted or archived records.
- `limit` defaults to 8, at most 50; the global `@text` menu asks for 4.
- Answer fast: the console gives each provider 2.5 s and moves on without it.
- `context.session_id` is the chat the search was asked from; leave that
  record out when it makes no sense to mention (session-manager drops the
  asking session).

**Get** — `{ id }` → a view, or `null` for an id the worker does not know.

```json
{ "id": "6ac4f6df-…" }
→ { "id": "6ac4f6df-…", "label": "Fix login redirect", "hint": "KAN-12",
    "description": "In progress", "icon": "ticket", "color": "amber",
    "fields": [ { "label": "Priority", "value": "high", "tone": "warning" },
                { "label": "Assignee", "value": "backend-engineer" } ],
    "open": { "page": "<worker>-item", "context": { "id": "KAN-12" } },
    "summary": "Ticket KAN-12 \"Fix login redirect\" · status: In progress · priority: high",
    "data": { "…": "your domain object, for your own preview renderer" },
    "updated_at": "2026-10-06T20:17:20.749Z" }
```

- `label` and `hint` make the pill; `description` and up to six `fields` make
  the hover card.
- `summary` is the one line the agent reads with the message. Make it
  self-contained: kind, handle, quoted title, state. Without it the agent
  gets `hint "label" — description · field: value …`.
- `open` is one of `{ "page", "context" }` (a page the worker registered with
  `host.pages`; `traces` is the built-in traces screen, `context.trace_id`
  picks the trace), `{ "session": "<session id>" }` or `{ "url": "https://…" }`
  (http/https only). Omit it when there is nothing to open.
- `data` is optional and only reaches a preview renderer; keep it a small
  projection (kanban sends the board card plus a 280-character description
  excerpt), never the full record with history.
- Vocabulary: `icon` is one of `ticket`, `issue`, `task`, `session`, `chat`,
  `message`, `post`, `trace`, `span`, `activity`, `event`, `calendar`,
  `email`, `mail`, `user`, `person`, `team`, `group`, `channel`, `hash`,
  `tweet`, `mention`, `file`, `doc`, `document`, `folder`, `link`, `agent`,
  `bot`, `database`, `table` (anything else falls back to `@`). `color` is one
  of `neutral`, `blue`, `purple`, `teal`, `green`, `amber`, `rose`. A field's
  `tone` is `neutral`, `info`, `success`, `warning` or `danger`. Items may
  override the provider's icon and color (color by status or priority).

## The descriptor

Register it as `metadata.mention` on the **get** function — the function that
carries it *is* the get function, so the descriptor names only the search:

```json
{
  "internal": true,
  "trace_hidden": true,
  "mention": {
    "v": 1,
    "name": "<name>",
    "label": "Tickets",
    "description": "A ticket, by uuid or key (KAN-12)",
    "icon": "ticket",
    "color": "blue",
    "search": "<worker>::mention::search",
    "details": { "function_id": "<worker>::ticket::get", "id_field": "id" }
  }
}
```

- `name`: lowercase letters, digits and `-`, starting with a letter, at most
  40 characters. It is what users type after `@`, so prefer the domain noun
  (`calendar`, `email`) over the worker's package name when they differ.
- `label`: the plural noun of one group header ("Tickets", "Events").
- `description`: one line on what an id refers to; agents read it in
  `<mention_providers>`, so say how the id looks.
- `details`: the agent-facing function returning the full record, called with
  `{ <id_field>: "<id>" }` (`id_field` defaults to `id`). It must be callable
  by agents (allowed in the worker's `iii-permissions.yaml`); the hook offers
  it only when the session's policy allows it. Omit it when the summary is all
  there is.

## Registering them

**Node** (`iii-sdk`; see `iii-node` for the worker shape):

```ts
const MENTION = {
  v: 1,
  name: 'calendar',
  label: 'Events',
  description: 'A calendar event, by event id',
  icon: 'event',
  color: 'purple',
  search: 'calendar::mention::search',
  details: { function_id: 'calendar::event::get', id_field: 'id' },
}

iii.registerFunction('calendar::mention::search', searchEvents, {
  description: 'Search events for the chat @calendar mention menu; best match first.',
  request_format: MENTION_SEARCH_REQUEST,
  response_format: MENTION_SEARCH_RESPONSE,
  metadata: { internal: true, trace_hidden: true },
})

iii.registerFunction('calendar::mention::get', eventView, {
  description: 'Resolve a @calendar(id=…) chat mention to its view; null for an unknown id.',
  request_format: MENTION_GET_REQUEST,
  response_format: MENTION_VIEW_OR_NULL,
  metadata: { internal: true, trace_hidden: true, mention: MENTION },
})
```

Write the four JSON Schemas from the shapes above (copy them from the
session-manager goldens, `session-manager/tests/golden/schemas/session.mention.*.json`,
which are exactly these types).

**Rust** (path dependency on `crates/mention-contract`):

```rust
use mention_contract::{MentionProvider, MentionSearchRequest, MentionSearchResponse,
    MentionGetRequest, MentionView, color};

pub fn provider() -> MentionProvider {
    MentionProvider::new("calendar", "Events", "calendar::mention::search")
        .description("A calendar event, by event id")
        .icon("event")
        .color(color::PURPLE)
        .details("calendar::event::get", "id")
}

iii.register_function("calendar::mention::get",
    RegisterFunction::new_async(move |req: MentionGetRequest| get(req)) // -> Result<Option<MentionView>, _>
        .description("Resolve a @calendar(id=…) chat mention to its view; null for an unknown id.")
        .metadata(provider().metadata()));
```

`MentionProvider::metadata()` already adds `internal` and `trace_hidden`;
give the search function `json!({ "internal": true, "trace_hidden": true })`.
`MentionView::agent_summary()` is the fallback line; `MentionSearchRequest::
effective_limit()` applies the default and the cap.

**Python or anything else**: the same JSON — the console and the judge read
only `engine::functions::list` metadata and call the two functions by id.

Permissions: deny both mention functions to agents and allow the details
function:

```yaml
rules:
  - '!<worker>::mention::search'
  - '!<worker>::mention::get'
  - <worker>::ticket::get
```

## A richer preview (optional)

Without UI code the hover shows the generic card (icon, provider label,
handle, name, description, fields). A worker that ships console UI can draw
its own card from `view.data`:

```tsx
host.mentions?.registerRenderer({
  provider: '<name>',
  Preview: ({ view, open }) => <EventCard event={view.data as Event} onOpen={open} />,
})
```

Feature-detect `host.mentions` (older consoles lack it). If the renderer
throws — for example `data` from an older backend — the generic card is
shown, so throw rather than render a half card. Keep it about 22rem wide.

## What the agent sees

You write nothing for this; it is how to phrase `summary` and `description`.
On the first step of a session the judge appends:

```text
<mention_providers>
- @calendar — Events: A calendar event, by event id
</mention_providers>
```

and, for each new mention a user wrote:

```text
<mentions>
- @calendar(id="evt_42") — Event "Design review" · Tue 14:00–15:00 · 6 attendees
  details: calendar::event::get {"id":"evt_42"}
</mentions>
```

The chat shows the latter as a quiet "Context for the agent" row.

## Definition of done

- `engine::functions::list { include_internal: true }` lists the get function
  with `metadata.mention`, and the descriptor validates (name rules, non-empty
  `label` and `search`).
- `iii trigger <worker>::mention::search --json '{"query":""}'` answers recent
  items fast; a handle query (`KAN-12`) ranks that record first.
- `iii trigger <worker>::mention::get --json '{"id":"<id>"}'` answers the view,
  and `null` for an unknown id.
- `iii trigger judge::mentions::resolve --json '{"text":"@<name>(id=\"<id>\")"}'`
  answers `status: "resolved"` with your summary and the details call.
- In the ADE composer `@<name>` shows the provider, Tab scopes the search,
  picking inserts a pill; the pill previews on hover and opens on click.
- Unit tests cover ranking (handles before titles, empty query = recent,
  deleted records skipped) and the view (fields, open target, summary).
