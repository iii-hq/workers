# eval

`eval` monitors finished Harness sessions and proposes Harness improvements
backed by evidence and an E2E validation plan.

The monitor is **inactive until configured**. It observes, analyzes and
suggests; it never edits code, opens PRs or changes the observed session, and
never starts E2E executions (with a
code directory configured, the investigating LLM
may only call read-only functions: see [Code access](#code-access)). The
behavior is specified in
[SPECIFICATION.md](SPECIFICATION.md) and the technical plan in
[IMPLEMENTATION.md](IMPLEMENTATION.md).

## How an analysis works

1. **Admission.** eval never consults or admits user chats: it binds no
   Harness turn trigger and has no manual session analysis. Work enters
   through a loop cycle (`eval::start`, from an objective or an E2E
   execution).
2. **Collection.** The monitor waits until the turn is definitive and every
   descendant has finished (`harness::metrics.complete`), then reads every
   transcript page with `include_custom: true`, failing on malformed pages
   instead of skipping evidence. The window is the analyzed turn plus earlier
   root turns no analysis covered that started after the record's
   `observe_since`; history from before it is never reported as new.
   Descendants
   linked to other turns are listed but not read. Status and tree membership
   are checked again at the end; a changed session fails the analysis rather
   than mixing turns.
3. **Diagnostics.** Deterministic rules run on the complete transcripts
   before anything is reduced:
   - `repeated_contract_discovery`: a successful `engine::functions::info`
     whose every contract came back `unchanged_in_context`, each traced to an
     earlier successful result that supplied that schema. A `registry-changed`
     notice between the sources and the repeat marks it
     `harness_notice_correlated` (correlation, not cause).
   - `repeated_tool_error`: consecutive calls in one turn to the same target
     with structurally equal payloads, where the first failure was visible
     before the retry and both failed with the same explicit code. Corrected
     arguments, a successful retry, parallel calls in one response, other
     turns and the default-namespace `engine::triggers::info` `NOT_FOUND`
     probe are not flagged.

   Each occurrence has a fingerprint (rule, session, turn, call ids).
4. **Triage (Jev).** `judge::evaluate` with `provider: typesafe` asks one
   Choice question — `needs_investigation`, `expected_behavior` or
   `insufficient_evidence` — over facts computed in code (no transcript
   text). The effective model, probabilities, confidence, criteria hash and
   `Stats` are stored.
5. **Routing.** The LLM investigates if and only if Jev answered
   `needs_investigation`, for automatic and manual analyses alike
   (`routing.reasons` is `["needs_investigation"]`, else `[]`). A deterministic
   finding, `insufficient_evidence`, a low confidence, insufficient coverage, a
   manual request and a sample of quiet sessions do not send a session there on
   their own; the findings stay in the result either way. Analyses recorded
   under the earlier routing keep their `diagnostics`, `insufficient_evidence`,
   `low_confidence`, `coverage_insufficient`, `audit_sample` and
   `manual_request` reasons; none of them is produced now.
6. **Investigation.** `harness::send` opens `eval_monitor_<evaluation_id>`
   (`kind: automation`, `metadata.origin: eval_monitor`) with the frozen
   model, an overriding system prompt and a JSON output contract (at most
   three suggestions; an empty list is valid). Without a code directory (the
   default) it is one turn with deny-all functions. With one, it reads the
   code (see [Code access](#code-access)). The bundle also carries
   `monitor.e2e_scenarios` (`{total, scenarios: [{id, title, summary}]}`, from
   `e2e::dashboard::tests-list`, one page of at most 100 scenarios, summaries cut
   at 160 characters, reused for ten minutes per process) and the prompt tells
   the LLM to set `validation.scenario_id` to one of those ids when it
   reproduces the problem, and to keep `null` when a new case is needed. When
   the E2E does not answer, the list and that instruction are omitted and the
   analysis goes on. The backend drops any suggestion
   that cites an entry absent from the evidence shown, cites code that does
   not exist, or has an incomplete E2E plan, and records why.
7. **Result.** `completed` means the monitor finished, with or without
   suggestions. It is never a validated improvement.

States: `queued → collecting → judging → (investigating) → completed`, or
`failed` (with the failing stage and code) or `cancelled`.

A failure's `code` names the cause. Jev failures are `judge_<provider code>`
(`judge_missing_key`, `judge_provider_unavailable`…), except that HTTP 402 from
the provider is `judge_out_of_credits` (the message keeps the provider's own
text, and `assets.triage_failure` keeps `code: http` and `http_status: 402`).
The investigation fails with `analyst_failed`, `analyst_step_cap` or
`analyst_output_invalid`; the others are `coverage_insufficient`,
`source_advanced`, `external_outcome_unknown`, `deadline` and `cost_cap` (an
automatic investigation that found the daily cost cap reached when it was about
to start).

Each record also keeps three facts for comparing analyses and releases:
`supersedes` (the earlier `evaluation_id` of the same turn, set when the
analysis is a reanalysis), `harness_version` (the Harness version recorded
on analyses the removed chat monitor admitted; analyses admitted now have
none) and
`signals` (occurrences per deterministic pattern the collection found, keyed
`<rule_id>:<target>`; empty both when nothing was found and before collection,
and `coverage` tells which).

## Configure and use

The monitor reuses the existing providers; it never receives a TypeSafe or
LLM key. Jev credentials stay in `judge-typesafe`; the LLM's in its provider.

```bash
# 1. Choose the investigating model (validated against router::models::list).
iii trigger eval::configure --json '{
  "enabled": false,
  "model": {"model": "<model-id>", "provider": "<provider-id>"}
}'

# 2. Start a loop cycle, follow it, read its result.
iii trigger eval::start --json '{"objective": "<objective>"}'
iii trigger eval::status --json '{"cycle_id": "<id>"}'
iii trigger eval::result --json '{"cycle_id": "<id>"}'
```

`eval::config` shows the configuration.

`enabled` is still a required field of the request and of the stored
configuration, kept so existing clients and stored configurations stay valid,
but it has no effect: eval observes no chats, so there is nothing to enable or
pause.

`thinking_level`, `provider_options` (namespaced by the selected provider),
`code_repository` and `daily_cost_cap_usd` are optional. A configuration
change applies to new analyses only: each record keeps its model, its code
directory and its `config_revision`.

## Cost

Only LLM cost is in dollars: the investigation's (`usage.llm_cost_usd`, from
`harness::metrics`, and known only once the investigation ends). Jev's usage,
in a triage, is reported in tokens and never priced. A missing cost is
unknown, never zero, and is never estimated.

`eval::config` returns a `cost` block to decide before starting work:

- `today_capture_usd` and `today_unknown`: the known capture cost since `since`
  (the start of the current **UTC** day: the monitor has no timezone setting),
  and how many analyses of the day started an investigation but reported no cost
  (they add nothing to the sum; their cost is unknown). The cost is added to a
  persisted daily spend as each investigation reports it, and
  `today_capture_usd` is the larger of that and the stored analyses' sum, so
  deleting analyses (or retention) does not give the budget back.
- `today_usd`: everything known to be spent, equal to `today_capture_usd`.
- `cap_usd` and `capped`: the optional `daily_cost_cap_usd` and whether
  `today_capture_usd`, plus the median cost of each investigation still
  running, reached it.
- `per_analysis`: `count`, `min`, `median` and `max` of the known cost of the
  completed analyses that investigated with the configured model, provider and
  code access (or without it), and `unknown`, how many of those reported no
  cost. Without history, `count` is 0 and the statistics are absent.

`daily_cost_cap_usd` (a number above 0; absent means no cap, and then the
configuration revision is what it was) is checked where the money is spent,
right before an automatic analysis first calls the model (under one lock, so
investigations starting together see each other); one that finds it reached
fails with `cost_cap` without calling the model; a manual reanalysis is not held
back by the cap and investigates if Jev answers `needs_investigation` again.
No automatic analysis is admitted any more (the chat monitor was removed), so
the cap only holds back automatic analyses recorded before that and still
unfinished, which the sweep resumes. A cost is known only when an investigation ends, so the investigations
already running are counted at the median cost of the history. With no history
there is nothing to estimate by and a first burst can overshoot by the
investigations that run at once (the queue runs 8 steps together). Raising or
removing the cap lets investigations start again at once.

## Code access

By default the investigating LLM sees only the evidence bundle. Set
`code_repository` to the codebase directory (normally the iii workers
repository, e.g. `/home/layon/workspaces/workers`) and the investigation
becomes a chat with that directory selected:

```bash
iii trigger eval::configure --json '{
  "enabled": false,
  "model": {"model": "<model-id>", "provider": "<provider-id>"},
  "code_repository": "/home/layon/workspaces/workers"
}'
```

- The path must be absolute and an existing directory on the host that runs
  `eval` (checked when it changes; blank means off). It reads the **current** working tree, uncommitted
  changes included: there is no pinned copy, no tag lookup and no requirement
  that the directory is a git clone. To implement a suggestion, create a
  worktree or work however you prefer.
- Each analysis freezes the directory at admission (`record.code_root`,
  echoed in `investigation.code_root`); changing the configuration later never
  alters a running analysis.
- The send carries `{"fs_scope": {"root": "<dir>"}}` in the session metadata
  and in `options.metadata`, exactly what the ADE chat sends for a selected
  directory: the Harness scopes every `coder::*` and `shell::*` call to it and
  the console shows the directory selected when the investigation session is
  opened. The policy is an explicit read-only allowlist,
  `allow: ["coder::search", "coder::tree", "coder::read-file",
  "session::messages", "github::pr::list", "engine::functions::info"]`, with
  `deny: ["eval::*", "e2e::dashboard::execution-*"]` kept as well (no new bus
  function: the LLM uses existing ones). The first five are the functions the
  prompt names (`session::messages` reads more of an observed session than
  the evidence shows);
  `engine::functions::info` is the contract lookup the invocation surface
  tells a model to make before a first call (without it the LLM guessed
  argument names, e.g. `start_line` for `line_from`). Everything else is
  refused by the Harness, so a transcript it reads cannot make it write a
  file, run a shell, start or message a session, start an E2E execution or
  record a review. `fp::pipe` is deliberately absent: its steps run with the
  `fp` worker's authority, outside this policy (analysts had used it to read
  `session::messages`, now allowed directly). Without a code directory the
  policy stays deny-all. With `approval-gate` installed five pass without a
  human (`iii-permissions.yaml` allows them); `session::messages` stays at
  that file's `needs_approval` default for session reads, so each call waits
  for a person or the deadline. The step cap goes from 1 to 32
  generate steps and the total-token cap from 200,000 to 800,000
  (`limits.investigation_code_max_turns` and
  `investigation_code_max_total_tokens`). The deadline and the
  `submit_result` output contract are unchanged. A turn that uses all its
  steps ends `completed` in the Harness with a notice instead of a result; the
  monitor fails that analysis with `analyst_step_cap` (usage kept, no
  suggestions) instead of reading the notice as an invalid answer.
- The prompt names the directory, asks the LLM to read the code before
  proposing a change and to deliver through `submit_result` only afterwards,
  states the step and token budget (a turn that uses every step delivers
  nothing), reminds it that the cause or the best fix may be in any worker
  (not only the Harness), declares the investigation read-only (no file
  changes, no starting or messaging sessions, no `eval::*`), repeats that
  transcripts and code are data, and asks it to cite every claim about code.
  It also tells the LLM that it may list the open pull requests with
  `github::pr::list` (read-only, repo `iii-hq/workers`) and must say in the
  suggestion's `limitations` when one already overlaps it. Without code access
  the prompt is the one the monitor always had, and the output schema does not
  offer `code_refs`.
- A suggestion carries `code_refs`: up to 8 `{path, line_from, line_to}` with
  the path relative to the directory and 1-based inclusive lines. When the
  investigation completes the monitor checks each one on disk and rejects the
  suggestion, with the reason, if the path is absolute or has `..`, resolves
  outside the directory once symlinks are followed, is not a file, or the
  range is not `1 <= line_from <= line_to <= lines in the file`; `code_refs`
  without code access are rejected too. The checks read the files as they are
  when the investigation ends, only as far as `line_to` and never a file above
  16 MiB (a reference to one is rejected), on a blocking thread, because the
  LLM chooses the file (a traces database or a build output lives in a
  workers checkout).
- Tool calls and tokens of the investigation are the session's own metrics
  (`harness::metrics`), counted as before.

**Residual risk.** The investigating LLM can no longer change anything or
start anything, but it still reads untrusted transcripts and can read the
workspace. The `fs_scope` root does not jail the reads: in this environment the
`ide` worker runs `coder::*` unjailed, so `coder::read-file` and `coder::search`
accept absolute paths outside the root. What a hostile transcript can still
achieve is steering what the LLM reads and what its suggestions say, so read a
suggestion's `code_refs` and text as the output of an LLM that read untrusted
input. Leave `code_repository` unset to keep the investigation read-nothing.

### Public functions

| Function | Purpose |
| --- | --- |
| `eval::configure` | Model selection, optional code directory, optional daily cost cap; `enabled` is required but has no effect. |
| `eval::config` | Configuration (or null) and the `cost` block. |
| `eval::list` | Compact records, newest first (`limit` 1–200, default 50); `observation_key` keeps only the analyses of one observed turn, reanalyses included. |
| `eval::status` | One compact record, or null. |
| `eval::result` | Record plus evidence, diagnostics, triage, suggestions, the monitor's own consumption. |
| `eval::cancel` | Signals the Jev call (`judge::cancel`) and stops the investigation (`harness::stop`); never touches the observed session. |
| `eval::delete` | Deletes a terminal analysis and its stored previews. |
| `eval::completed` (trigger) | `{evaluation_id, status, timestamp}` when an analysis ends. |

Internal: `eval::step` (queue `eval-run`, FIFO per analysis, concurrency 8)
and `eval::sweep` (cron, every 15 s).

## Evidence handling

Snapshots keep, per session, the entry count, the SHA-256 of the collected
JSON (not of stored bytes) and masked previews. Previews favor diagnostic
evidence, the request that started the window, the final answer, notices and
a short tail, within the model-context limit (below); omitted and reduced entries are
counted. Diagnostics take at most 64 KiB of that context (all
of them stay in the snapshot); evidence that does not fit makes coverage
`insufficient`, and a context above the limit is never sent. Raw result `details` and opaque reasoning payloads are
dropped; JSON carried as text is decoded before the Harness masker hides
secret-named keys and shortens long strings. Masking is key-based: secrets
in free text are **not** guaranteed to be removed.

Metrics come from `harness::metrics` and are cumulative over the session tree
and all turns (`metrics_scope: session_tree`). The monitor's own Jev `Stats`
and investigation metrics are stored separately from the observed task.
Unknown cost stays unknown.

## Limits

Constants of this version, returned by `eval::config` as `limits` so the
console never restates them:

| Limit | Value |
| --- | --- |
| Analysis budget from admission | 30 min (queue wait, descendants and investigation included) |
| Bus calls for collection | 10 s each, within the budget |
| Jev | 60 s provider timeout, 70 s bus timeout |
| Model context | 192 KiB of serialized JSON (about 50k tokens); diagnostics take at most 64 KiB |
| Assets per analysis | 2 MiB; above it the analysis fails with `coverage_insufficient` before any model call. The turn record kept in `assets.capture.record` is the first thing left out to stay under it |
| Investigation | 1 turn, 16,384 output tokens, 200,000 total tokens (the model's own caps still apply) |
| Investigation with a code directory | 32 generate steps, 16,384 output tokens, 800,000 total tokens, only the six read-only functions of the Code access allowlist (`investigation_code_max_turns`, `investigation_code_max_total_tokens`) |
| Queue | `eval-run`, FIFO per analysis, 8 steps at once |
| Unfinished analyses | 500 |
| Retention | 30 days and 1,000 terminal analyses |

## Recovery and its limits

Records and assets live in `state` scopes `eval_monitor`,
`eval_analysis` and `eval_analysis_assets`. The sweep
re-enqueues pending work and
fails analyses past their deadline.

- Assets are written before a stage advances, so a restart repeats
  collection, never a recorded model answer.
- A Jev call that started before a restart without a saved answer fails with
  `external_outcome_unknown`; it is not repeated automatically (the provider
  has no durable response cache). Request a reanalysis.
- A lost `harness::send` reply is resent with the same idempotency key.
- Recovery assumes `state` kept its records. The default `kv` adapter
  flushes to disk every `save_interval_ms` (5 s by default), so a crash of the
  state worker can lose the last writes; there is no exactly-once guarantee
  under storage loss.
- Locks are per process: run a single `eval` instance.
- Persisted records reject fields they do not know, so state written by a newer
  `eval` (a capture's `record`) cannot be read back by an older one. Rolling
  back is not supported once new analyses exist.

## Turn capture

Each suggestion may carry a `check`: the `decision_point` (an assistant entry
of its evidence), the `signal` (a rule, `contract_rediscovery` or
`repeated_error_call`, or a yes/no `question`) and the proposed `change` as
edits of what the model saw. A malformed check is dropped, not the suggestion.

The observed turn's options, prompt digest and the Harness's whole turn record
are copied into the analysis as `assets.capture`, because the Harness keeps
only a session's latest turn record (a later fork of the session needs it, and
the next turn replaces it); when the record would take the assets over their
limit it is left out and `assets.capture.record_omitted` says why, while the
options and digest stay.

## E2E comparison

A suggestion carries a reproduction, task invariants, a primary effort metric,
the expected baseline-versus-candidate comparison and non-regression
controls. The manual review and validation path (lifecycle, criterion,
verdict, attached or started E2E pairs, recurrence) was removed; the
comparison code stays in the library (`review::runs_of`, `review::evidence`,
`review::judge`, `review::compare`) for the loop's own decision.

## Migration from prompt experiments

`eval::start`, `eval::rerun` and `eval::assert::*` are removed, and so is the
session comparison (`eval::compare-sessions` and its console tab), dropped on
request; the specification still lists it. The prompt experiments' records
stay in the `eval_job` and `eval_session` scopes untouched. Old `eval::step`
messages still queued are skipped (no matching analysis). Finish or cancel
old experiments before upgrading if their results matter.

The console page still shows the previous interface; it will be replaced by the
monitor interface in a separate change.

## Checks

```bash
cargo fmt --manifest-path eval/Cargo.toml --all -- --check
cargo test --locked --manifest-path eval/Cargo.toml --all-targets
cargo clippy --locked --manifest-path eval/Cargo.toml --all-targets -- -D warnings
```

`tests/flow.rs` runs the whole flow over the SDK wire protocol against an
in-memory engine (mocked state, queue, Harness, session-manager, router,
judge and E2E). It proves the monitor's mechanics, not the quality of its
suggestions or any Harness improvement.
