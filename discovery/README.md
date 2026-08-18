# discovery

One-shot lexical function search for agents on the iii bus. One
natural-language query returns the API reference of only the relevant
functions, grouped by worker in rank order — so the model's next step is
calling them directly instead of walking the engine catalog with
`engine::functions::list`/`info` (whose payload scales with the catalog;
search scales with the query).

Extracted from the reflex spike, keeping only the measured winner: pure BM25
ranking. Three independent 5-run live measurements showed every model-consult
stage token-equal to the lexical rank and latency-worse; the rescue mechanism
that actually works is the agent re-querying with different vocabulary. The
full evidence lives in `docs/reflex-discover-findings.md`.

## Functions

| Function | Kind | What it does |
|---|---|---|
| `discovery::search_functions` | public | `{ query }` → `{ guidance, workers[], latency_ms }`: BM25 rank over the live engine catalog, at most 3 workers / 12 contracts. |
| `discovery::pre-generate` | internal hook | Injects the conditional search hint into a harness generation (at most once per session). |
| `discovery::on-functions-change` | internal | Refreshes the search catalog on the engine's functions-available push. |
| `discovery::on-config-change` | internal | Hot-reloads the configuration entry. |

## Ranking pipeline

1. **Corpus**: the live engine catalog (boot snapshot + push refresh),
   slimmed to name + first description sentence + argument names;
   `engine::`/`reflex::`/`discovery::` ids never participate.
2. **Scoring**: Okapi BM25 (k1 1.2, b 0.75) with the function name indexed at
   3× weight, camelCase segmentation (`presignUrl` → presign + url), a
   22-word grammatical stoplist, conservative plural folding
   (values→value, facilities→facility), JSON-key stripping from the query,
   and a two-distinct-terms minimum match.
3. **Multi-intent queries**: clauses split on list punctuation and "and"
   (only when both sides keep two informative terms) are ranked on their own
   leaders with a fair share of the function cap, so one call answers every
   capability.
4. **Pruning**: coverage-aware function floor (≥50% of the leader AND full
   term coverage or ≥85% score) drops same-worker family riders; a
   namespace-level floor (40% of the leader) drops trailing workers.
5. **Session memory** (keyed by caller-supplied OTel baggage, fail-open):
   repeat queries omit contracts already delivered; two consecutive empty
   answers widen the next one to single-term matches.

## The hint

The pre-generate hook appends one `<discovery_assist>` block pointing the
model at `search_functions` — at most once per session, and only when every
gate clears: the function is callable in the surface, no search result is in
the window yet, the surface spans at least `hint_min_workers` distinct
workers, the session has no real function results yet, and the conversation
does not already name a callable function id. Measured on 26 pre-existing e2e
scenarios: an unconditional hint *induces* redundant discovery on guided
tasks (up to +110% tokens); the gates are what make default-on affordable.

## Configuration

Entry `discovery` in the builtin configuration worker:

| Key | Default | Meaning |
|---|---|---|
| `inject_hint` | `true` | Bind the pre-generate hook at all. Off unbinds it hot (no restart): the hint never joins a generation and the model only finds `search_functions` through normal discovery. |
| `hint_min_workers` | `2` | Minimum distinct non-engine workers in the surface before the hint fires; `0` hints everywhere. |

## Console UI

The worker injects a call card for `search_functions` results (meta pills,
query line, per-worker collapsed contract rows) and a one-line transcript
rendering for the hook's `origin.discovery` annotations.
