---
name: iii-directory-function-search
description: >-
  Use when a task needs to find which iii functions to call: one-shot lexical
  function search (directory::search_functions) over installed functions plus
  installable registry workers, and the conditional pre-generate search hint.
type: how-to
---

# Function search

One overall goal plus optional capability queries returns only compact
`function_id` + description candidates, grouped by worker. Choose the needed
ids, then fetch their contracts in one batched `engine::functions::info` call.

## When to Use

- `directory::search_functions`: set `query` to the overall goal. Add up to
  six short, non-overlapping `capabilities` derived from the goal and current
  execution state, omitting work already satisfied. Include every unmet
  external capability once; when one is more precise than the overall
  `query`, include it even if it is the only one. Write `query` and every
  capability in English, translating non-English requests while preserving
  proper names, URLs, and function IDs. Exclude intrinsic reasoning,
  summarization, planning, and formatting; requests to summarize provided
  text or content are ignored. Explicit capabilities are authoritative; when
  the field is absent, the overall `query` provides fallback ranking, with
  commas and "and" providing a deterministic clause fallback. Each capability
  ranks independently and candidates merge
  round-robin. The result contains at most six workers and twelve compact
  candidates, never request schemas or a whole worker surface. Choose the
  smallest needed id set and call `engine::functions::info` once with
  `{ "function_ids": [...] }` before using them. Repeat queries in one
  session omit candidates already delivered.
- The response may also carry an `installable` section: workers from the
  public registry (verified authors only) whose functions match but are NOT
  installed. Those functions are not callable yet — confirm with the user,
  install with `worker::add` (`{ "source": { "kind": "registry", "name":
  "<worker>" }, "wait": false }`, then poll `worker::status`), and only then
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
