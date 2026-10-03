# eval

`eval` monitors finished Harness sessions and proposes Harness improvements
backed by evidence and an E2E validation plan.

The monitor is **inactive until configured**. It observes, analyzes and
suggests; it never edits code, opens PRs, changes the observed session or
starts E2E campaigns (with a code directory configured, the investigating LLM
is only *told* to stay read-only: see [Code access](#code-access)). The
behavior is specified in
[SPECIFICATION.md](SPECIFICATION.md) and the technical plan in
[IMPLEMENTATION.md](IMPLEMENTATION.md).

## How an analysis works

1. **Admission.** A terminal `harness::turn-completed` of a root session (or
   a manual `eval::analyze-session`) admits one analysis per session turn.
   Progress events, descendants and the monitor's own sessions are never
   admitted; a redelivered event returns the existing analysis.
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
5. **Routing.** The LLM investigates when there is a diagnostic, Jev answers
   `needs_investigation` or `insufficient_evidence`, confidence is below 0.8,
   coverage is insufficient, the request is manual, or the observation falls
   in the stable 5% audit sample (`sha256(observation) mod 100 < 5`). The
   threshold and the sample rate are initial hypotheses, not measured
   accuracy.
6. **Investigation.** `harness::send` opens `eval_monitor_<evaluation_id>`
   (`kind: automation`, `metadata.origin: eval_monitor`) with the frozen
   model, an overriding system prompt and a JSON output contract (at most
   three suggestions; an empty list is valid). Without a code directory (the
   default) it is one turn with deny-all functions. With one, it reads the
   code (see [Code access](#code-access)). The backend drops any suggestion
   that cites an entry absent from the evidence shown, cites code that does
   not exist, or has an incomplete E2E plan, and records why.
7. **Result.** `completed` means the monitor finished, with or without
   suggestions. It is never a validated improvement.

States: `queued → collecting → judging → (investigating) → completed`, or
`failed` (with the failing stage and code) or `cancelled`.

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

`thinking_level`, `provider_options` (namespaced by the selected provider)
and `code_repository` are optional. A configuration change applies to new
analyses only: each record keeps its model, its code directory and its
`config_revision`.

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
  opened. The policy is `allow: ["*"]` (no new bus function: the LLM uses the
  existing functions, and the prompt points it at `coder::search`,
  `coder::tree` and `coder::read-file`, which stacks with `approval-gate`
  allow without a human; the shell stays callable but a held call would stall
  the investigation until its deadline), the step cap goes from 1 to 32
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
  Without code access the prompt is the one the monitor always had, and the
  output schema does not offer `code_refs`.
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

**Residual risk.** With the function restriction lifted the investigating LLM
may call *any* function, including ones that change files, start or message
sessions or call `eval::*`, while it reads untrusted transcripts. The guards
are the prompt rules and the `fs_scope` root; in this environment the `ide`
worker runs `coder::*` unjailed, so absolute paths are not contained by the
root. This was chosen "for now"; no other restriction is added. Leave
`code_repository` unset to keep the investigation read-nothing.

### Public functions

| Function | Purpose |
| --- | --- |
| `eval::configure` | Model selection, optional code directory and enable/pause. |
| `eval::config` | Configuration (or null) and observer status. |
| `eval::analyze-session` | Manual analysis; `reanalyze: true` creates another analysis after the previous one ended. |
| `eval::list` | Compact records, newest first (`limit` 1–200, default 50). |
| `eval::status` | One compact record, or null. |
| `eval::result` | Record plus evidence, diagnostics, triage, suggestions, the monitor's own consumption and E2E links. |
| `eval::cancel` | Signals the Jev call (`judge::cancel`) and stops the investigation (`harness::stop`); never touches the observed session. |
| `eval::delete` | Deletes a terminal analysis; the turn stays marked as analyzed until retention. |
| `eval::attach-validation` | Links baseline and candidate E2E executions to a suggestion (`dry_run: true` only looks them up). |
| `eval::propose-validation` | Asks Jev which existing E2E pair fits a suggestion; attaches nothing. |
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
| Assets per analysis | 2 MiB; above it the analysis fails with `coverage_insufficient` before any model call |
| Investigation | 1 turn, 16,384 output tokens, 200,000 total tokens (the model's own caps still apply) |
| Investigation with a code directory | 32 generate steps, 16,384 output tokens, 800,000 total tokens, every function allowed (`investigation_code_max_turns`, `investigation_code_max_total_tokens`) |
| Queue | `eval-run`, FIFO per analysis, 8 steps at once |
| Unfinished analyses | 500 |
| Retention | 30 days and 1,000 terminal analyses |

## Recovery and its limits

Records, indexes and assets live in `state` scopes `eval_monitor`,
`eval_observation`, `eval_analysis` and `eval_analysis_assets`. A record is
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
availability, evidence errors). It is a reference, not a verdict: improvement,
no improvement, regression or inconclusive belong to the E2E comparison and its
criteria.

### Proposing the pair with Jev

`eval::propose-validation {evaluation_id, suggestion_index}` (the console's
"Fill with Jev") reads `e2e::dashboard::executions-list` (the 100 executions
the E2E keeps) and decides in code which ordered pairs may be offered:

- only `passed` or `failed` runs; every other status, runs without the plan's
  scenario (`other_scenario`) and entries without an id are counted in
  `excluded`;
- same known model and provider, and the same case: both include the plan's
  scenario, or, when the plan names none, both ran the same scenario set;
- identical recorded stacks are kept (a change in an uncommitted build is not
  in the record); each pair instead carries the stack difference computed in
  code, e.g. `recorded stack differs: harness 1.8.42·f3a49e1 → 1.8.43·00c21f5`;
- the 60 most recent pairs (by their older run); the rest is `pairs_dropped`.

Without a pair, Jev is not called (`outcome: no_comparable_pair`). Otherwise
one `judge::evaluate` call (provider `typesafe`) gets the suggestion, the plan
and the runs the pairs refer to, with one Choice over the pairs plus `none`.
The answer is `proposed` (`proposal.baseline_execution_id`,
`candidate_execution_id`, `confidence`, `low_confidence` below 0.8) or
`none_fits`, plus up to three `alternatives` (other offered pairs with at
least 5% of Jev's probability). Confidence describes Jev's choice among the offered pairs, not
whether the change works: the person checks the runs and attaches them with
`eval::attach-validation`. The call's tokens are added to the analysis's
`usage` (also when Jev answers an error). Errors carry stable prefixes:
`e2e_unavailable:`, `jev_unavailable:` (with the provider's explanation, such
as an HTTP 402 billing message) and `jev_invalid_response:`.

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
