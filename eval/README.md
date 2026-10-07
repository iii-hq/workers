# eval

`eval` monitors finished Harness sessions and proposes Harness improvements
backed by evidence and an E2E validation plan.

The monitor is **inactive until configured**. It observes, analyzes and
suggests; it never edits code, opens PRs or changes the observed session, and
starts E2E executions only when a person calls `eval::start-validation` (with a
code directory configured, the investigating LLM
may only call read-only functions: see [Code access](#code-access)). The
behavior is specified in
[SPECIFICATION.md](SPECIFICATION.md) and the technical plan in
[IMPLEMENTATION.md](IMPLEMENTATION.md).

## How an analysis works

1. **Admission.** A terminal `harness::turn-completed` of a root session (or
   a manual `eval::analyze-session`) admits one analysis per session turn.
   Automatic observation admits only the user's console chats. From
   `session::get` the session must have kind `user` (a record without a kind
   counts as `user`), `metadata.surface` equal to `console` (the console writes
   it on every send) and no `metadata` key starting with `e2e_`. Everything
   else answers `not_user_chat` and can still be analyzed by hand: E2E runs
   (kind `e2e`, or `e2e_*` metadata on the ones recorded before the kind
   existed), automations (sentinel investigations and the monitor's own, which
   it opens as `automation`; the console stamps `console` on those too, so the
   kind is what excludes them), scripted and sub-agent sessions (kind `user`
   but no surface) and sessions whose record cannot be read. Progress events,
   descendants and the monitor's own sessions are never admitted; a
   redelivered event returns the existing analysis.
2. **Collection.** The monitor waits until the turn is definitive and every
   descendant has finished (`harness::metrics.complete`), then reads every
   transcript page with `include_custom: true`, failing on malformed pages
   instead of skipping evidence. The window is the observed turn plus earlier
   root turns no analysis covered that started while the monitor was already
   observing (a wake continuation); history from before it is never reported
   as new. Descendants
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
analysis is a reanalysis), `harness_version` (the version of this worker's
Harness that `engine::workers::list` reported when a live event admitted the
analysis, read from the engine's `default` namespace: the turn just ended on
it. A manual analysis, which may be of an older session, has none, and neither
has one admitted while the engine did not answer, which never blocks admission) and
`signals` (occurrences per deterministic pattern the collection found, keyed
`<rule_id>:<target>`; empty both when nothing was found and before collection,
and `coverage` tells which).

## Configure and use

The monitor reuses the existing providers; it never receives a TypeSafe or
LLM key. Jev credentials stay in `judge-typesafe`; the LLM's in its provider.

```bash
# 1. Choose the investigating model (validated against router::models::list)
#    and keep it paused.
iii trigger eval::configure --json '{
  "enabled": false,
  "model": {"model": "<model-id>", "provider": "<provider-id>"}
}'

# 2. Analyze one finished root session by hand (works while paused).
iii trigger eval::analyze-session --json '{"session_id": "<session>"}'

# 3. Follow it, then read evidence, triage and suggestions.
iii trigger eval::status --json '{"evaluation_id": "<id>"}'
iii trigger eval::result --json '{"evaluation_id": "<id>"}'

# 4. Enable automatic observation once the manual results look right.
iii trigger eval::configure --json '{
  "enabled": true,
  "model": {"model": "<model-id>", "provider": "<provider-id>"}
}'
```

`eval::config` shows the configuration and whether the
`harness::turn-completed` binding is active (`observer_bound`); when it is
false, automatic observation is unavailable even if enabled.

`thinking_level`, `provider_options` (namespaced by the selected provider),
`code_repository` and `daily_cost_cap_usd` are optional. A configuration
change applies to new analyses only: each record keeps its model, its code
directory and its `config_revision`.

## Cost

Only LLM cost is in dollars: the investigation's (`usage.llm_cost_usd`, from
`harness::metrics`, and known only once the investigation ends) and the
`eval::reproduce` samples' (`cost_usd` of each reply). Jev's usage, in a
triage or in classifying a replay's replies, is reported in tokens and never
priced. A missing cost is unknown, never zero, and is never estimated: a
replay sample that answered without a cost adds nothing to the sums, is counted
in the reproduction's `cost_unknown_samples` (which makes its `cost_usd` a lower
bound) and in the day's `today_replay_unknown`.

