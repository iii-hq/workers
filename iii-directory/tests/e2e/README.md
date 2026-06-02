# iii-directory worker — end-to-end harness

Self-asserting smoke harness for the `iii-directory` worker. Builds + installs
the worker, starts its own iii engine, downloads **real** worker bundles from
the public registry (https://api.workers.iii.dev), and asserts every
`directory::*` behavior with one command. Exits 0 on PASS, 1 on any FAIL.

Coverage: the reads (`list` / `get` / `index`), the registry proxy, the
downloads, the prose error contract (`D110` / `D112` / `D210` / `D310` /
`D311`), security validation, and ~150 adversarial / dumb-LLM scenarios.

## Prerequisites

- Rust toolchain (`cargo` on `$PATH`)
- `jq` on `$PATH`
- The iii engine on `$PATH` (or at `$HOME/.local/bin/iii`). Install with:
  ```sh
  curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
  ```
- Network access — the run downloads real bundles from the registry.

## Run

```sh
./run-tests.sh              # build + install the worker, then run the full suite
./run-tests.sh --no-build   # reuse the iii-directory already in ~/.iii/workers
./run-tests.sh --keep       # leave the engine running afterwards (debugging)
PORT=49210 ./run-tests.sh   # use a non-default engine port
```

Builds `iii-directory` (debug), copies it to `~/.iii/workers/iii-directory`,
substitutes `config.yaml` into `reports/engine-config.yaml` (absolute paths),
starts the engine, lays down local-override fixtures under `.iii/skills/`,
downloads `shell` / `database` / `coder` / `iii`, and runs every assertion.
Logs land in `reports/`.

## Layout

```
run-tests.sh   the asserting suite (one command, exits 0/1)
config.yaml    engine-config template; __E2E_DIR__ is substituted at runtime
reports/       generated logs + the effective engine config (gitignored)
skills-home/   registry downloads land here at runtime (gitignored)
.iii/skills/   local-override fixtures created at runtime (gitignored)
```

## What it proves

Every `directory::*` function against real registry data, plus:

- **Error recovery** — prose, self-correcting: `D110` (skill miss) with
  `Did you mean: …` + `Next: call …`, `D112` (a function id like
  `database::execute` passed to `get`), `D210` (prompt miss), `D310` (registry
  worker miss). No internal registry URL / raw HTTP status leaks.
- **Download** — the explicit `download_from_registry` / `download_from_repo`
  split (required fields make the source unambiguous) plus the `download` alias.
- **Security** — skill-id and worker-name validation (path traversal,
  query/fragment injection, uppercase / non-ASCII), and git repo-URL RCE guards
  (`ext::`, `file://`, `--upload-pack`, `::` transport).
- **Dumb-LLM scenarios** — wrong function names, id-vs-function_id confusion,
  wrong parameter names, type confusion (number/array/null), copy-paste from
  prior output (`.md` / `iii://` / bare name resolve), natural-language ids.
