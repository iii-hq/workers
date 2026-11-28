# iii-cloud worker: tasks

Five tasks that run in parallel. Each owns disjoint files. The contracts
between them are fixed in [`spec.md`](spec.md):

- function ids and JSON types: §11;
- errors: §12;
- events: §10;
- configuration: §1;
- permissions: §13;
- the UI: §14.

A change to a contract goes through the lead, never through a silent edit.

Every task follows **spec.md → Machine rules**. In short:

- Work only in `~/workspaces/worktrees/workers-iii-cloud`, except task D,
  which uses its own iii-cloud clone, and task E, which uses
  `~/tmp/iii-cloud-worker/e2e/`.
- `CARGO_TARGET_DIR=~/tmp/iii-cloud-worker/target` and
  `TMPDIR=~/tmp/iii-cloud-worker/tmp`; never `/tmp` or the scratchpad.
- No `pnpm install`, `pnpm exec` or `pnpm run` in a worktree.
- `SKIP_UI_BUILD=1` for cargo.
- Never touch `~/workspaces/workers`, `~/workspaces/iii-cloud`, the dev or
  fabrica stacks, or containers you did not create.
- Never use plain `kubectl`.
- No secret in an argument, a log or the output.
- No commit, push or PR until the owner asks. Never merge. Nothing reaches
  production or `iii-hq/registry`.

Sync points:

- **A's golden schemas** (`iii-cloud/tests/golden/schemas/`) are the
  executable form of §11. B checks its `ui/src/types.ts` against them before
  it finishes.
- **E** runs E2E-1 once A's binary builds, the captures once B's bundle
  builds, and E2E-2 once D's branches build.
- **C's** `iii.worker.yaml` config block must equal A's `config.yaml` and
  `Config::default()`.

---

## A. Worker core (Rust)

**Goal.** The `iii-cloud` binary: every function of spec §11 with the
behavior of §2–§10, the configuration, the store, the events, and UI asset
serving.

**Owns:**

- `iii-cloud/Cargo.toml`, `iii-cloud/Cargo.lock`, `iii-cloud/build.rs`,
  `iii-cloud/config.yaml`;
- `iii-cloud/src/**`;
- `iii-cloud/tests/**`: Rust tests, `golden/schemas/*.json`, fixtures,
  among them a fake `gh` script and the tiny run-here app.

The stub `iii-cloud/ui/dist/{page.js,styles.css}` it creates is git-ignored,
and B overwrites it.

**Depends on.** Nothing to start. Copy, don't import:

- `github/` for layout, catalog, golden test, `configuration.rs` and
  `ui.rs`;
- `secrets/src/error.rs` for coded errors;
- `github/src/events.rs` for the event call;
- `provider-openai-codex/src/session.rs` for device polling and the vault;
- `github/tests/support/real_engine.rs`.

Crates:

- `iii-sdk = "=0.27.0"`;
- `iii-console-ui = { path = "../crates/console-ui" }`;
- `schemars 0.8`, `serde`, `serde_json`, `serde_yaml_ng 0.10`;
- `tokio`;
- `reqwest 0.12` with `default-features = false` and `["json",
  "rustls-tls"]`;
- `sha2`, `hex`, `uuid`, `chrono`, `anyhow`, `thiserror`, `tracing`;
- dev: `wiremock 0.6`, `tempfile`.

No `serde_yaml` (CI denies it).

**Verify first** (each was flagged in the spec; write what you find into a
code comment with the file and line):

1. The run engine's registered-triggers function id on iii 0.27.0. The cloud
   calls it `registered-triggers::list`
   (`crates/core/src/engine.rs:16` in iii-cloud). Also the fields of
   `engine::workers::list`. Check both with `engine::functions::list` on a
   0.27.0 engine.
2. The healthcheck path with `web.api.prefix` and `strip`: what
   `cluster.health.check` requests (iii-cloud `crates/control-plane/src`).
3. The worker config the cloud writes for a managed Postgres: `git -C
   ~/workspaces/iii-cloud grep -n databases origin/main -- crates/control-plane/src crates/plan/src`.