The day's spend has two buckets. **Capture** is what an analysis spends (the
investigation; Jev's triage is in tokens). **Replay** is what `eval::reproduce`
spends. Only capture is compared with the daily cap, so a manual replay never
stops automatic observation. The spend persisted before the buckets (one total)
counts as capture for its day.

`eval::config` returns a `cost` block to decide before enabling:

- `today_capture_usd` and `today_unknown`: the known capture cost since `since`
  (the start of the current **UTC** day: the monitor has no timezone setting),
  and how many analyses of the day started an investigation but reported no cost
  (they add nothing to the sum; their cost is unknown). The cost is added to a
  persisted daily spend as each investigation reports it, and
  `today_capture_usd` is the larger of that and the stored analyses' sum, so
  deleting analyses (or retention) does not give the budget back.
- `today_replay_usd` and `today_replay_unknown`: the replay bucket, the known
  cost of the day's `eval::reproduce` samples and how many replied without a
  cost. Reported, never capped.
- `today_usd`: both buckets together, everything known to be spent. **The cap
  does not compare it.**
- `cap_usd` and `capped`: the optional `daily_cost_cap_usd` and whether
  `today_capture_usd`, plus the median cost of each investigation still
  running, reached it.
- `per_analysis`: `count`, `min`, `median` and `max` of the known cost of the
  completed analyses that investigated with the configured model, provider and
  code access (or without it), and `unknown`, how many of those reported no
  cost. Without history, `count` is 0 and the statistics are absent.

