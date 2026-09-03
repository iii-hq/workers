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
found that an absolute cutoff removed substantial valid recall. Re-calibrate
the floor whenever the embedding model changes, and treat the separately
qualified categorical-admission stage as the fuller answer for abstention.

The compose stack also prevents the catalog-only provider workers and
`llm-router` from inheriting API keys or tokens from the operator's shell.
Local credential imports are disabled for GitHub Copilot and redirected to
nonexistent directories for Codex and Claude Code. The provider function
surfaces remain available to the directory catalog, but their external model
operations remain unconfigured.

## Runtime prerequisites

MiniLM retrieval and reranking are compiled into `iii-directory` on every
target for which `ort-sys` ships a pinned, SHA-256-verified static ONNX
Runtime: Linux glibc x86_64 and aarch64, Apple Silicon macOS, and Windows
MSVC. Other targets (musl, armv7, Intel macOS) build the BM25-only worker and
log that at boot. The compose file uses these portable defaults:

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
directory must contain `libonnxruntime.a`. Without `ORT_LIB_PATH`, `ort-sys`
downloads the pinned runtime for the build target itself at build time
(verified against its own manifest); `iii-directory/scripts/provision-onnxruntime.sh`
pre-provisions the x86_64 Linux archive for offline or cached builds, and
`ORT_LIB_PATH` points at it (as `harness/worker-compose.yaml` does).

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