4. The console URL segment `$project`: iii-cloud
   `apps/web/src/routes/$team/$project/$environment/deployments/$number.tsx`.
5. The `iii --version` output format of 0.26.0 and 0.27.0, for the version
   parse.

**Order of work:**

1. **Skeleton.** `main.rs` and `lib.rs`, the configuration (§1, hot reload),
   `store.rs` (atomic `state.json`), `events.rs` (`iii-cloud::on-change`),
   `ui.rs` (`ConsoleUi::new("iii-cloud")` with `iii-cloud/page.js` and
   `iii-cloud/styles.css`), and `functions::catalog()` with every §11 id and
   description. Then the golden test and the catalog-order test.
2. `cloud.rs`, the HTTP client (§2 "Every cloud call", §11 `api`), and
   `connect` (§2).
3. `project::resolve` and `project::use` (§3).
4. `check`: the plan, the checks here, `Problem`, the owner table, `ready`,
   `report` (§4).
5. `secret` and `secret::provide` (§5), with `.observable(false)` and
   `trace_hidden`.
6. The run here (§6): the run copy, the ports, Postgres, the stages,
   teardown, `RUN_BUSY`, the 240 s wait.
7. `deploy` (§8), with the plan digest; `watch` and the background
   followers (§9); resume after restart.
8. `status` (§11; a store read, `refresh`).

**Done when** these pass, from `iii-cloud/`:

```
export CARGO_TARGET_DIR=~/tmp/iii-cloud-worker/target TMPDIR=~/tmp/iii-cloud-worker/tmp SKIP_UI_BUILD=1
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
III_ENGINE_BIN=~/tmp/iii-cloud-worker/bin/iii cargo test --locked --all-features
III_ENGINE_BIN=~/tmp/iii-cloud-worker/bin/iii cargo test --locked -- --ignored run_here
```

- The tests cover every bullet of spec §Testing → "Unit and contract". The
  golden files exist for all 11 public ids (`connect`, `status`,
  `project::resolve`, `project::use`, `api`, `check`, `secret`,
  `secret::provide`, `deploy`, `watch`, `on-change`).
- `ss -ltn` shows no new listener and no port 49134 or 3111 change after the
  ignored run test.
- `docker ps -a` has no `iii-cloud-run-*`.
- Once C has landed `iii.worker.yaml`, `python3 .github/scripts/validate_worker.py
  --worker iii-cloud --base-ref origin/main --source-changed '["iii-cloud"]'`
  (from the worktree root) has no hard error.

---

## B. Console UI

**Goal.** The `iii cloud` page matching the 12 boards (spec §14): the
renderers for `connect`, `check`, `secret`, `deploy` and `watch`, the
configuration form, and the palette command.

**Owns:**

- `iii-cloud/ui/**`: `package.json`, `build.mjs`, `tsconfig.json`,
  `vitest.config.ts`, `page.tsx`, `styles.css`, `src/**`, among them
  `src/types.ts`, `src/fixtures.ts` and the tests;
- `iii-cloud/ui/fixtures/<board>/state.json`: the 12 store files for the
  visual check;
- the line `- iii-cloud/ui` in `pnpm-workspace.yaml`;
- the `iii-cloud/ui:` importer block in `pnpm-lock.yaml`.

**Depends on:**

- spec §11 types, the function ids, the event hub id
  `iii::iii-cloud-ui::events`, and §14;
- A's golden schemas, to compare before finishing;
- the boards in `design/*.dc.html`, extracted to text (structure and copy)
  and opened in a browser for layout.

Read first, in this order: `~/.claude/skills/ade-worker-design/SKILL.md`,
`console-injectable-ui.md`, `console-design.md`, `patterns.md`,
`design-console-ui.md`. Components come only from
`packages/console-ui/index.d.ts` (and `hooks.d.mts`, `format.d.mts`); icons
from `lucide-react`.

**Setup without installing:**

