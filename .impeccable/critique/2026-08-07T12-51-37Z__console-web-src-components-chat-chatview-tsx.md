---
target: chat top bar (ChatView PageHeader + injected session chips)
total_score: 22
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 3
timestamp: 2026-08-07T12-51-37Z
slug: console-web-src-components-chat-chatview-tsx
---
⚠️ DEGRADED: single-context (standing session instruction: no sub-agents unless requested)

Mode: **Operate**. Target: the chat pane top bar — `console/web/src/components/chat/ChatView.tsx:1676-1763`, plus the two injected chips it hosts (`harness/ui/src/context-chip`, `iii-directory/ui/src/session-chip`) and `PageChrome.tsx:PageHeader`.

Evidence: the supplied screenshot (613×173, upscaled 3× for inspection), full source of all five components, and `detect.mjs` on the five files. No browser overlay pass — the deterministic scan and the screenshot cover this surface, and injecting into the live console at :3113 would have needed a separate browsing session.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 2 | The ctx meter's 56px track paints `--color-rule-2` = `transparent`, so the bar conveys nothing at a glance; status change is never announced to a screen reader |
| 2 | Match System / Real World | 3 | Vocabulary is genuinely this product's (`ctx`, `ready`, `$`); only `SYSTEM:` and `ENRICH` are internal category-speak |
| 3 | User Control and Freedom | 2 | Read-only by design, but the dialog dead-ends — no "start a chat with this prompt" exit |
| 4 | Consistency and Standards | 2 | Three interactive items, three affordance treatments, one inert item identical to all of them; 12px rhythm breaks to 6px at the ✕ |
| 5 | Error Prevention | 3 | Export correctly disables when empty and says why; nothing destructive lives here |
| 6 | Recognition Rather Than Recall | 3 | Everything is labelled, no icon-only nav — but which items are clickable must be discovered by hovering |
| 7 | Flexibility and Efficiency | 2 | No shortcuts; focus indicator differs across three adjacent controls; no overflow strategy as chips multiply |
| 8 | Aesthetic and Minimalist Design | 2 | Five items at one weight; three numbers for one fact; a dead 56px gap where a track should be |
| 9 | Error Recovery | 1 | `error` state parks its only explanation in a `title` tooltip — not keyboard-reachable, not SR-reachable, gone on touch |
| 10 | Help and Documentation | 2 | Tooltip copy is genuinely thoughtful, but `title` is the only channel |
| **Total** | | **22/40** | **Acceptable — significant improvements needed** |

## Design Specificity Verdict

**LLM assessment:** This is not category-interchangeable, and that is worth saying first. The mono voice, the `$` prompt glyph, the lowercase/uppercase split, the refusal to draw lines — you could not lift this bar into a generic SaaS product without it looking stolen. The specificity is real.

The problem is that the specificity is applied *uniformly*. Every item gets the same 11px mono uppercase 0.06em treatment in the same faint-ink band, so the bar has texture but no hierarchy. It reads like a terminal status line — which is the intent — but a terminal status line earns its flatness by being one line emitted by one system. This one aggregates four independent producers (harness worker, iii-directory worker, console chrome, pane chrome) and applies no compositional rule between them.

That is the real finding: **the top bar has quietly become an extension surface with no layout contract.** `host.chat.registerSessionChip` is open to any worker. Two use it today. There is no spec for what a chip may look like, how it shrinks, or which tokens it may paint with — so the harness chip draws a progress track in a colour the console zeroed out, the directory chip renders bare text with no focus style, and the console interleaves dividers that also don't paint. It works today only because both chip authors are the same person.

**Deterministic scan:** `detect.mjs` returned 2 advisory findings, both `design-system-font-size` (10px off the DESIGN.md ramp) at `ChatView.tsx:1753` and `:1872` — the "copied" flash and the queued-message hint. Neither is in the top-bar cluster. The detector is silent on this surface, which is itself informative: the defects here are *semantic token misuse*, not literal off-ramp values. `bg-rule-2` is a legitimate token name; nothing static can know it resolves to `transparent`.

**Visual overlays:** not available — no injection pass was run, so no user-visible overlay exists in a browser tab.

## Overall Impression

Calm, dense, on-brand, and quietly broken in two places that a screenshot proves instantly. The two things you most recently asked for — dividers between components, and a legible context meter — are both in the code and both invisible on screen, for the same reason. Fix that root cause and the bar improves more than any restyling would.

The biggest opportunity is hierarchy: decide which one thing in this bar deserves to be read first, and let the other four recede.

## What's Working