`daily_cost_cap_usd` (a number above 0; absent means no cap, and then the
configuration revision is what it was) pauses **automatic** observation for the
rest of the UTC day once `today_capture_usd` reaches it: the turn is not admitted,
`eval::on-turn-completed` answers `cost_cap`, and `eval::config`'s
`last_rejection` records the turn with `reason: cost_cap` (`at_capacity` for the
unfinished-analyses cap). A manual `eval::analyze-session` is never refused by
the cap, and a redelivered event of an already admitted turn is still reused.
The cap is checked twice: at admission and again where the money is spent,
right before an automatic analysis first calls the model (under one lock, so
investigations starting together see each other); one that finds it reached
fails with `cost_cap` without calling the model; a manual reanalysis is not held
back by the cap and investigates if Jev answers `needs_investigation` again. A cost is known only when an investigation ends, so the investigations
already running are counted at the median cost of the history. With no history
there is nothing to estimate by and a first burst can overshoot by the
investigations that run at once (the queue runs 8 steps together). Raising or
removing the cap resumes observation at once.

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
  `eval` (checked when it changes; blank means off). The monitor stays
  disabled by default. It reads the **current** working tree, uncommitted
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
  "github::pr::list", "engine::functions::info"]`, with `deny: ["eval::*",
  "e2e::dashboard::execution-*"]` kept as well (no new bus function: the LLM
  uses existing ones). The first four are the functions the prompt names;
  `engine::functions::info` is the contract lookup the invocation surface
  tells a model to make before a first call (without it the LLM guessed
  argument names, e.g. `start_line` for `line_from`). Everything else is
  refused by the Harness, so a transcript it reads cannot make it write a
  file, run a shell, start or message a session, start an E2E execution or
  record a review. `fp::pipe` is deliberately absent: its steps run with the
  `fp` worker's authority, outside this policy. Without a code directory the
  policy stays deny-all. With `approval-gate` installed the coder functions
  and `engine::functions::info` pass without a human (`iii-permissions.yaml`
  allows them); `github::pr::list` is not listed there, so a held call would
  stall the investigation until its deadline. The step cap goes from 1 to 32
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
| `eval::configure` | Model selection, optional code directory, optional daily cost cap and enable/pause. |
| `eval::config` | Configuration (or null), observer status, the latest rejection and the `cost` block. |
| `eval::analyze-session` | Manual analysis; `reanalyze: true` creates another analysis after the previous one ended. |
| `eval::list` | Compact records, newest first (`limit` 1–200, default 50); `observation_key` keeps only the analyses of one observed turn, reanalyses included. |
| `eval::status` | One compact record, or null. |
| `eval::result` | Record plus evidence, diagnostics, triage, suggestions, the monitor's own consumption, E2E links and one review row per suggestion (`reviews`). |
| `eval::cancel` | Signals the Jev call (`judge::cancel`) and stops the investigation (`harness::stop`); never touches the observed session. |
| `eval::delete` | Deletes a terminal analysis; the turn stays marked as analyzed until retention. |
| `eval::attach-validation` | Links baseline and candidate E2E executions to a suggestion and computes the pair's evidence (`dry_run: true` only looks them up). |
| `eval::start-validation` | Explicitly starts a baseline and a candidate E2E execution (Docker) of one scenario, pinned to two pushed commits. Spends model tokens. |
| `eval::review` | `set_lifecycle`, `set_criterion` or `set_verdict` on one suggestion of a terminal analysis. |
| `eval::reviews` | The stored review rows and, per analysis, its suggestions counted by lifecycle status. |
| `eval::recurrence` | For a suggestion shipped in a version, how often its patterns appeared per analysis before and from that version. |
| `eval::reproduce` | Replays a suggestion's decision point: rebuilds the request the model received at that step (optionally edited), samples the next reply N times without running any function and reads the signal in each. `dry_run` only rebuilds and checks fidelity. See [VALIDATION.md](VALIDATION.md). |
| `eval::completed` (trigger) | `{evaluation_id, status, timestamp}` when an analysis ends. |

Internal: `eval::step` (queue `eval-run`, FIFO per analysis, concurrency 8),
`eval::on-turn-completed` and `eval::sweep` (cron, every 15 s).

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
| Investigation with a code directory | 32 generate steps, 16,384 output tokens, 800,000 total tokens, every function allowed but `eval::*` and `e2e::dashboard::execution-*` (`investigation_code_max_turns`, `investigation_code_max_total_tokens`) |
| Queue | `eval-run`, FIFO per analysis, 8 steps at once |
| Unfinished analyses | 500 |
| Retention | 30 days and 1,000 terminal analyses |

## Recovery and its limits

Records, indexes and assets live in `state` scopes `eval_monitor`,
`eval_observation`, `eval_analysis` and `eval_analysis_assets`; the review rows
in `eval_suggestion` (below). A record is
saved before its index; startup reconciles missing indexes before the
observation trigger is bound, and the sweep re-enqueues pending work and
fails analyses past their deadline.

- Assets are written before a stage advances, so a restart repeats
  collection, never a recorded model answer.
- A Jev call that started before a restart without a saved answer fails with
  `external_outcome_unknown`; it is not repeated automatically (the provider
  has no durable response cache). Request a reanalysis.
- A lost `harness::send` reply is resent with the same idempotency key.
- Events emitted while `eval` was offline are not recovered; analyze those
  sessions manually.
- Recovery assumes `state` kept its records. The default `kv` adapter
  flushes to disk every `save_interval_ms` (5 s by default), so a crash of the
  state worker can lose the last writes; there is no exactly-once guarantee
  under storage loss.
- Locks are per process: run a single `eval` instance.

## Validation by replay

The first check of a suggestion is a replay of the step where its behavior
happened ([VALIDATION.md](VALIDATION.md)). Each suggestion carries a `check`:
the `decision_point` (an assistant entry of its evidence), the `signal` (a rule,
`contract_rediscovery` or `repeated_error_call`, or a yes/no `question` Jev
answers per reply) and the proposed `change` as edits of what the model saw.
Older suggestions have none; `eval::reproduce` then takes `check` in the request.

`eval::reproduce` rebuilds the request the Harness sent at that step: the
window from the durable log (`harness::window::build`, notices included), the
frozen runtime context, the turn's system prompt and skills baseline (copied
into the analysis as `assets.capture`, because the Harness keeps only a
session's latest turn record), the `agent_trigger` tool and `context::assemble`.
`assets.capture.record` also keeps the Harness's whole turn record as read (a
later fork of the session needs it, and the next turn replaces it); when it
would take the assets over their limit it is left out and
`assets.capture.record_omitted` says why, while the options and digest above
stay.
It counts the result with `router::count_tokens` (the context manager's estimate
when the provider has no counter) against the recorded usage: `exact` when the
difference equals the fixed overhead measured at the turn's first step,
`approximate` otherwise, with the reasons. It then samples `router::complete`
(one warm-up call, then four at a time), never runs a function, reads each
reply's signal and stores everything in the suggestion's review row
(`reproductions[]`). `extend` adds replies; `samples: 0` finishes one that
failed. A restart marks running replays `failed` (interrupted). Not supported
yet: native function exposure, output contracts, and windows the context
manager would prune or compact.

## E2E validation

A suggestion carries a reproduction, task invariants, a primary effort metric,
the expected baseline-versus-candidate comparison and non-regression
controls. Turning it into proof is a separate, explicit step: fix the
scenario version and criteria, run baseline and candidate with
`harness-e2e`, then link both executions:

```bash
iii trigger eval::attach-validation --json '{
  "evaluation_id": "<id>", "suggestion_index": 0,
  "baseline_execution_id": "<baseline>", "candidate_execution_id": "<candidate>"
}'
```

The link stores what `e2e::dashboard::execution-get` reports (status, report
availability, evidence errors). Attaching the same pair again refreshes its link
but keeps the first `attached_at`, which still says when results first existed. Each scenario keeps its `run_count` and
`measures` (the execution's own averages) and, when
`plan_execution.measurements.cohorts[]` has exactly one cohort for it, that
cohort's own aggregates: `pass_rate`, `mean_score`, `completed_runs`,
`planned_runs`, `cost_usd_per_run` and `total_tokens_per_run` (the cohort's
totals divided by its completed runs; the totals also cover failed attempts),
`median_wall_time_ms` and `p50_function_calls`. Each is absent when the E2E did
not report it (a count may arrive as a float), never zero. It is a reference,
not a verdict: improvement,
no improvement, regression or inconclusive belong to the E2E comparison and its
criteria.

## Review and validation

What people decide about a suggestion lives in its own row, in the
`eval_suggestion` state scope keyed `<evaluation_id>:<suggestion_index>`
(`SuggestionReviewV1`), never in the suggestion, whose schema is what the
investigating LLM must return. The row copies the suggestion's `title`,
`harness_component`, plan `scenario_id` and its `patterns` (`<rule_id>:<target>`
of the diagnostics the suggestion's evidence cites), so it outlives the
analysis's retention and `eval::delete`. A suggestion nobody acted on has no
stored row and reads as `new` in `eval::result.reviews` (one row per
suggestion) with `lifecycle.history: []`.

`eval::review {evaluation_id, suggestion_index, action, ...}` needs the
analysis to be terminal and the suggestion to exist. The author of every change
is `by` when the request names one, else the user the eval worker runs as
(`$USER`, else `$LOGNAME`), else the caller's identity (`_caller_worker_id`):
a browser's worker id is an opaque uuid, so it comes last. The console sends no
`by`, so what it records is the host's user.

- **`set_lifecycle`** with `status` `accepted`, `in_progress`, `shipped`,
  `rejected` or `duplicate` (never back to `new`; a shipped suggestion stays
  shipped, to complete its `pr` or `version`). `shipped` needs a `pr` (kept from
  an earlier status when omitted) and may carry the `version` it shipped in, a
  semantic version like `1.8.43`; `rejected` needs a `reason`; `duplicate`
  needs `duplicate_of`. `lifecycle.history` keeps `{status, at, by, note?}` for
  every change.
- **`set_criterion`** registers what a validation is judged by, before its
  results: a `metric` (`signal_per_run` with one of the row's `patterns`,
  `pass_rate`, `cost_usd`, `duration`, `tokens` or `function_calls`), a
  `direction`, a `min_effect` (the smallest difference between the baseline and
  candidate means that counts, in the metric's own unit: signals per run, USD,
  seconds of wall time, function calls or tokens; percentage points, 0-100, for
  `pass_rate`. `1` is one signal per run, never 1%) and `min_runs` (1-20
  completed runs each side), on the plan's scenario unless `scenario_id` names
  another. The same criterion again keeps its registration time. Once a pair
  is attached or a run started, an existing criterion is frozen
  (`criterion_frozen:`): one chosen after seeing results proves nothing.
- **`set_verdict`** records the person's `outcome` (`validated_improvement`,
  `no_improvement`, `regression` or `inconclusive`), `rationale` and
  `controls_checked`, with a snapshot of the criterion. `validated_improvement`
  is refused (`verdict_refused:`) unless a criterion was registered no later than
  the first moment results could exist (the earliest of each attached
  execution's own `started_at`, the attach, and the executions this worker
  started, also those a restart replaced), so an old pair attached after its
  criterion does not count as unseen, and the evidence has at least `min_runs`
  measured runs on both sides. The computed outcome (below) is not
  a precondition: the person may disagree, and both are kept.

### Evidence

Whenever a pair is attached (by hand or by a started run) and the suggestion's
scenario is known (the criterion's, else the plan's), the runs of that scenario
in both executions (`detail.reports[].report.scenarios[].runs[]`) become the
row's `evidence`: per run its `completed` flag (the E2E's `completion` is
`completed` and `technical` is `valid`, both kept as reported, so infrastructure
failures and incomplete runs are listed, never silently dropped), `status`,
cost, wall time, tokens, function calls, and the occurrences of each of the
row's patterns that `diagnostics::detect` finds in the run's own transcript
(`transcript.messages`; a run without transcript has no count, 0 is a
measurement). Per side `n` is the measured runs with a value of the criterion's
metric and `mean` their mean. A run is measured when its `technical` is `valid`:
one the agent left unfinished (`completion: task_incomplete`) is the Harness's
result, so it counts, as a fail of the pass rate whatever its `status` says,
while an infrastructure failure measured nothing and stays out. `computed_outcome` is decided in code, with a
`reason`: `validated_improvement` when the candidate moves the baseline mean by
at least `min_effect` (in its unit) in the wanted direction, `regression` when it
moves the wrong way by that much, `no_improvement` otherwise, and `inconclusive`
when there is no criterion, a side has no run of the scenario, more than half of
a side's runs did not complete (technical failures and unfinished runs) or a
side has fewer than `min_runs` measured runs with the metric. Only the root
session of each run is
read (a Docker run's sessions never reach this engine); every metric but
`signal_per_run` is a mean over the same runs, not the E2E cohort aggregate.

### Starting a validation

`eval::start-validation {evaluation_id, suggestion_index, scenario_id,
candidate_ref, baseline_ref?, runs?, model, provider, criterion}` replaces the
manual steps of linking two E2E runs. It runs only when called: it spends model
tokens and Docker time on two executions whose cost depends on the scenario and
`runs` (a past cohort of the same scenario in the E2E is the best estimate).

With `dry_run: true` (default `false`) it stops after the checks that need no
E2E: the refs resolve (step 1), both commits are pushed and differ, and the
scenario and runs are valid (and `criterion.min_runs <= runs`, when a criterion
is sent; `model`, `provider` and `criterion` are otherwise only required for a
real start). It answers `{baseline, candidate, warnings}`, each side
`{commit, short, branch}` (`short` is 12 characters, `branch` a remote branch
that holds the commit, the one named like the ref when several do), and
registers nothing, records no run and calls no E2E function. A baseline given
that is not an ancestor of the candidate is a warning, not an error. The
console asks it 400 ms after the last keystroke and when a field loses focus,
shows `Resolved · <sha> · pushed to <branch>` or the refusal under each field,
and keeps Start disabled until a dry run succeeded for exactly the current
refs, scenario and runs.

1. Needs `code_repository` configured (`code_repository_required:`). The refs
   are resolved with `git` in it: `candidate_ref` and `baseline_ref` may be a
   plain branch, tag or commit name; the baseline defaults to the merge base of
   the candidate and `origin/main`. Each commit must be on a remote branch of
   the clone (`git branch -r --contains`, nothing is fetched: run `git fetch`
   after a push) because the E2E builds the Harness from GitHub. Errors:
   `git_unavailable:`, `git_ref_invalid:` (also when both resolve to the same
   commit), `commit_not_pushed:`.
2. Each stack is the YAML of the E2E's stack built on the `harness` template
   (`e2e::dashboard::stacks-list`) with only its `harness` container replaced by
   `{worker: package://harness, commit: <sha>, repository: iii-hq/workers}`.
   Everything else, including the `latest` versions the base declares, is the
   E2E's own (a comparability check on the attached pair shows when two
   executions resolved different ones).
