# Adding a new worker

This is a normative checklist for adding a new worker to this monorepo. Each
requirement maps directly to either a CI gate (`pr-checks`, lint, tests) or a
CD gate (Create Tag → release dispatcher). Skipping a step will block the PR
or the release.

It is written for AI agents to follow step-by-step but humans should be able
to use it just as well.

## 1. Identity

- Pick a folder name at the repo root, matching `^[a-z0-9][a-z0-9_-]*$` (same
  regex as the workers registry's `worker_name`).
- The folder name **is** the worker name. It appears in:
  - the git tag pushed by Create Tag (`<folder>/v<X.Y.Z>`)
  - `iii.worker.yaml.name`
  - the registry record at `api.workers.iii.dev`
  - the consumer install command (`iii worker add <folder>`)
- Names must be unique. If you want to ship two related workers, give them
  distinct folders (e.g. `image-resize` and `image-thumbnail`).

## 2. Required files

Each file below is checked by the `pr-checks` job in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml). Missing files fail the
PR.

| File | Required for | Purpose |
|---|---|---|
| `<worker>/README.md` | all | Non-empty. Body becomes the `readme` field on `POST /publish`. |
| `<worker>/iii.worker.yaml` | all | Declares `name`, `language`, `deploy`, `manifest` (and `bin` for Rust binaries). |
| `<worker>/skill.md` | all | Top-level skill registered at `iii://<worker>` so MCP clients can orient to the worker. Convention only — not yet a CI gate. See §10. |
| Language manifest | all | `Cargo.toml` (Rust), `package.json` (Node), `pyproject.toml` (Python). The `version` field is the source of truth. |
| `<worker>/tests/` | all | Non-empty. Holds at least one test file the standard runner picks up. |
| `<worker>/Dockerfile` | `deploy: image` only | Listens on `III_URL`, exits cleanly on `SIGTERM`. |

### `iii.worker.yaml` shape

```yaml
iii: v1
name: my-worker          # must equal the folder name
language: rust           # rust | node | python
deploy: binary           # binary | image
manifest: Cargo.toml     # path relative to <worker>/
bin: iii-my-worker       # binary deploy only — name produced by cargo
description: One-line description shown in the registry.
```

For containers, drop `bin`.

## 3. Pick a deploy type

| Worker shape | `deploy` | What CD does |
|---|---|---|
| Rust standalone CLI/daemon | `binary` | Cross-compiles to 9 targets via [`_rust-binary.yml`](.github/workflows/_rust-binary.yml), uploads tar.gz / zip + sha256 to a GitHub Release, then publishes binary URLs via `POST /publish`. |
| Node or Python worker | `image` | Builds a multi-arch image via [`_container.yml`](.github/workflows/_container.yml), pushes to `ghcr.io/<owner>/<worker>:<version>` and `:<registry_tag>`, then publishes the image reference via `POST /publish`. |
| Rust worker with hard-to-cross-compile system deps | `image` | Same container path; ship a Rust-base `Dockerfile`. |

## 4. Linting

Lint configs live at the repo root. Per-worker overrides are allowed but
discouraged.

- **Rust** — runs `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings`. Nothing extra
  to add per worker.
- **Node** — must lint clean against [`biome.json`](biome.json). Run locally:
  `npx @biomejs/biome ci <worker>`.
- **Python** — must lint clean against [`ruff.toml`](ruff.toml). Run locally:
  `ruff check <worker> && ruff format --check <worker>`.

## 5. Tests

The standard CI runner per language. The `tests/` folder must exist and be
non-empty.

- **Rust** — `tests/integration.rs` using `#[tokio::test]`. Either call
  handler functions directly (preferred for fast tests) or boot the worker as
  a subprocess. CI runs `cargo test --all-features`.
- **Node** — `tests/*.test.ts` using [Vitest](https://vitest.dev). Add
  `vitest` to `devDependencies` and define a `test` script. Suggested:

  ```json
  "scripts": {
    "test": "vitest run"
  }
  ```

- **Python** — `tests/test_*.py` using `pytest`. Add `pytest` to a
  `[project.optional-dependencies] dev` group; CI installs with
  `pip install -e .[dev]`.

See [`image-resize/tests/integration.rs`](image-resize/tests/integration.rs),
[`todo-worker/tests/handlers.test.ts`](todo-worker/tests/handlers.test.ts),
and [`todo-worker-python/tests/test_handlers.py`](todo-worker-python/tests/test_handlers.py)
for working examples.

## 6. Pull request flow

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) will:

1. Discover which worker folders changed in this PR by reading
   `iii.worker.yaml` in each affected dir.
2. Run `pr-checks` per changed worker:
   - `README.md` exists and is non-empty.
   - `iii.worker.yaml` parses; has `name`, `language`, `deploy`, `manifest`.
   - The manifest version is **strictly greater** than the version on the
     PR's base branch.
   - `tests/` exists and is non-empty.
