---
target: system prompt dialog (session chip)
total_score: 22
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 3
timestamp: 2026-08-07T17-10-09Z
slug: iii-directory-ui-src-session-chip-index-tsx
---
Method: dual-agent (A: design-review subagent · B: detector+browser-evidence subagent)

Mode: **Operate**. Target: the system-prompt dialog — `iii-directory/ui/src/session-chip/index.tsx` (dialog JSX), `iii-directory/ui/styles.css` (`.dir-ui-sysprompt-*`), composing `console/web/src/components/ui/Dialog.tsx` and `MarkdownPreview.tsx`.

Evidence: A read the user's screenshot (default state, long openai-codex prompt) + all five sources + DESIGN.md. B ran `detect.mjs` (exit 0, **0 findings** on all four files) and drove the live console: opened the dialog on the `ptbr · replace` session, measured computed styles, captured light+dark screenshots.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 2 | Bare `loading…`, no reserved height, no live region; enrich's two async loads pop in at different times |
| 2 | Match System / Real World | 3 | Send-order fidelity is excellent; the seam the model actually sees between enrich's halves is hidden |
| 3 | User Control and Freedom | 2 | Esc/✕ work; dead end otherwise — and on long content the ✕ scrolls away with the body |
| 4 | Consistency and Standards | 2 | Part label uppercases the filename the chip itself refuses to uppercase; `border: var(--color-rule)` on the body is transparent — inert paint |
| 5 | Error Prevention | 2 | Fetch failure impersonates "provider serves none" — plants a false debugging lead |
| 6 | Recognition Rather Than Recall | 3 | Strategy restated per open; provider named only in the default state |
| 7 | Flexibility and Efficiency | 1 | No copy, no size readout, no power-user affordance on a power-user surface |
| 8 | Aesthetic and Minimalist Design | 3 | Quiet and content-dominant; docked for the heading-scale inversion and potential triple-nested scroll |
| 9 | Error Recovery | 1 | Catch → `DEFAULT_UNAVAILABLE` (false claim); deleted file → silent stale snapshot |
| 10 | Help and Documentation | 3 | State descriptions double as inline docs; `DEFAULT_UNAVAILABLE` explains harness internals honestly |
| **Total** | | **22/40** | **Acceptable — significant improvements needed** |

## Design Specificity Verdict

**LLM assessment (A):** The semantics are authored; the presentation is borrowed. Send-order two-part rendering, fresh-from-disk reads with a snapshot net, and the honest built-in-fallback copy could only belong to this product. But the document itself renders through MarkdownPreview at full-page README scale inside a stock centered modal — the reading experience the dialog exists for is category-interchangeable. Authored frame, unadapted content.

**Deterministic scan (B):** 0 findings across all four files. The detector is silent while the design review found six priority issues — every defect here is semantic (error-copy truthfulness, scroll architecture, missing affordances), invisible to static scanning. No false positives to reconcile.

**Live evidence (B):** dialog 672px wide, `max-h 765px`, `overflow-y auto` on the DialogContent itself; only button is "close"; `role=dialog` + labelledby + describedby correctly wired. B's screenshots suggested a missing modal scrim — **refuted by a follow-up runtime probe**: the overlay is mounted full-viewport at opacity 1 painting `oklab(0 0 0 / 0.6)` and is topmost by hit-test. Recorded as Assessment B's one false positive. The kernel that survives: in dark, the near-black panel over a dimmed near-black page separates mainly by `shadow-floating` — adequate, worth watching.

## What's Working

- **Send-order fidelity** — enrich renders `built-in` then `appended · name` in transmission order: wire truth, not a config abstraction.
- **Fresh-from-disk with a snapshot net** — the dialog never lies about a file edited mid-session.
- **State copy quality** — each state's first sentence is load-bearing; the default one preemptively answers "how do I change this."
- **Correct dialog ARIA** — role/labelledby/describedby all present (B-verified).

## Priority Issues

- **[P1] The prompt is unreadable by keyboard and unannounced to SR.** The scroll region has no tabindex/role/label; `loading…` swaps to content silently. Keyboard users cannot read past the fold — on a surface whose only job is reading.
- **[P1] The copy job is unserved.** B confirmed: the only button is "close". No copy affordance, no size readout, for the largest fixed token spend in every turn.
- **[P1] Fetch failure impersonates absence.** `.catch(() => setDefaultBody(DEFAULT_UNAVAILABLE))` reports a transient router/engine failure as "the provider serves no identity prompt" — a false lead at exactly the debugging moment.
- **[P2] Deleted file → silent stale snapshot.** The fallback shows session-start text with zero marking, undercutting the fresh-read promise.
- **[P2] Part label fails three ways at once.** ink-ghost ≈2.4:1 at 10px; uppercase mangles `pt-BR`; it's a `<p>`, invisible to SR outline while the prompt's own h1s dominate both visually (20px vs the 13px dialog title) and structurally.
- **[P2] Scroll architecture.** Two independent 38vh regions + whole-dialog `overflow-y auto` can triple-nest scrolling in enrich, and the absolutely-positioned ✕ scrolls out of reach. One scroll context with a pinned header is the fix.

## Persona Red Flags

**Alex** — no copy button, no token count; manual text-selection out of a 38vh box is the only export path. **Sam** — unfocusable unlabeled scroll region (Safari can't focus it at all); 2.4:1 part labels; silent loading swap; the primitive's ✕ is a 14px icon with no padded hit area. **Riley** — 50k-token prompt renders into one 38vh box; empty prompt renders a blank slab; network error wears the "provider serves none" costume; deleted file shows unmarked stale text.

## Minor Observations

- Enrich's two async loads reflow the centered dialog twice; no reserved loading height.
- The body's only edge is a fill delta (#f2f0ed on #f7f5f2) — the transparent `--color-rule` border contributes nothing (borders were missed by the earlier repo paint sweep, which only checked backgrounds).
- A prompt using `####` produces h4s that mimic the part labels — two label systems colliding.
- Default state's focus trap contains exactly one tab stop (the ✕).

## Questions to Consider

1. The console meters context in the header — why does the surface displaying the largest fixed spend not put a number on it? *(addressed in the fix pass)*
2. What byte-level separator sits at enrich's seam, and should the dialog show it?
3. Is a modal the right container at all — should this be a receipt (provider, name, strategy, sizes) with a jump to the real document in the directory page, rather than a glass case holding the whole thing?
