---
name: discovery
description: >-
  Use when a task needs to find which iii functions to call: one-shot lexical
  function search (discovery::search_functions), the conditional search hint,
  or the search catalog refresh.
---

# discovery

One natural-language query returns the API reference of only the relevant
functions, grouped by worker — so the next step is calling them directly,
without `engine::functions::list`/`info` round-trips.

## When to Use

- `discovery::search_functions`: pass a `query` naming everything the task
  needs (multi-intent queries are ranked per clause — commas and "and"
  separate capabilities). Returns at most three workers and twelve contracts,
  never a whole worker's surface. Repeat queries in one session omit
  contracts already delivered; after two empty answers the next one widens to
  single-term matches. The result's guidance OVERRIDES the general discovery
  requirement for the listed functions and points at a more specific re-query
  for anything unlisted.
- The pre-generate hint is automatic (configurable: `inject_hint` in the
  `discovery` configuration entry binds/unbinds the hook hot): at most once
  per session, and only when discovery is plausibly needed — it skips when `search_functions` is not in
  the surface, a search result is already in the window, the surface spans
  fewer than `hint_min_workers` workers, the session is already calling real
  functions, or the conversation already names a callable function id.

`discovery::pre-generate`, `discovery::on-functions-change`, and
`discovery::on-config-change` are internal handlers, not direct tools.

## Safety and privacy

A search result adds documentation only; it neither executes a function nor
grants new authority — normal policy and approval still apply. The hook's
transcript rows carry only coarse outcome/reason and counts: no prompts,
messages, tool contracts, arguments, session ids, or timings.