3. Run lint + tests for the matching language matrix.

A new worker added in a PR satisfies the "version > base" check trivially
(no base version exists yet).

## 7. Releasing

After merge to `main`:

1. Open Actions → **Create Tag**
   ([`.github/workflows/create-tag.yml`](.github/workflows/create-tag.yml)).
2. Pick `worker`, `bump` (`patch` | `minor` | `major`), and `tag`
   (`latest` | `next`).
3. The workflow bumps the manifest, commits to `main`, and pushes an
   annotated tag `<worker>/v<X.Y.Z>` whose body carries `registry-tag: <tag>`.
4. The tag push fires the [`release.yml`](.github/workflows/release.yml)
   dispatcher, which:
   - Creates the GitHub Release.
   - Routes on `deploy`: `binary` → multi-arch binaries to GH Release;
     `image` → multi-arch image to ghcr.io.
   - Calls `POST https://api.workers.iii.dev/publish` with
     `WORKERS_REGISTRY_API_KEY`.

The `tag` you picked (`latest` / `next`) becomes the registry tag attached to
this version, and is atomically moved off any previously-tagged version on
the same worker (see [`openapi.yaml`](openapi.yaml)).

## 8. Worked examples

Use these as templates:

- **Rust binary** — [`image-resize/`](image-resize/): cross-compiled CLI,
  GH Release artifacts, no Dockerfile.
- **Node container** — [`todo-worker/`](todo-worker/): `Dockerfile`,
  `iii.worker.yaml`, handlers tests via `node --test`.
- **Python container** — [`todo-worker-python/`](todo-worker-python/):
  `pyproject.toml` + ruff + `Dockerfile`, pytest under `tests/`.

## 9. Copy-paste skeleton

For an AI agent scaffolding a new worker, the minimal set of files to
materialise is:

`<worker>/iii.worker.yaml`

```yaml
iii: v1
name: <worker>
language: <rust|node|python>
deploy: <binary|image>
manifest: <Cargo.toml|package.json|pyproject.toml>
# bin: iii-<worker>            # rust binary only
description: One-line description.
```

`<worker>/README.md`

```markdown
# <worker>

One paragraph explaining what the worker does, its iii functions, and the
expected `config.yaml` shape.

## Functions

- `<worker>::<function>(input)` → `output`
```

`<worker>/tests/<smoke>.{rs,ts,py}`: at least one assertion against an
exported handler.

`<worker>/skill.md`: top-level skill markdown for the registry; see §10.2
for the content template.

`<worker>/tests/skill.rs` (Rust workers): one assertion that `skill.md` is
well-formed and the SKILL_ID is valid; see §10.6 for the test code.

For `deploy: image`: `<worker>/Dockerfile` that respects `III_URL` and traps
`SIGTERM`.

## 10. Skill registration

