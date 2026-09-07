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
  fetch `compose::schema { function_id: "compose::add" }`, install with
  `compose::add` using the completion flow below, and only after terminal success
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

## Install with container settings

The returned `install.payload` is a minimal `{ "worker": "<worker>" }` shorthand.
If configuration is needed and the running daemon's schema supports objects, replace
that singular field with `workers`:

```json
{
  "operation_id": "<operation-id>",
  "workers": [
    "state",
    {
      "worker": "<worker>",
      "start_after": ["state"],
      "config_override": { "<config-key>": "<value>" }
    }
  ]
}
```

Review `directory::registry::workers::info` for the worker's configuration before
choosing values. Objects also accept package `version`, `config_name`, `scripts`
(`pre_run`, `pre_run_timeout`, `run`, `post_run`), `working_dir`, `environment`
(string values), `env_file`, and `startup_timeout`. Use `scripts`, not `script`;
`scripts.run` is valid only for local workers. `worker` accepts local directories
and `path://` sources as well as packages. Relative paths resolve from the compose
file. `working_dir` sets where the process and hooks run; its default is the local
worker directory, or the compose directory for packages.

Omitted settings stay in place. A supplied map replaces the whole field, including
`scripts`, `environment`, and `config_override`; use `{}` or `[]` to clear maps
or lists. `start_after` names container keys and includes required package dependencies.
An unversioned package resolves the latest matching version; use an explicit version
to keep it pinned. Settings changes can restart a running worker.

Before installation, choose a unique operation ID and register a one-shot wake with
`engine::register_trigger { trigger_type: "compose-operation", config: {
operation_id: "<operation-id>", terminal_only: true }, once: true }`.
Keep the subscription ID and pass the same `operation_id` to `compose::add`.
Read `compose::operation { operation_id: "<operation-id>" }` once for race recovery.
If terminal, unregister the wake and inspect the result; otherwise end the turn with
the wake armed. Do not poll. Acceptance is not readiness, and a terminal event can
report failure. Search again and fetch function contracts only after terminal success.
Under the harness, omit `namespace` and `file`; it supplies the Compose scope.

`directory::pre-generate`, `directory::on-functions-change`, and
`directory::hint-preview` are internal handlers, not direct tools.

## Safety and privacy

A search result adds candidate metadata only; it neither executes a function
nor grants new authority — normal policy and approval still apply. Suggested
installable workers are vetted only by registry author verification;
installing remains a stack mutation that deserves explicit confirmation. The
hook's transcript rows carry only coarse outcome/reason and counts: no
prompts, messages, tool contracts, arguments, session ids, or timings.