3. Nothing changed so far; now the criterion is registered and the run recorded
   as `starting` (`run` in the row: both commits, scenario, runs, model,
   provider, `started_at`), and two executions start with
   `e2e::dashboard::execution-start` (`where: docker`, one technical retry,
   `runs` 1-20, default 5, label `eval <evaluation_id> S<n> baseline|candidate`
   with `n` the suggestion number from 1). The row goes to `running` with both
   execution ids. A refusal is `e2e_busy:` or `e2e_unavailable:`; the run is
   then `failed` with the error, naming the baseline execution when it had
   already started (cancel it in the E2E). A start the E2E never answers
   (`e2e_start_unconfirmed:`, after 30 s) may still have started an execution,
   so the run stays `starting`, with the baseline id when it had one, and no
   second start can duplicate it: look for the label in the E2E; the sweep fails
   the run after 5 minutes. One run per suggestion at a time.
4. The sweep (every 15 s) polls `e2e::dashboard::executions-list {ids}` for
   running runs. When both executions are `completed`, `interrupted`,
   `cancelled` or `failed` the run is `finished` and the pair is attached with
   the evidence computed (`attached`), also when the executions ended badly: the
   evidence then says why it cannot judge. An E2E that does not answer leaves it
   `finished` with the error and the next sweep tries again, for 30 minutes
   after both ended (`finished_at`), then it is `failed` with `attach_gave_up:`
   and the last error, and the pair can be attached by hand or started again; an
   execution the E2E no longer lists, or an analysis deleted meanwhile, makes it
   `failed` at once. A `starting` run older than 5 minutes (a restart between the
   request and its answer, or an E2E that never answered) is `failed`. Startup
   recovery does not wait on the E2E: the first sweep, 15 s later, does.

### After the release

`eval::recurrence {evaluation_id, suggestion_index}` for a suggestion marked
`shipped` with a `version` returns, per row pattern, the occurrences, the
analyses with at least one and the occurrences per analysis for the analyses on
Harness versions before that version (`before`) and from it on
(`from_version`). Only analyses whose collection finished count (`signals` is
empty both when nothing was found and before collection), and analyses without a
semantic `harness_version` are left out and counted in `without_version`: the
manual ones, among them every reanalysis, so one turn is never counted twice.
Analyses are kept 30 days, so the earlier window is bounded by retention.

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
# Replay the detectors over exported session::messages entries:
cargo run --manifest-path eval/Cargo.toml --example replay -- <session>=<entries.json>
```

`tests/flow.rs` runs the whole flow over the SDK wire protocol against an
in-memory engine (mocked state, queue, Harness, session-manager, router,
judge and E2E). It proves the monitor's mechanics, not the quality of its
suggestions or any Harness improvement.