Every worker should register a markdown skill on the [`skills` platform
worker](https://workers.iii.dev/workers/skills) at startup so MCP clients
(Claude Desktop, Cursor, MCP Inspector) can discover and orient to its
functions. The skill body lives at `<worker>/skill.md` and is served at
`iii://<worker>`; the auto-rendered `iii://skills` index links every worker.

> The `skills` worker version pinned by this convention is **v0.2.0+** —
> needed for multi-segment ids and `skills::unregister`.

### 10.1 Skill ID validation rules (skills v0.2.0+)

- 1+ segments separated by `/`.
- Each segment: lowercase ASCII letters, digits, `-`, `_`; max 64 chars per segment.
- Total id length ≤ 1024 chars.
- First segment MUST NOT be the literal `fn` (reserved for section URIs).

For workers in this repo, the id equals the folder name — a single segment.

### 10.2 Content shape

`<worker>/skill.md` is loaded into agent context. Keep it small — aim for
1–3 KB; the registry hard cap is 256 KiB. Content is "when and why to use",
not install/configure (that lives in `README.md`). Imperative tone.

```markdown
# <worker-name>

<One-sentence summary used as the description in the iii://skills index.>

## When to use

<Bullet list of agent intents where this worker is the right call.>

## Functions

- `<worker>::<fn>(input) → output` — one-line purpose
- `<worker>::<fn>(input) → output` — one-line purpose

## When NOT to use

<Close-but-different situations where another worker is the right answer.>

## Notes

<Optional: required config, dependencies on other workers, operational caveats.>
```

The heading is `## Functions` (not `## Tools`). iii terminology is
"function" for registered handlers; "tool" is reserved for MCP's own term.

### 10.3 Optional: nested sub-skills

Workers with a wide function surface MAY register additional sub-skills
under slashed paths (`<worker>/<group>`, `<worker>/<group>/<leaf>`). The
top-level `skill.md` should then be a small router that links to the
sub-skills via `[label](iii://<worker>/<group>/...)`. Sub-skill bodies
live under `<worker>/skills/...` and are loaded with `include_str!` from
`src/lib.rs`. Each sub-skill is its own `skills::register` call. Out of
scope for the current 7-worker batch — all are flat.

### 10.4 Wire-up: lib.rs

Expose the id and the embedded markdown as `pub const`s in `src/lib.rs`
so both `main.rs` and the integration tests can reference them:

```rust
// In src/lib.rs (near the top):
pub const SKILL_ID: &str = "<worker>"; // must equal the folder name
pub const SKILL_MD: &str = include_str!("../skill.md");
```

`include_str!("../skill.md")` resolves from `src/lib.rs`, so the file must
be at the worker root. Don't use `env!("CARGO_PKG_NAME")` — that returns
`iii-<worker>` (with the `iii-` prefix) for these workers, not the
folder name.

### 10.5 Wire-up: main.rs

Add three small helpers to `main.rs` and call them from `main`:

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iii_sdk::TriggerRequest;
use serde_json::json;

// Background task — fires AFTER the worker's register_with_iii() returns,
// so the skill never advertises functions that aren't registered yet.
fn spawn_skill_register(iii: Arc<iii_sdk::III>) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let started = Instant::now();
        loop {
            let res = iii
                .trigger(TriggerRequest {
                    function_id: "skills::register".into(),
                    payload: json!({
                        "id": <crate>::SKILL_ID,
                        "skill": <crate>::SKILL_MD,
                    }),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await;
            match res {
                Ok(_) => {
                    log::info!("registered skill: {}", <crate>::SKILL_ID);
                    return;
                }
                Err(e) => {
                    if started.elapsed() > Duration::from_secs(180) {
                        log::warn!(
                            "skills handshake gave up for {}; install/start the skills worker and restart (last error: {e})",
                            <crate>::SKILL_ID
                        );
                        return;
                    }
                    log::debug!(
                        "skills::register failed for {}: {e}; retrying in {backoff:?}",
                        <crate>::SKILL_ID
                    );
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    });
}

// Catches BOTH SIGINT (Ctrl-C in dev) and SIGTERM (container kill) so the
// unregister below runs in production container shutdown, not just dev.
async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}

// Best-effort: a missed unregister is self-healing on next boot's re-register.
async fn unregister_skill(iii: &Arc<iii_sdk::III>) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: json!({ "id": <crate>::SKILL_ID }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}
```

Replace `<crate>` with the worker's library crate name (e.g.
`auth_credentials`, `auth_rbac`).

In `main()`, immediately after the worker's existing
`register_with_iii(...)` call, replace the old `tokio::signal::ctrl_c().await.ok();`
line with three calls:

```rust
// After register_with_iii(...) succeeds:
spawn_skill_register(iii.clone());

wait_for_shutdown().await?;

unregister_skill(&iii).await;
Ok(())
```

### 10.6 Tests

Add `<worker>/tests/skill.rs` with two assertions. They run as part of
`cargo test` — no iii engine needed.

```rust
//! Compile-time and format checks for the registered skill.
//! Runs without an iii engine connection.
//!
//! Single-segment skill id checks. Workers in this repo all use a flat,
//! folder-name-equals-skill-id convention (see §10.1). If a future worker
//! adopts nested sub-skills, replace these tests with multi-segment-aware
//! variants.

#[test]
fn skill_md_well_formed() {
    let skill = <crate>::SKILL_MD;
    assert!(!skill.trim().is_empty(), "skill.md is empty");
    assert!(
        skill.len() <= 256 * 1024,
        "skill.md exceeds 256 KiB ({} bytes)",
        skill.len()
    );
    let first_non_blank = skill.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    assert!(
        first_non_blank.starts_with("# "),
        "skill.md must start with an H1, got: {first_non_blank:?}"
    );
    assert!(
        skill.lines().any(|l| l.trim() == "## Functions"),
        "skill.md must contain a `## Functions` section"
    );
}

#[test]
fn skill_id_is_valid() {
    let id = <crate>::SKILL_ID;
    assert!(!id.is_empty(), "SKILL_ID is empty");
    assert!(id.len() <= 64, "SKILL_ID exceeds 64 chars");
    // `fn` is the only reserved first-segment literal as of skills v0.2.0.
    assert_ne!(id, "fn", "SKILL_ID must not be the reserved literal `fn`");

    let first = id.chars().next().unwrap();
    assert!(
        first.is_ascii_lowercase() || first.is_ascii_digit(),
        "SKILL_ID first char must be lowercase ASCII letter or digit"
    );
    assert!(
        id.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "SKILL_ID has invalid characters"
    );
}
```

### 10.7 Lifecycle summary

```
boot:
  register_worker()  →  configure_store/cfg  →  register_with_iii()  →  serve traffic
                                                       │
                                                       ▼  (spawn here, async)
                                           skills::register
                                           retry 5s → 60s, give up at 180s

shutdown (SIGINT or SIGTERM):
  wait_for_shutdown()  →  skills::unregister (2s timeout, errors swallowed)  →  exit
```