- `package.json`: name `@iii-workers/iii-cloud-ui`, and exactly the
  dependency set of `provider-openai/ui/package.json`: `@iii-dev/console-ui`
  `workspace:*`; devDependencies `@types/react`, `@types/react-dom`,
  `esbuild`, `jsdom ^30.0.1`, `lucide-react`, `react`, `react-dom`,
  `typescript`, `vitest` (`catalog:`). Scripts: `build` = `tsc --noEmit &&
  node build.mjs`, `typecheck`, `test` = `vitest run`, `watch`.
- `build.mjs`: `buildWorkerUi({ scope: 'iii-cloud', keyframePrefixes:
  ['iii-cloud-'], lint: { strict: true } })`.
- `tsconfig.json` extends `@iii-dev/console-ui/tsconfig.worker-ui.json`.
- `ln -s ~/workspaces/workers/provider-openai/ui/node_modules
  iii-cloud/ui/node_modules`. Remove it when done.
- `pnpm-lock.yaml`: copy the `provider-openai/ui:` importer block (line
  ~1523) as `iii-cloud/ui:`, in sorted position.

**Done when:**

```
cd iii-cloud/ui
./node_modules/.bin/tsc --noEmit
node build.mjs                      # design lint strict; writes dist/page.js, dist/styles.css
./node_modules/.bin/vitest run
~/workspaces/workers/node_modules/.pnpm/@biomejs+cli-linux-x64@2.5.4/node_modules/@biomejs/cli-linux-x64/biome ci .
# lockfile, in a scratch copy with no node_modules anywhere:
rm -rf ~/tmp/iii-cloud-worker/lockcheck && mkdir -p ~/tmp/iii-cloud-worker/lockcheck
(cd ~/workspaces/worktrees/workers-iii-cloud && git ls-files -co --exclude-standard -z | tar --null -T - -cf - | tar -x -C ~/tmp/iii-cloud-worker/lockcheck)
(cd ~/tmp/iii-cloud-worker/lockcheck && pnpm install --lockfile-only --ignore-scripts)
diff ~/workspaces/worktrees/workers-iii-cloud/pnpm-lock.yaml ~/tmp/iii-cloud-worker/lockcheck/pnpm-lock.yaml   # no output
rm -rf ~/tmp/iii-cloud-worker/lockcheck
```

- Vitest covers each of the 12 board states (texts, chips, buttons, the pill
  rule), the phone layout under 640 px, the renderers' `details` unwrap and
  the error fall-through, and that a typed secret value reaches only
  `iii-cloud::secret::provide`.
- `src/types.ts` agrees with A's golden response schemas (a vitest that
  loads `../tests/golden/schemas/*.json` and checks the field names of each
  type).
- After A builds, in E's stack: the visual check of spec §Testing → "UI" at
  1440, 720 and 390 px, in both themes, with the `design-diff` skill on port
  3413, and `ade::ui-manifest` free of warnings. Record every deviation from
  a board in the PR description. The only planned one is the Environment
  field of board 01 (D3).

---

## C. Skill, profile, README, manifest, permissions

**Goal.** Everything that makes `compose::add worker=iii-cloud` usable by a
person and their agent, apart from the code.

**Owns:**

- `iii-cloud/iii.worker.yaml` (spec §1, verbatim);
- `iii-cloud/iii-permissions.yaml`, and the same block appended to the
  repo-root `iii-permissions.yaml` (spec §13);
- `iii-cloud/README.md`;
- `iii-cloud/skills/SKILL.md`, `iii-cloud/skills/onboarding.md`,
  `iii-cloud/skills/plan-codes.md`;
- `iii-cloud/agents/cloud-ready.md`;
- the `iii-cloud:` entry in `.deploy/workers.yaml`: a copy of `github:` with
  path, binary and `frontends[0].source_path: iii-cloud/ui`, and `publish:
  true`.

**Depends on:**

- spec §2–§15, the function ids and descriptions (A's catalog
  descriptions must match the SKILL's one-liners: agree them with A through
  the lead);
- `worker-readme.md` and `DOCUMENTATION_GUIDELINES.md` (absolute links, the
  required sections);
- `docs/sops/new-worker.md`;
- `kanban/agents/*.md` for the profile frontmatter;
- iii-cloud `crates/core/src/error.rs:26-82,220-330` (`git -C
  ~/workspaces/iii-cloud show origin/main:crates/core/src/error.rs`) for
  `plan-codes.md`;