- **Nothing is icon-only.** Every control carries a text label. That is rarer than it should be in engineer-facing chrome, and it is why Jordan-class users are not stranded here.
- **The tooltip copy is genuinely good.** "download session as markdown — paste into another AI for analysis" and "matches iii.session.id in the traces tab" teach the workflow, not the widget. Most products would have written "Export" and "Session ID".
- **`tabular-nums` on every number.** The percentage and token counts don't jitter as they tick. Small, correct, easy to skip.
- **The disabled-export state explains itself** ("no messages yet — nothing to export") instead of just greying out.

## Priority Issues

### [P1] Two header elements paint with `--color-rule*`, which this design system defines as `transparent`

`HeaderDivider` (`ChatView.tsx:103`) is `bg-rule-2`. The harness context chip's track (`harness/ui/styles.css:40-48`) is `background: var(--color-rule-2); border: 1px solid var(--color-rule)`. `index.css:51-53` sets all three rule tokens to `transparent` in both themes — "the system draws no lines."

**Why it matters:** the dividers you specifically asked for don't exist on screen, and the context meter is a 56px void with a 3px orange fill floating in it. In the screenshot it reads as `CTX ▪ ⟨dead space⟩ 6%` — a stray dot, not a gauge. Both are silent failures: the code is present, reviewed, and shipped, and the pixels are absent.

**Fix:** `HeaderDivider` → `bg-edge` (the one sanctioned stroke, `index.css:59-61`). Harness track → `background: var(--color-surface)`, drop the transparent border. Then grep the repo for every other `rule`/`rule-2` used as a *paint* rather than as an inert legacy border — this class of bug is not limited to these two.

**Suggested command:** `/impeccable polish`

### [P1] Five items, one weight — the bar has no entry point

`CTX 6% 6.3k/107k`, `SYSTEM: PTBR · ENRICH`, `⤓ EXPORT`, `● READY` are all 11px mono uppercase tracked at 0.06em in `ink-faint`. The one item that changes moment to moment (status) and the one item that is an action (export) are visually identical to two static read-outs.

**Why it matters:** in an Operate surface the bar's job is peripheral glanceability — you should be able to catch "still working" or "context is nearly full" without reading. Right now everything must be read, left to right, every time. That is the definition of extraneous cognitive load.

**Fix:** pick one primary. Status is the honest answer — it is the only item whose value you need continuously. Give it the ink, a real `ok` tone instead of `bg-ink` black for ready, and let the rest sit at `ink-faint`/`ink-ghost`. Then cut `6.3k/107k` from the bar entirely: it is three numbers for one fact, and the popover the chip already opens is where detail belongs.

**Suggested command:** `/impeccable layout`

### [P1] `ink-ghost` on `panel-raised` is 2.4:1 — the token counts fail WCAG AA by a wide margin

