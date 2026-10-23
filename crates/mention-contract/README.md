# mention-contract

Worker-defined chat mentions. Any worker can let users write
`@<name>(id="<id>")` in a chat — a ticket, a session, a trace, an email, a
calendar event, a Slack user — and have it:

- autocomplete in the console composer (`@kanban`, Tab, then the worker's
  own search);
- render as a pill (icon, color, name) inline, with a preview card on hover
  and its page (or chat, or URL) on click;
- reach the agent already resolved: the judge's mention hook appends a
  one-line summary and a pre-verified call for the full item.

The Rust workers in this repo use this crate; workers in other languages
write the same JSON.

## Becoming a provider

Register two functions.

**Search** (`MentionSearchRequest` → `MentionSearchResponse`), called as the
user types `@<name>:<text>` (and, from two characters on, for a plain
`@<text>`):

```json
{ "query": "fix log", "limit": 8, "context": { "session_id": "…", "working_dir": "…" } }
→ { "items": [ { "id": "6ac4…", "label": "Fix login redirect", "hint": "KAN-12",
                 "description": "In progress · high", "icon": "ticket", "color": "amber" } ] }
```

An empty `query` asks for the most relevant recent items. Keep it fast: the
console gives each provider 2.5 s.

**Get** (`MentionGetRequest` → `MentionView` or `null` for an unknown id):

```json
{ "id": "6ac4…" }
→ { "id": "6ac4…", "label": "Fix login redirect", "hint": "KAN-12",
    "description": "In progress", "icon": "ticket", "color": "amber",
    "fields": [ { "label": "Priority", "value": "high", "tone": "warning" } ],
    "open": { "page": "kanban-ticket", "context": { "id": "KAN-12" } },
    "summary": "Kanban ticket KAN-12 \"Fix login redirect\" · status: In progress",
    "data": { "...": "your domain object, for your own preview renderer" } }
```

- `id` is the canonical id: a get may accept other spellings (a human key)
  but answers with the id the token should carry.
- `open` is one of `{ "page", "context" }` (a console page — `traces` is the
  built-in traces screen, `context.trace_id` picks the trace),
  `{ "session" }` (a chat) or `{ "url" }` (http/https only).
- `summary` is what an agent reads; it defaults to the label, hint,
  description and fields.

**Declare the provider** in the get function's registration metadata:

```json
{
  "internal": true,
  "trace_hidden": true,
  "mention": {
    "v": 1,
    "name": "kanban",
    "label": "Tickets",
    "description": "A kanban ticket, by uuid or human key (KAN-12)",
    "icon": "ticket",
    "color": "blue",
    "search": "kanban::mention::search",
    "details": { "function_id": "kanban::ticket::get", "id_field": "id" }
  }
}
```

Mark the search function `internal` and `trace_hidden` too. In Rust,
`MentionProvider::new(name, label, search)…metadata()` builds this object.

- `name`: lowercase letters, digits and `-`, starting with a letter, at most
  40 characters; `fn`, `file` and `skill` are taken. Two functions claiming
  one name: the lexicographically first function id wins.
- `details` names the agent-facing function that returns the full item,
  called with `{ <id_field>: "<id>" }`. The mention functions stay
  internal; agents use `details`, offered only when their policy allows it.
- `icon`: `ticket`, `issue`, `task`, `session`, `chat`, `message`, `post`,
  `trace`, `span`, `activity`, `event`, `calendar`, `email`, `mail`, `user`,
  `person`, `team`, `group`, `channel`, `hash`, `tweet`, `mention`, `file`,
  `doc`, `document`, `folder`, `link`, `agent`, `bot`, `database`, `table`
  (anything else falls back to `@`).
- `color`: `neutral`, `blue`, `purple`, `teal`, `green`, `amber`, `rose`.
- Field `tone`: `neutral`, `info`, `success`, `warning`, `danger`.

## A richer preview

The console's generic card (icon, label, description, fields) needs no UI
code. A worker that ships console UI can draw its own:

```tsx
host.mentions?.registerRenderer({
  provider: 'kanban',
  Preview: ({ view, open }) => <TicketCard ticket={view.data} onOpen={open} />,
})
```

If the renderer throws, the generic card is shown instead.

## The token

`@<name>(id="<id>")`, the id written as a JSON string literal, never
following a letter, digit or `_`. Inside markdown code it is literal text.
`token::parse_mentions` / `parse_prose_mentions` read it;
`fixtures/tokens.json` holds the grammar cases the Rust and the console
(`ade/web/src/lib/mentions/token.ts`) parsers both run.
