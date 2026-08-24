---
name: iii-directory-function-search
description: >-
  Use when a task needs to find which iii functions to call: one-shot lexical
  function search (directory::search_functions) over installed functions plus
  installable registry workers, and the conditional pre-generate search hint.
type: how-to
---

# Function search

A required list of one to six unmet external capabilities returns only compact
`function_id` + description candidates, grouped by worker. Choose the needed
ids, then fetch their contracts in one batched `engine::functions::info` call.

## When to Use

- `directory::search_functions`: call once with
  `{ "capabilities": ["<unmet external capability>", "<another>"] }`. Provide
  one to six short, non-overlapping capabilities derived from the goal and
  current execution state, omitting work already satisfied. Include every
  unmet external capability once in the same call. Write every capability in
  English, translating non-English requests while preserving proper names,
  URLs, and function IDs. Exclude intrinsic reasoning,
  summarization, planning, and formatting; requests to summarize provided
  text or content are ignored. Each capability ranks independently and
  candidates merge round-robin. The result contains at most six workers and
  twelve compact candidates, never request schemas or a whole worker surface.
  Choose the smallest needed id set and call `engine::functions::info` once with
  `{ "function_ids": [...] }` before using them. Repeat queries in one
  session omit candidates already delivered.
- The response may also carry an `installable` section: workers from the
  public registry (verified authors only) whose functions match but are NOT
  installed. Those functions are not callable yet — confirm with the user,
  install with `compose::add` (`{ "worker": "<worker>" }`) and wait for that
  call to report the worker ready, and only then
  search again, batch the selected ids through `engine::functions::info`, and
  then call them. The `registry_search` configuration knob turns the section
  off.
- The pre-generate hint is automatic (configurable: `inject_hint` in the
  `iii-directory` configuration entry binds/unbinds the hook hot): at most
  once per turn, and only when discovery is plausibly needed — it skips when
  `search_functions` is not in the surface, a search result is already in the
  current task window, the surface spans fewer than `hint_min_workers`
  workers, the current task is already calling real functions, or it already
  names a callable function id.

`directory::pre-generate`, `directory::on-functions-change`, and
`directory::hint-preview` are internal handlers, not direct tools.

## Safety and privacy

A search result adds candidate metadata only; it neither executes a function
nor grants new authority — normal policy and approval still apply. Suggested
installable workers are vetted only by registry author verification;
installing remains a stack mutation that deserves explicit confirmation. The
hook's transcript rows carry only coarse outcome/reason and counts: no
prompts, messages, tool contracts, arguments, session ids, or timings.
