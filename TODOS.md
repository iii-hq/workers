# Workers TODOs

> Closed by the iii-directory migration (2026-05-12):
> - `iii-sdk: register/unregister helpers` — the migration replaced the
>   per-worker `skills::register` snippet with file-based publish via
>   `iii-directory`. There is no boot-time RPC to wrap.
> - `CI: enforce <worker>/skill.md presence` — added to
>   `.github/workflows/ci.yml`'s `pr-checks` job as part of the migration's
>   C3 guard (per-bootstrap-worker presence + non-empty + ≤256 KiB).

## mcp: extract `unwrap_function_list` + `format_to_input_schema` (harness side gone)

**What:** `mcp/src/tools.rs:67-94` still has the two helpers. The harness's copy in `agent_call.rs` was deleted as part of the Tier 2 thin-dispatcher refactor (spec `docs/superpowers/specs/2026-05-07-tier2-iii-pure-harness-design.md`).

**Why:** Drift risk dropped — only one copy now — but if any future caller in workers/ wants the same pattern, it's worth extracting before the second copy comes back.

**Fix:** When a second caller appears, move the helpers into `harness-types`, a new `iii-tool-catalog` crate, or upstream into `iii-sdk`. Until then, leave the mcp copy where it is.

**Effort:** ~15 min when triggered. Picking the home is the bulk of the decision.

**Tracked here because:** the iii-native + Tier 2 refactors made this less urgent. Capturing the design here so it isn't re-derived when the second caller emerges.
