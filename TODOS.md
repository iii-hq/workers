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
