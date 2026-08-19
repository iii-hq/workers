---
name: iii-directory-function-search
description: >-
  Use when a task needs to find which iii functions to call: one-shot lexical
  function search (directory::search_functions) over installed functions plus
  installable registry workers, and the conditional pre-generate search hint.
type: how-to
---

# Function search

One natural-language query returns the API reference of only the relevant
functions, grouped by worker — so the next step is calling them directly,
without `engine::functions::list`/`info` round-trips.

## When to Use

- `directory::search_functions`: pass a `query` naming everything the task
  needs (multi-intent queries are ranked per clause — commas and "and"
  separate capabilities). Returns at most three workers and twelve contracts,
  never a whole worker's surface. Repeat queries in one session omit
  contracts already delivered; after two empty answers the next one widens to
  single-term matches. The result's guidance OVERRIDES the general discovery
  requirement for the listed functions and points at a more specific re-query
  for anything unlisted.
- The response may also carry an `installable` section: workers from the
  public registry (verified authors only) whose functions match but are NOT
  installed. Those functions are not callable yet — confirm with the user,
  install with `worker::add` (`{ "source": { "kind": "registry", "name":
  "<worker>" }, "wait": false }`, then poll `worker::status`), and only then
  call them. The `registry_search` configuration knob turns the section off.
- The pre-generate hint is automatic (configurable: `inject_hint` in the
  `iii-directory` configuration entry binds/unbinds the hook hot): at most
  once per session, and only when discovery is plausibly needed — it skips
  when `search_functions` is not in the surface, a search result is already
  in the window, the surface spans fewer than `hint_min_workers` workers, the
  current task is already calling real functions, or it already names a
  callable function id.

`directory::pre-generate`, `directory::on-functions-change`, and
`directory::hint-preview` are internal handlers, not direct tools.

## Safety and privacy

A search result adds documentation only; it neither executes a function nor
grants new authority — normal policy and approval still apply. Suggested
installable workers are vetted only by registry author verification;
installing remains a stack mutation that deserves explicit confirmation. The
hook's transcript rows carry only coarse outcome/reason and counts: no
prompts, messages, tool contracts, arguments, session ids, or timings.
