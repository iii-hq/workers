---
target: new-session system prompt selector
total_score: 22
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 2
timestamp: 2026-08-07T18-38-01Z
slug: console-web-src-components-chat-emptystate-tsx
---
Method: dual-agent (A: critique_design_retry · B: critique_evidence)

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|---|---:|---|
| 1 | Visibility of System Status | 2 | Prompt loading and selection resolution expose no loading, success, or failure state. |
| 2 | Match System / Real World | 2 | “system prompt,” “default,” “enrich,” and “replace” assume agent-domain knowledge. |
| 3 | User Control and Freedom | 3 | The choice can be changed before sending, but cannot be undone after the session locks. |
| 4 | Consistency and Standards | 3 | Picker focus and shadow treatment diverge from iii Schematic tokens. |
| 5 | Error Prevention | 2 | Named prompts expose only names before an irreversible choice. |
| 6 | Recognition Rather Than Recall | 2 | Descriptions live in hover-only title text and strategy meaning must be remembered. |
| 7 | Flexibility and Efficiency | 2 | Saved prompts form an unsearchable, data-dependent list. |
| 8 | Aesthetic and Minimalist Design | 3 | The screen is clean, but its consequential setting reads as a weak footnote. |
| 9 | Error Recovery | 1 | Fetch failures silently empty the list or reset the choice to default. |
| 10 | Help and Documentation | 2 | Permanence is stated, but the safe default and strategy consequences are not explained in context. |
| **Total** | | **22/40** | **Acceptable — significant clarity work remains.** |

## Design Specificity Verdict

**LLM assessment:** The `$ new session` eyebrow, lowercase technical voice, warm paper palette, and function identifiers feel authored for iii. The system-prompt area does not: a generic document icon, context-free “default” value, and conditional text toggle make a distinctive built-in/enrichment model look like an interchangeable settings filter.

**Deterministic scan:** The CLI detector scanned `EmptyState.tsx` and its direct `SystemPromptPicker.tsx` dependency. Both completed with exit code 0 and produced zero findings. This agrees that the problem is not a detectable anti-pattern; it is hierarchy, consequence clarity, interaction semantics, and product specificity.

**Visual overlays:** No browser mutation API was available, so no overlay was injected or claimed. The two supplied screenshots and corrected feature-worktree source were used as static fallback evidence.

## Overall Impression

The welcome builds warmth and technical confidence, then ends on a permanent choice that looks incidental and unexplained. The single biggest opportunity is to turn that footer-like row into a compact session preflight that explains the safe default and shows the exact consequence of a named-prompt strategy.

## What’s Working

1. Progressive disclosure keeps strategy controls out of the way until a non-default prompt makes them relevant.
2. The permanence warning is colocated with the control and correctly states both scope and timing.

## Priority Issues

### P1 — Irreversible choice without inspectable consequences

- **Why it matters:** Users cannot tell what “default” means, what a saved prompt contains, or whether “enrich” versus “replace” is safe before the choice locks.
- **Fix:** Give the setting its own preflight surface, explain the default inline, show prompt descriptions in the menu, and render both strategies as explicit choices with one-line consequences.
- **Suggested command:** `$impeccable clarify`

### P1 — Silent asynchronous failure changes meaning

- **Why it matters:** A directory failure looks like an empty list; a prompt fetch failure silently falls back to default, so the session may start with instructions the user did not choose.
- **Fix:** Preserve the prior selection and show an inline loading/error state with a retry path.
- **Suggested command:** `$impeccable harden`

### P2 — Prompt discovery does not scale

- **Why it matters:** Once four or more named prompts exist, the flat menu exceeds the recommended visible-choice limit and forces scanning without useful context.
- **Fix:** Show descriptions beneath names now; add search only when real prompt counts justify it.
- **Suggested command:** `$impeccable distill`

### P3 — Picker chrome loses iii Schematic specificity

- **Why it matters:** Generic border/focus/shadow treatment weakens the fill-led visual language and produces a less visible keyboard focus state.
- **Fix:** Use the existing rounded surface, focus-ring, selected-fill, and floating-shadow tokens.
- **Suggested command:** `$impeccable polish`

## Persona Red Flags

**Jordan (First-Timer):** “default,” “enrich,” and “replace” are undefined; prompt contents cannot be previewed; the lock warning raises stakes without identifying the safe choice.

**Sam (Accessibility-Dependent):** Prompt descriptions rely on `title`; async state changes are not announced; focus uses a subtle border swap rather than the established visible focus ring.

**Alex (Power User):** Long saved-prompt lists have no search, and inspecting or authoring a prompt requires leaving chat for the directory UI.

## Minor Observations

- Long selected names truncate at `10rem` without exposing their description.
- The conditional strategy button changes row width and feels appended rather than part of one decision.
- In shorter panels, the setting can fall below the fold after the orientation copy.

## Questions to Consider

1. If the choice locks after the first message, why should it look like a low-stakes filter instead of a session preflight?
2. Can the final onboarding beat state exactly what the agent receives so users finish with confidence rather than caution?