- the design page §2–§6 for the registry examples and the web conversation.

**Content rules:**

- **README:** Install (`iii call compose::add worker=iii-cloud` in the
  harness, never the app; requires `git`, `gh` logged in, Docker for a
  database resource, and iii equal to the cloud's engine), Quickstart (sign
  in, resolve, use, check), Configuration (the §1 table), Event functions
  (`iii-cloud::on-change` and its filters), and what the run here does not
  cover.
- **`plan-codes.md`:** one row per code in iii-cloud's `ALL` (58 at
  `d31ed9457`), with severity, kind, owner (spec §4) and a one-line fix; the
  seven local codes; the three registry decisions with their
  recommendations.
- **`onboarding.md`:** the eleven steps of spec §15, with exact calls
  (`harness::observe` filters, `ade::workspace::open`,
  `ade::working-directory::propose`, `gh pr create` with `check.report`).
- **`cloud-ready.md`:** the frontmatter of spec §15, and a body that plans
  first (board 02), deploys only when asked, and never asks for a secret
  value in chat.

**Done when** (from the worktree root):

```
python3 .github/scripts/validate_worker.py --worker iii-cloud --base-ref origin/main --source-changed '["iii-cloud"]'
python3 .github/scripts/build_skills_payload.py --worker iii-cloud --version 0.1.0 --out ~/tmp/iii-cloud-worker/skills-payload.json
python3 .github/scripts/deployment_compiler.py compile-index --source-sha "$(git rev-parse HEAD)" --compiler-repository iii-hq/workers --output-dir ~/tmp/iii-cloud-worker/descriptor-index
python3 -c 'import yaml,sys; [yaml.safe_load(open(p)) for p in sys.argv[1:]]' iii-cloud/iii.worker.yaml iii-cloud/iii-permissions.yaml iii-permissions.yaml
```

- The skills payload has `SKILL.md`, `skills/onboarding.md`,
  `skills/plan-codes.md` and `agents/cloud-ready.md`.
- A check compares the code column of `plan-codes.md` with the codes of
  `crates/core/src/error.rs` at `origin/main` and finds none missing.
- `iii.worker.yaml`'s `config` equals A's `config.yaml`.

---

## D. The two iii-cloud PRs

**Goal.** C1 (device sign-in for workers) and C2 (plan any commit of the
repository), exactly as spec §16, as two branches ready for review. Not
merged, not deployed.

**Owns** its own clone, so the other session's checkout is only read:

```
git clone --no-checkout ~/workspaces/iii-cloud ~/workspaces/worktrees/iii-cloud-worker-changes
cd ~/workspaces/worktrees/iii-cloud-worker-changes
git remote set-url origin git@github.com:iii-hq/iii-cloud.git && git fetch origin
git switch -c feat/device-sign-in-workers origin/main              # C1
git worktree add ../iii-cloud-plan-any-commit -b feat/plan-any-commit origin/main   # C2
```

Record the base (`git rev-parse origin/main`). Re-read the line numbers of
§16 if it is not `d31ed9457`.

**Depends on:** spec §16 only. Nothing in the workers repo.

**Tooling:**

- `CARGO_TARGET_DIR=~/tmp/iii-cloud-worker/target-cloud`.
- Postgres for the database tests: our own container,
  `docker run -d --name iii-cloud-worker-pg -e POSTGRES_PASSWORD=postgres -p 127.0.0.1:55432:5432 postgres:17`.
  Remove it at the end.
- Node tooling: no install in the clone. Symlink
  `~/workspaces/iii-cloud/node_modules` and the `node_modules` of
  `apps/web` and `contracts/openapi` read-only, if present, and call the
  binaries directly. If they are absent, report it and run only the Rust and
  database checks.

**Done when**, in each branch:

```
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55432/postgres .github/scripts/db-tests.sh
.github/scripts/migrate.sh check && .github/scripts/migrate.sh unchanged origin/main
# contracts/openapi: lint, generate (git diff --exit-code on client/schema.d.ts), typecheck, test
# apps/web (C1): typecheck, biome check, and the device-approval and devices Playwright specs
```

