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

## harness/web: port PR #150 ApprovalRow + reducer changes (deferred from harness-node port)

**What:** PR #150 includes `harness/web/src/{App.tsx,components/ApprovalRow.tsx,components/ApprovalRow.test.tsx,reducer.ts,reducer.test.ts,types.ts,tests/e2e/approval.spec.ts}` changes that make the new pending/approved/denied approval-gate states render correctly in the web console. The harness-node port (branch `feat/console-catalog-model-keys`, plan `~/.claude/plans/look-at-the-pr-velvety-castle.md`) explicitly scopes these out.

**Why:** Once the harness-node port lands, the runtime emits the new envelope shape (`status: pending`, `subscriber: approval-gate`, `approval_gate: true`, `denial: {kind, detail}`). Without the corresponding web UI changes, the console may show pre-PR-150 behavior (e.g. doesn't distinguish pending from denied) until somebody backports the harness/web half.

**Fix:** Read the `harness/web/*` files from PR #150 and apply verbatim (or as close as the local harness/web state allows). Confirm the e2e test (`tests/e2e/approval.spec.ts`) covers the new flow.

**Effort:** ~3 h (per the original planning estimate).

**Tracked here because:** the harness-node port handles the runtime/server half; this captures the UI half that must follow to complete the user-facing experience. Likely owned by whoever lands the Rust PR #150, but recording here in case the harness-node port lands first and the web UI lags.

## console/web: replace native `<select>` with a custom Listbox if Safari optgroup styling becomes a real complaint

**What:** `console/web/src/components/chat/ModelPicker.tsx` renders a native `<select>` with `<optgroup>` labels for each provider. `console/web/src/index.css` flattens `optgroup` font-style/weight/color so Chrome and Firefox render provider headings in the mono-lowercase aesthetic. Safari ignores most `optgroup` CSS — provider labels stay italic there.

**Why:** Native `<select>` is correct, accessible, keyboard-navigable, and free. The Safari italic is a minor visual inconsistency, not a functional bug. Replacing the picker today would cost ~150+ lines of custom Listbox/Popover code plus full ARIA listbox semantics for a small composer-footer control that's rarely open.

**Fix:** If/when Safari styling becomes a real complaint, replace `Select<ModelId>` in `ModelPicker` with a custom Popover/Listbox component. Keep the existing `groups` data shape (it transfers 1:1). Drop the `select optgroup` CSS block in `index.css` once the native element is gone.

**Effort:** ~150 lines + tests + manual a11y verification (keyboard nav, focus trap, screen reader announce).

**Tracked here because:** the model picker grouping change (branch `feat/console-catalog-model-keys`) explicitly scoped this out. Recording the trigger condition ("Safari styling complaint") so we don't pre-build the custom dropdown speculatively.
