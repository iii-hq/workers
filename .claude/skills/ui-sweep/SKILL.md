---
name: ui-sweep
description: Full UI/UX sweep of the ADE and every worker page — every control state (rest, hover, focus, active, open, disabled), both themes, desktop and phone — with screenshots, fixes, and before/after pairs. Use when asked to "check all UI", "find hover/color/px issues", or before pushing a design PR.
---

# UI sweep

A review that only looks at screens at rest misses hover washes, focus rings,
open menus and selects, checked boxes and phone layouts. This sweep walks the
whole product through every state and both themes, screenshots each, and
fixes what it finds. It is slow on purpose.

## 0. Serve this branch, not the running stack

Worker pages come from the RUNNING worker's bundle. A rebuilt `<worker>/ui`
is only seen through the vite override, and a worker missing from the
override list still shows the old UI. Build every worker UI first, then start
vite with all of them:

```sh
for w in a2ui browser cron database harness ide iii-directory kanban onboarding sentinel state voice; do (cd $w/ui && pnpm run build); done
cd ade/web && III_WORKER_UI_OVERRIDES="browser=../../browser/ui/dist,harness=../../harness/ui/dist,kanban=../../kanban/ui/dist,state=../../state/ui/dist,cron=../../cron/ui/dist,ide=../../ide/ui/dist,learn-iii=../../onboarding/ui/dist,sentinel=../../sentinel/ui/dist,voice=../../voice/ui/dist,a2ui=../../a2ui/ui/dist,database=../../database/ui/dist,iii-directory=../../iii-directory/ui/dist" node node_modules/vite/bin/vite.js --port 5190 --strictPort
```

The override key is the bundle SCOPE, not the worker name (onboarding →
`learn-iii`). Check `page.on('request')` for `/ui/<scope>/page.js` when unsure.
A worker that is not in this repo (an installed package) cannot be reviewed
here; say so instead of guessing.

For a first-run check use a fresh stack: `.context/ade-stack/up.sh` (own
namespace, engine 49154, console 3133, empty `data/`), then vite on 5195 with
`III_ENGINE_URL=ws://127.0.0.1:49154 III_CONSOLE_URL=http://127.0.0.1:3133`.
Launch Playwright with `--disable-blink-features=AutomationControlled` or the
wizard never auto-opens.

## 1. The matrix

Screens: chat (empty, with messages, with an ask card, streaming), sidebar,
command palette (typed query, each mode chip), context panel (every card),
settings (General and one worker form), setup wizard (every step), tour card,
workers page, traces, and every worker page: state, cron, sentinel, kanban
(board, releases, ticket, a dialog), database, browser (new tab and a page),
ide (explorer, collapsed rail, branch picker, SCM), iii-directory, voice.

For each screen, capture:

| State | How |
| --- | --- |
| rest | plain screenshot |
| hover | `locator.hover()` on a row, a button, a chip, a tab |
| focus | `locator.focus()` or Tab from the previous control |
| active / selected | select a row, check a box, open a tab |
| open | every menu, select, popover, dialog, sheet |
| disabled | where one exists |
| phone | viewport 390×844, `isMobile: true` |

Both themes, always: click "Follow the system theme" off and use
`colorScheme`. Dark and light fail differently (a white check on a white
primary only shows in dark; a faint `input` border only vanishes in light).

## 2. What to look for

- A wash touching a card's or popup's edge (rows need inset from a border).
- Anything that disappears on hover or in one theme: borders on `input`,
  ticks in `text-white`, thumbs in `bg-background`.
- Two classes fighting (`gap-2 gap-4`), legacy offsets for icons that are
  now in an addon slot, `pl-0` next to an icon.
- Native controls: `grep -rn "<select\b" --include='*.tsx'` must be empty
  outside tests; same for raw `<input type=checkbox>` outside `controls.tsx`.
- Literal colours: `grep -rn "text-white\|bg-white\|#fff\|white;"` outside
  brand marks and iframe backgrounds.
- Labels ending in "…" or "..." on actions (loading states keep theirs).
- Radii: fields `rounded-field`, cards `rounded-xl`, menus `rounded-lg`,
  nothing hard-coded.
- Phone: clipped placeholders, controls centred against multi-line rows,
  horizontal overflow.

`hover:bg-accent` is NEVER a hover in this console: `--color-accent` is the
primary fill. Hover is `bg-surface-hover` or `bg-muted`.

## 3. Fix, then prove it

Fix in the shared recipe or token first (`ui-recipes.css`, the shadcn part,
the console wrapper), the leaf second. Rebuild the worker UI, re-shoot the
same state, and compose a before/after sheet with the earlier shot:
`ade/web/node_modules/.cache/ds-shots/r-pairs.mjs` is the pattern (two PNGs
inlined as data URLs, side by side, one PNG out). Open the sheets with `open`
so they show on screen. Commit locally and wait for approval before pushing.

## 4. Report

One table: item, state, theme, viewport, file:line, fix, before/after path.
Say which screens and workers were NOT covered and why.
