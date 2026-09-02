# Local MiniLM production stack

`worker-compose.minilm-production.yaml` starts every worker declared by a
top-level `iii.worker.yaml` in this repository. The directory worker runs the
hybrid production search pipeline on CPU:

1. BM25 and `sentence-transformers/all-MiniLM-L6-v2` retrieve against the full
   installed function catalog.
2. Reciprocal-rank fusion combines both retrieval orders.
3. `cross-encoder/ms-marco-MiniLM-L6-v2` reranks the complete retrieval union.
4. A second reciprocal-rank fusion anchors reranker order to retrieval.

No hosted embedding or LLM API is used by function search. Exact function IDs
bypass both models. If a local model is unavailable or returns invalid output,
the affected query falls back to BM25.

This stack intentionally ships the v14 ordering policy without the separate
categorical-admission judge. MiniLM retrieves and orders candidates; it does
not reliably decide that no installed function supports a natural-language
capability. A no-match request can therefore still return plausible but
unsupported functions. Adding an absolute MiniLM score cutoff is not an
equivalent substitute: the v14 evaluation found that this removes substantial
valid recall. Reliable abstention requires the separately qualified
categorical-admission stage.

The compose stack also prevents the catalog-only provider workers and
`llm-router` from inheriting API keys or tokens from the operator's shell.
Local credential imports are disabled for GitHub Copilot and redirected to
nonexistent directories for Codex and Claude Code. The provider function
surfaces remain available to the directory catalog, but their external model
operations remain unconfigured.

## Runtime prerequisites

The `minilm-production` Cargo feature is supported on Linux x86_64 GNU. The
compose file uses these portable defaults:

- Model bundle:
  `~/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf`
- Static ONNX Runtime 1.28.0:
  `~/.cache/iii/onnxruntime-static-1.28.0-x86_64-unknown-linux-gnu`

The model directory must contain the embedding files at its root and the
reranker files under `reranker/`. At startup, the directory worker verifies all
ten files by exact byte length and SHA-256 before loading them. The ONNX runtime
directory must contain `libonnxruntime.a`; set `ORT_LIB_PATH` before starting
compose to override its default location.

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
mean/p50/p95/max latency, MiniLM-minus-BM25 deltas, and every per-case ranking.
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
  --features minilm-production -- --ignored --nocapture --test-threads=1
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
| Overall | nDCG@12 | 0.7442 | 0.8400 | +0.0957 |
| Overall | Worker recall@12 | 0.8148 | 0.9630 | +0.1481 |
| Exact | MRR@1 | 0.8333 | 0.8333 | 0.0000 |
| Exact | Recall@12 | 0.8651 | 0.9643 | +0.0992 |
| Paraphrase | MRR@1 | 0.3333 | 0.5833 | +0.2500 |
| Paraphrase | Recall@12 | 0.3333 | 0.8750 | +0.5417 |
| Paraphrase | nDCG@12 | 0.3721 | 0.6957 | +0.3236 |
| Multi-capability | Complete coverage@12 | 0.7000 | 1.0000 | +0.3000 |
| No-match | False-positive rate | 0.1333 | 1.0000 | +0.8667 |

MiniLM provides the intended quality gain on semantic wording and complete
multi-capability retrieval. The no-match result also quantifies the known
boundary of this ordering-only stack: MiniLM is not an admission classifier.
It needs the separately qualified categorical-admission stage to abstain
reliably.

The overall ranking metrics use the 54 `match` cases, complete coverage uses
the 10 `multi` cases, and false-positive rate uses the 15 `no_match` cases.
The report records these denominators for every slice. All model-eligible
queries completed production MiniLM retrieval and reranking; none of the 79
hybrid calls fell back to BM25.

| Latency | BM25 | MiniLM | Delta |
| --- | ---: | ---: | ---: |
| Mean | 32.7 ms | 4,951.8 ms | +4,919.1 ms |
| p50 | 32.2 ms | 4,369.6 ms | +4,337.4 ms |
| p95 | 35.3 ms | 8,703.6 ms | +8,668.3 ms |
| Max | 36.8 ms | 12,455.0 ms | +12,418.1 ms |

These latency values came from the Cargo test profile on one local CPU, with
one warm-up case and the full 663-function corpus. They are useful for
same-machine comparison, not as a production service-level target.
