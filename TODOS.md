# Workers TODOs

## iii-sdk: register/unregister helpers for skill registration

**What:** Add `iii.register_skill_with_retry(id, markdown)` and `iii.unregister_skill(id)` to `iii-sdk`. Encapsulates the canonical 5s→60s→180s backoff + give-up loop and the 2s best-effort unregister currently inlined per worker.

**Why:** Each iii worker that registers a skill duplicates ~25 LOC of boot snippet + ~10 LOC of shutdown snippet. With 7 workers in this branch and more to come (harness, image-resize, mcp, iii-lsp, etc.), that's 175+ LOC of mechanical copy-paste. DRY violation; future tuning of retry policy needs N edits instead of 1.

**Fix:** ship the two helpers in `iii-sdk` (whatever crate that is upstream), version-bump, then replace each worker's inline snippet with the one-line call. Update `AGENTS-NEW-WORKER.md` §10 to use the helpers in its example.

**Effort:** ~30 LOC in iii-sdk + tests + version bump + replace 7 inlined sites. Coordination with the iii-hq/workers repo for the SDK release.

**Tracked here because:** publishing-map rule is "no cross-worker path dependencies" so the helpers can't be vendored; they have to live in iii-sdk. Out of scope for the skill-registration PR.

## CI: enforce `<worker>/skill.md` presence in pr-checks

**What:** Extend the per-worker `pr-checks` step in `.github/workflows/ci.yml` with three checks: `<worker>/skill.md` exists, is non-empty after trim, is ≤ 256 KiB. Three named failure messages (missing / empty / oversize), each pointing at `AGENTS-NEW-WORKER.md` §10.

**Why:** Skill registration is currently doc-driven only. New workers can ship without a skill and nothing in CI flags it; the runtime signal is absence from `iii://skills`, which is easy to miss. Add the gate when silent-skip becomes a recurring problem.

**Fix:** ~8 lines of shell inside the existing `pr-checks` loop. No new job. Same shape as the existing `README.md` check, plus the 256 KiB cap.

**Effort:** ~30 min including failure-message wording and a smoke test.

**Tracked here because:** the original skill-registration plan deliberately deferred the CI gate. Capturing the design here so it isn't re-derived when we decide to enforce.

## harness: migrate tool filter from prefix denylist to `metadata.mcp.expose`

**What:** Replace the prefix denylist in `turn-orchestrator/src/tools_catalog.rs::DENIED_PREFIXES` with `metadata.mcp.expose === true`. Touch each LLM-callable function's `register.rs` to set `metadata.mcp.expose = true` alongside the existing `metadata.tool.label`.

**Why:** Single shared advertise-selector across mcp + harness. The denylist forks the convention — every new control-plane prefix has to be added by hand to the harness list. `metadata.mcp.expose` already exists in `mcp/src/tools.rs:44-63` with passing tests at `:280-292`.

**Fix:** ~3-5 LOC per affected register site (shell-bash, shell-filesystem, subagent, skills::skill::fetch). Then collapse `should_advertise` in `tools_catalog.rs` to a one-line metadata read.

**Effort:** ~30 LOC of register-site touches + ~10 LOC of harness filter rewrite + test updates.

**Tracked here because:** plan-eng-review D3 chose denylist for the slim-harness PR to minimize register-site churn. After D6 added `metadata.tool.label` to the same files, the marginal cost of `expose: true` is one JSON line per call.

**Depends on:** the slim-harness PR landing (denylist must exist as the thing being replaced).

## harness: extract `unwrap_function_list` + `format_to_input_schema` to a shared crate

**What:** Move the two helpers from `mcp/src/tools.rs:67-94` and the copy in `turn-orchestrator/src/tools_catalog.rs:48-82` into a shared module — `harness-types`, a new `iii-tool-catalog` crate, or upstream into `iii-sdk`.

**Why:** Plan-eng-review D5 chose copy-paste; the copies will drift the first time the engine envelope changes or `format_to_input_schema` gets a bug fix. mcp tests pass while harness silently breaks (or vice versa).

**Fix:** Pick a home, move the two functions + their mcp tests, import from both crates. The functions are pure, stateless, ~20 LOC total.

**Effort:** ~15 min once the home is chosen. Picking the home is the bulk of the decision (harness-types is a types-not-utilities crate; new crate adds a workspace member; iii-sdk needs an upstream PR + version bump).

**Tracked here because:** the slim-harness PR copy-pasted intentionally to keep crate dep edges flat. Drift risk is real but contained; consolidate when the next bug or schema-format change touches either copy.

**Depends on:** the slim-harness PR landing (so there's a second copy to consolidate).