- **C1:** the new and changed tests of §16 pass, and the existing app-session
  tests pass unchanged.
- **C2:** the renamed scenario passes in both `tests/api.rs` and
  `tests/api_pg.rs`, and a POST of an off-branch commit is still 422.
- Each branch has one commit, titled `feat(identity): device sign-in for
  workers` or `feat(deploy): plan any commit of the repository`. The body
  carries the security note of §16, and the PR text is ready to paste
  (label `shared`).
- Push and draft PRs happen only on the owner's go. Never merge.

---

## E. End to end on the kind stack

**Goal.** The e2e of spec §Testing → "End to end on the kind stack",
repeatable, with evidence: the stack, the fixtures, E2E-1, E2E-2, the leak
check and the restore.

**Owns** `~/tmp/iii-cloud-worker/e2e/**`, outside git:

- `runbook.md`;
- `stack/worker-compose.yaml` and `stack/.env` (`0600`);
- `bin/{up.sh,down.sh,cloud.sh,setup-kind.sh,load-fixture.sh,e2e-1.sh,e2e-2.sh,leaks.sh,restore.sh}`;
- `fixtures/{F1,F2,F3}`;
- `src/` (a `git archive` of iii-cloud `origin/main`, with `.tools` copied
  in);
- `before/`, `ev/`, `secrets/`.

It also owns the private iii 0.27.0 at `~/tmp/iii-cloud-worker/bin/iii`.

**Depends on:**

- A's binary at `~/tmp/iii-cloud-worker/target/debug/iii-cloud`, to run
  steps 1-12;
- B's `dist/`, for step 11;
- D's branches, for E2E-2;
- spec §11 for every call and assertion;
- `~/tmp/assistant-real/runbook.md` for the safe reuse of kind;
- iii-cloud `tests/e2e/suites/lib/{demo.sh,push.sh,assert.sh}` and
  `tests/e2e/lib/guard.sh`, read from `git show origin/main:…` into `src/`.

**Can start at once:**

- the private iii 0.27.0 (`iii --version` prints 0.27.0);
- `up.sh` and `down.sh` for the stack (engine `127.0.0.1:49410`, namespace
  `iiicloud-e2e`, ADE on 3413, `secrets` and `ade` pinned from
  `~/workspaces/dev/worker-compose.lock`);
- the guarded `setup-kind.sh` (copy `.tools`, `guard_check`, the
  "nobody else is using it" checks, `before/`, our database, D02-D07, the API
  key into `stack/.env` with Python);
- the fixtures F1-F3 and `load-fixture.sh` (bare repository, worktree reset,
  `/repo-rm`, the environment's manifest);
- `leaks.sh` (the token, the API key, the cookie and each secret value
  grepped in the stack logs, `engine::traces::*`, `state.json` and `ev/` →
  0);
- `restore.sh`.

**Rules specific to E:**

- `k` (the guarded wrapper) only, never `kubectl`.
- Own image tags and digests only, never `:dev`.
- One environment at a time: the CPU budget of kind.
- Stop and ask if `k get ns -l iii.cloud/env` is not empty, or if the
  control plane runs another image than the one recorded, before patching.
- Real GitHub is never called: the bare repository and `insteadOf` stand in
  for it.

**Done when:**

- `bin/e2e-1.sh` runs steps 1-12 of E2E-1 and writes `ev/<step>/{request.txt,status,result.json,verdict}`,
  every verdict `PASS`.
- `bin/e2e-2.sh` does the same for E2E-2 against the image built from D's
  branches.
- `bin/leaks.sh` prints 0 for every value.
- `bin/restore.sh` leaves the cluster as `before/` (control-plane spec
  diff empty, `/repo-rm` restored, our database dropped, no `iii.cloud/env`
  namespace).
- The dev stack's `iii ps` on 49134 is unchanged before and after.
- `runbook.md` lists the commands, the evidence, and what E2E-1 and E2E-2
  cannot cover (spec §Testing).
