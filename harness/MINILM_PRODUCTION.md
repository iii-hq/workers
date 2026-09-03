# Local MiniLM production stack

`worker-compose.minilm-production.yaml` starts every worker declared by a
top-level `iii.worker.yaml` in this repository. The directory worker runs the
hybrid production search pipeline on CPU:

1. BM25 and `sentence-transformers/all-MiniLM-L6-v2` retrieve against the full
   installed function catalog.
2. Reciprocal-rank fusion combines both retrieval orders.
3. `cross-encoder/ms-marco-MiniLM-L6-v2` reranks the top 48 fused candidates;
   the tail keeps its fused retrieval order below them.
4. A second reciprocal-rank fusion anchors reranker order to retrieval.

No hosted embedding or LLM API is used by function search. Exact function IDs
bypass both models. Degradation is staged: if the embedding index is
unavailable, the query falls back to BM25; if only the cross-encoder fails,
returns invalid output, or exceeds its 3 s budget, the query keeps the fused
BM25+MiniLM retrieval order without reranking.

MiniLM retrieves and orders candidates; it does not by itself decide that no
installed function supports a natural-language capability. The stack therefore
gates MiniLM behind a cosine admission floor (`PRODUCTION_ADMISSION_COSINE`,
0.30): a query whose best dense match sits below the floor keeps its BM25
result, which is empty for most no-match wording. On the 2026-09-02 snapshot
this rejected 12 of 15 no-match cases and none of the 64 match or multi cases
(their weakest best-cosine was 0.315; the holdout split's was 0.351). The
margin is thin and the floor is model-specific: the earlier v14 evaluation
found that an absolute cutoff removed substantial valid recall, and the legacy
0.44 Potion floor would drop nine match cases here. Re-run the
`record_admission_scores_per_stage` diagnostic whenever the embedding model or
the qrels change, and treat the separately qualified categorical-admission
stage as the fuller answer for abstention.

The compose stack also prevents the catalog-only provider workers and
`llm-router` from inheriting API keys or tokens from the operator's shell.
Local credential imports are disabled for GitHub Copilot and redirected to
nonexistent directories for Codex and Claude Code. The provider function
surfaces remain available to the directory catalog, but their external model
operations remain unconfigured.

## Runtime prerequisites

MiniLM retrieval and reranking are always compiled into `iii-directory` on
Linux x86_64 GNU (other targets build the BM25-only worker). The compose file
uses these portable defaults:

- Model bundle:
  `~/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf`
  (also the built-in default of `function_search_model_path`). With a semantic
  mode configured and the bundle missing at boot, `iii-directory` downloads the
  ten pinned files from Hugging Face once, each verified by byte length and
  SHA-256 before use (`function_search_model_download`, default `true`). This
  compose sets it to `false`: the stack makes no remote model calls and expects
  the bundle to be provisioned ahead of time.
- Static ONNX Runtime 1.28.0:
  `~/.cache/iii/onnxruntime-static-1.28.0-x86_64-unknown-linux-gnu`

The model directory must contain the embedding files at its root and the
reranker files under `reranker/`. At startup, the directory worker verifies all
ten files by exact byte length and SHA-256 before loading them. The ONNX runtime
directory must contain `libonnxruntime.a`; `iii-directory/scripts/provision-onnxruntime.sh`
downloads and verifies it once (no network when already present), and
`ORT_LIB_PATH` overrides its default location. Every `iii-directory` build on this
target links it, including `harness/worker-compose.yaml`.

Slack and Telegram use the repository's interface-collection token in this
stack. This lets both workers register their static function surfaces without
real account credentials; calls to their external APIs will fail until their
configuration entries receive real tokens.

## Start and inspect

```bash
cd harness
iii compose --up --file worker-compose.minilm-production.yaml
```

The stack is isolated on `ws://127.0.0.1:49234` in the
`minilm-production` namespace. For example:

```bash
iii trigger --port 49234 -n minilm-production \
  directory::search_functions \
  --json '{"capabilities":["fetch a web page by URL"]}'
```

## BM25 versus MiniLM benchmark

The ignored Rust benchmark runs both modes through the real
`directory::search_functions` handler. Both lanes use the same captured live
catalog and the same 79 reviewed qrel cases; registry search is disabled. The
report includes input and model hashes, overall and sliced quality metrics,
mean/p50/p95/max latency, MiniLM-minus-BM25 deltas with paired bootstrap 95%
intervals, and every per-case ranking.
Each case embeds its qrels, forbidden constraints, derived judgments, and a
flag proving that the production MiniLM path completed without fallback. A
capture with any collection error and a run with any MiniLM fallback fail.

With the stack ready, capture its normalized catalog without replacing the
committed fixture:

```bash
cd harness
python3 scripts/search_eval_catalog.py \
  --port 49234 \
  --namespace minilm-production
```

Use the `artifact` path printed by that command:

```bash
cd ../iii-directory
ORT_LIB_PATH="$HOME/.cache/iii/onnxruntime-static-1.28.0-x86_64-unknown-linux-gnu" \
III_DIRECTORY_MINILM_MODEL_PATH="$HOME/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf" \
III_DIRECTORY_SEARCH_BENCHMARK_CATALOG_PATH="<artifact>/catalog.json" \
III_DIRECTORY_SEARCH_BENCHMARK_OUTPUT="<artifact>/bm25-vs-minilm-production.json" \
cargo test --lib benchmark_bm25_against_minilm_production \
  -- --ignored --nocapture --test-threads=1
```

### Reference run: 2026-09-02

The reference snapshot contains 663 searchable functions from a stack where
all 69 declared containers were ready. It evaluates 79 cases containing 91
capabilities. Catalog SHA-256 is
`00ceafb48671db2329f9e4534b2146ce103724ec46ea3d126d81bc122d318ea2`;
qrels SHA-256 is
`94d159b85fa81f0a73e07a3b21211ed82a8299790a34482b95664cea933abb8a`.

| Quality slice | Metric | BM25 | MiniLM | Delta |
| --- | --- | ---: | ---: | ---: |
| Overall | MRR@1 | 0.7222 | 0.7778 | +0.0556 |
| Overall | Recall@12 | 0.7469 | 0.9444 | +0.1975 |
| Overall | nDCG@12 | 0.7442 | 0.8438 | +0.0996 |
| Overall | Worker recall@12 | 0.8148 | 0.9630 | +0.1481 |
| Exact | MRR@1 | 0.8333 | 0.8333 | 0.0000 |
| Exact | Recall@12 | 0.8651 | 0.9643 | +0.0992 |
| Paraphrase | MRR@1 | 0.3333 | 0.5833 | +0.2500 |
| Paraphrase | Recall@12 | 0.3333 | 0.8750 | +0.5417 |
| Paraphrase | nDCG@12 | 0.3721 | 0.7168 | +0.3447 |
| Multi-capability | Complete coverage@12 | 0.7000 | 1.0000 | +0.3000 |
| No-match | False-positive rate | 0.1333 | 0.2667 | +0.1333 |

The report also carries a paired percentile bootstrap (2000 resamples of
the case set, fixed seed) for every slice. A delta is significant when its 95%
interval excludes zero:

| Quality slice | Metric | Delta | CI 2.5% | CI 97.5% | Significant |
| --- | --- | ---: | ---: | ---: | :---: |
| Overall | MRR@1 | +0.0556 | -0.0200 | +0.1373 | no |
| Overall | Recall@12 | +0.1975 | +0.1037 | +0.2963 | yes |
| Overall | nDCG@12 | +0.0996 | +0.0321 | +0.1740 | yes |
| Exact | Recall@12 | +0.0992 | +0.0238 | +0.1905 | yes |
| Paraphrase | MRR@1 | +0.2500 | +0.0000 | +0.5000 | no |
| Paraphrase | Recall@12 | +0.5417 | +0.2917 | +0.7917 | yes |
| Multi-capability | Complete coverage@12 | +0.3000 | +0.1000 | +0.6000 | yes |
| No-match | False-positive rate | +0.1333 | +0.0000 | +0.3333 | no |
| Holdout | Recall@12 | +0.1562 | +0.0000 | +0.3333 | no |

With 79 cases the MRR@1 gain is not distinguishable from noise; the recall,
nDCG, multi-capability coverage and paraphrase gains are. After the admission
floor the no-match false-positive rate (4 of 15 versus BM25's 2 of 15) is no
longer distinguishable from BM25's.

MiniLM provides the intended quality gain on semantic wording and complete
multi-capability retrieval. The three remaining no-match false positives that
BM25 avoids are "write a friendly reply to this message" (`email::send`),
"format this JSON so a person can read it" and "draft a thirty minute meeting
agenda"; their best cosines (0.37, 0.37, 0.30) overlap the weakest genuine
matches, so the floor cannot remove them without losing recall.

The overall ranking metrics use the 54 `match` cases, complete coverage uses
the 10 `multi` cases, and false-positive rate uses the 15 `no_match` cases.
The report records these denominators for every slice. All model-eligible
queries completed production MiniLM retrieval and reranking; none of the 79
hybrid calls fell back to BM25.

| Latency | BM25 | MiniLM | Delta |
| --- | ---: | ---: | ---: |
| Mean | 33.2 ms | 371.6 ms | +338.5 ms |
| p50 | 32.1 ms | 364.9 ms | +332.8 ms |
| p95 | 37.3 ms | 732.8 ms | +695.4 ms |
| Max | 75.0 ms | 930.8 ms | +855.8 ms |

These latency values came from the Cargo test profile on one local CPU, with
one warm-up case and the full 663-function corpus. They are useful for
same-machine comparison, not as a production service-level target.

The cross-encoder scores only the top 48 fused candidates per query
(`PRODUCTION_RERANK_DEPTH`). Scoring the complete retrieval union instead, on
the same snapshot and machine, produced identical MRR@1, recall@12 and
per-case win/loss counts, nDCG@12 of 0.8400 overall and 0.6957 on paraphrases,
and a mean hybrid latency of 4,951.8 ms (p95 8,703.6 ms), about 12x slower.

The same run under `cargo test --release` (same snapshot, same machine)
measured BM25 at 4.4 ms mean and the hybrid lane at 362 ms mean, 316 ms p50,
656 ms p95, 830 ms max, with byte-identical rankings. The remaining hybrid cost
is ONNX inference (one MiniLM query embedding plus 48 cross-encoder pairs in
batches of 8), which the Rust build profile does not change.