`--color-ink-ghost: #a3a09c` on `--color-panel-raised: #f7f5f2` computes to **2.39:1**. AA needs 4.5:1 for 11px text. This is the colour carrying `6.3k/107k` and the `·` separators. (`ink-faint` #6b6865 measures 5.09:1 and is fine.)

**Why it matters:** it is not decorative text — it is the absolute token numbers, the thing you'd squint at when deciding whether to compact. Sam cannot read it; on a laptop in daylight, nobody can.

**Fix:** if the counts stay, move them to `ink-faint`. If they move into the popover (see above), the problem leaves the bar with them. Either way, audit `ink-ghost` usage repo-wide for load-bearing text — it is defensible for the `·` glyph and indefensible for numbers.

**Suggested command:** `/impeccable audit`

### [P2] Interactive and inert are indistinguishable, and focus behaves three different ways

`ctx` (opens a popover), `system: …` (opens the dialog), and `export` (opens a dropdown) are all buttons; `ready` is not. All four look the same at rest, and the only affordance is a colour shift on hover. Two of the three open overlays with no `▾` to say so. Meanwhile `ExportSessionButton` uses `focus-visible:outline-accent`, the ✕ uses `focus-visible:ring-rule-focus`, and both injected chips define no focus style at all — three indicators across five adjacent controls.

**Why it matters:** Alex tabs into this row and cannot tell where focus landed or what the current thing does. Casey on touch has no hover, so *nothing* here is discoverable — and hit areas are the ~11px text glyph box, well under the 24×24 minimum in WCAG 2.2 SC 2.5.8.

**Fix:** give interactive chips the `button-pill` treatment already in the design system (`bg-surface`, 6px radius, `6px 12px` padding) — that solves affordance, hit target, and grouping in one move, and it is a token that already exists. Add a `▾` to the two that open overlays. Publish the console's focus ring as the required convention for injected chips.

**Suggested command:** `/impeccable harden`

### [P2] No overflow strategy on an open extension surface

The actions cluster is `shrink-0` inside a `whitespace-nowrap` header, and `PageHeader` gives the children slot the only `min-w-0 flex-1`. Two workers register chips today. Any worker can. At four or five the cluster will push the model/mode/session-id run to zero width and then overflow the pane — and in `dock` density (`px-4`, narrow column) that happens sooner.

**Why it matters:** this is the failure mode where the bar looks fine for you and breaks for the next person who ships a chip. Riley finds it in thirty seconds by opening the dock.

**Fix:** cap the cluster, let chips past N collapse into a single `⋯` popover, and write the shrink contract into `docs/sops/injectable-console-ui.md` next to the chip API. Decide the priority order now (status never collapses; ctx collapses last).

**Suggested command:** `/impeccable adapt`

### [P2] The `error` state hides its explanation in a `title` attribute

`ChatView.tsx:1711-1715` puts `conversation.statusReason` in the tooltip of the status word. That is the only place a failed session explains itself in this bar.

**Why it matters:** `title` is unreachable by keyboard, unread by most screen readers, invisible on touch, and appears after a ~1s delay. The single highest-stakes moment on this surface has the weakest possible delivery. Nor is the ready→working→error transition announced: `StatusDot` is `aria-hidden` and the label is a plain `<span>` with no `role="status"`.

**Fix:** make the status a button when `status === 'error'` that opens the reason inline or scrolls to the failing message; add `role="status"` to the label span so transitions announce through the live region already mounted at `ChatView.tsx:1788`.

**Suggested command:** `/impeccable harden`

## Persona Red Flags

**Alex (Power User)** — Tabs into the header and hits five focusables with three different focus indicators, two of which are unstyled. No shortcut for export, none for the context popover. Reads `6.3k/107k` every time because the meter that should replace that reading is invisible. Will learn the tooltips once and then never look at the bar again — which means the ctx warning at 75% is wasted on the person most likely to hit it.

**Sam (Screen reader / low vision)** — `6.3k/107k` at 2.4:1 is unreadable; at 200% zoom the nowrap cluster pushes the session-id run off the pane. The ctx `role="progressbar"` announces correctly but has no visible counterpart, so a low-vision user and a sighted user are getting *different* interfaces from the same markup. Session status changes silently — no `role="status"`, dot is `aria-hidden`. Error reason lives in a `title` and never reaches the SR buffer.

**Riley (Stress tester)** — Opens the chat in dock density: the header is `px-4` and the actions still `shrink-0`, so the model name and session id truncate to nothing while four chips hold full width. Registers a third chip from a scratch worker and the row overflows the pane with no clipping rule. Picks a system prompt whose filename is `pt-BR`; the chip uppercases it to `PT-BR`, so the label no longer matches the file on disk. Sets a prompt named `staging-review-v2-with-the-long-name` and watches it push everything else out — the chip has `white-space: nowrap` and no `max-width`.

## Minor Observations

- **`SYSTEM:` earns nothing.** Seven characters of category label in front of a two-token value. `ptbr · enrich` alone is unambiguous in this row; the tooltip already carries the full sentence.
- **Uppercasing a filename is lossy.** `PTBR` is a user-authored identifier that could be `ptBR` or `pt-BR` on disk. The row's uppercase is a style rule; the prompt name is data. Exempt it.
- **Ready is black.** `tone='ink'` for the healthy idle state, while the palette has `ok` (#356f3d / #36c98f). A solid black dot reads as "off" or "unknown" more than "ready". Working (accent + pulse) and error (alert) are both right.
- **Spacing breaks at the ✕.** The inner row is `gap-3` (12px); `PageHeader`'s actions wrapper is `gap-1.5` (6px). So every gap in the cluster is 12px except the last one, which is 6px — and it is the gap before the only control that destroys something.
- **The ✕ is a different visual class sitting flush against a text run.** 28px hover-boxed icon button, no separation from `READY`. It reads as part of the status group.
- **`6.3k/107k` opts out of the row's typography** (`text-transform: none; letter-spacing: normal`) to stay legible. That is the correct call locally and a signal globally: the uppercase-tracked treatment is wrong for data, and this is the one place that already admits it.
- **The dialog dead-ends.** It explains the prompt beautifully and offers no next step. Read-only is the right decision; "open in directory" or "new chat with this prompt" would close the loop without reintroducing mid-session mutation.

## Questions to Consider

- If you could only keep **one** item in this bar, which is it? The answer names your primary, and everything else should visibly step back from it.
- The ctx chip already opens a popover with a full breakdown. Why is any of that detail also on the bar?
- What does this row look like when five workers have registered chips? That is not hypothetical — it is the API you shipped.
- The design system says "no lines," and you have now asked for dividers twice. Is the real need separation, or *grouping* — read-outs in one surface-filled cluster, actions in another? Grouping is on-system; dividers are fighting it.
- A terminal status line is flat because one program writes it. Four workers write this one. Should the console impose a chip shape, or stay a dumb slot and accept the drift?
