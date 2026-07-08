---
version: alpha
name: iii Schematic
description: A minimal, blueprint-style design system for the iii engine. Engineering-document aesthetic in warm cream and ink, with a single vivid orange accent and an all-monospace voice.
colors:
  bg: "#f2f0ed"
  panel: "#e9e6e2"
  paper: "#f2f0ed"
  paper-2: "#ebe8e3"
  ink: "#0a0a0a"
  ink-2: "#1a1a1a"
  ink-soft: "#1a1a1a"
  ink-faint: "#6b6865"
  ink-ghost: "#a3a09c"
  mute: "#6b6865"
  mute-2: "#a3a09c"
  rule: "#d8d5d0"
  rule-2: "#e6e3df"
  accent: "#ff5a1f"
  accent-dark: "#3ea8ff"
  bg-dark: "#111110"
  panel-dark: "#1a1916"
  ink-dark: "#f2f0ed"
  ink-faint-dark: "#9c9893"
  ink-ghost-dark: "#5d5a55"
  rule-dark: "#2a2926"
  rule-2-dark: "#1f1e1c"
  alert: "#c43e1c"
  warn: "#a87a00"
typography:
  logo:
    fontFamily: Chivo Mono
    fontSize: 18px
    fontWeight: 600
    lineHeight: 1
    letterSpacing: -0.02em
  display-hero:
    fontFamily: Chivo Mono
    fontSize: 72px
    fontWeight: 600
    lineHeight: 1.02
    letterSpacing: -0.02em
  display-foot:
    fontFamily: Chivo Mono
    fontSize: 48px
    fontWeight: 600
    lineHeight: 1.05
    letterSpacing: -0.03em
  headline-section:
    fontFamily: Chivo Mono
    fontSize: 28px
    fontWeight: 500
    lineHeight: 1.25
    letterSpacing: -0.01em
  headline-card:
    fontFamily: Chivo Mono
    fontSize: 20px
    fontWeight: 500
    lineHeight: 1.1
    letterSpacing: -0.02em
  title-cell:
    fontFamily: Chivo Mono
    fontSize: 16px
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: -0.01em
  body-md:
    fontFamily: Chivo Mono
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.7
  body-sm:
    fontFamily: Chivo Mono
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.7
  code-md:
    fontFamily: Chivo Mono
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.65
  code-sm:
    fontFamily: Chivo Mono
    fontSize: 12.5px
    fontWeight: 400
    lineHeight: 1.55
  label-caps-lg:
    fontFamily: Chivo Mono
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.18em
  label-caps-md:
    fontFamily: Chivo Mono
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.14em
  label-caps-sm:
    fontFamily: Chivo Mono
    fontSize: 11px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.06em
  micro:
    fontFamily: Chivo Mono
    fontSize: 9px
    fontWeight: 400
    lineHeight: 1
    letterSpacing: 0.04em
rounded:
  none: 0px
  sm: 0px
  md: 0px
  lg: 0px
  full: 9999px
spacing:
  base: 16px
  hairline: 1px
  micro: 2px
  4: 4px
  6: 6px
  8: 8px
  10: 10px
  12: 12px
  14: 14px
  16: 16px
  18: 18px
  20: 20px
  24: 24px
  28: 28px
  32: 32px
  36: 36px
  44: 44px
  56: 56px
  64: 64px
  80: 80px
  96: 96px
  gutter: 24px
  section-x: 36px
  section-y: 80px
  sheet-max: 1200px
  content-max: 1216px
  border-width: 1px
components:
  button-primary:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.bg}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: 12px 20px
  button-primary-hover:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
  button-ghost:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: 12px 20px
  button-ghost-hover:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.bg}"
  button-pill:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: 6px 12px
  button-pill-hover:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.bg}"
  button-icon:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink-faint}"
    rounded: "{rounded.none}"
    size: 30px
  button-icon-hover:
    textColor: "{colors.ink}"
  nav-link:
    typography: "{typography.body-sm}"
    textColor: "{colors.mute}"
    padding: 6px 0
  nav-link-hover:
    textColor: "{colors.ink}"
  card:
    backgroundColor: "{colors.bg}"
    rounded: "{rounded.none}"
    padding: 28px
  card-focus:
    backgroundColor: "{colors.panel}"
  card-head:
    backgroundColor: "{colors.panel}"
    typography: "{typography.label-caps-lg}"
    textColor: "{colors.ink-faint}"
    padding: 10px 14px
  input:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: 10px 2px
  input-placeholder:
    textColor: "{colors.ink-ghost}"
  input-focus:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
  badge-numeric:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.bg}"
    typography: "{typography.label-caps-sm}"
    rounded: "{rounded.none}"
    padding: 0 4px
    height: 16px
  status-dot:
    backgroundColor: "{colors.accent}"
    rounded: "{rounded.full}"
    size: 6px
  rule-line:
    backgroundColor: "{colors.rule}"
    height: 1px
  code-block:
    backgroundColor: "{colors.bg}"
    typography: "{typography.code-sm}"
    textColor: "{colors.ink}"
    padding: 18px 20px
  terminal-button:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: 10px 14px
  terminal-prompt:
    textColor: "{colors.accent}"
  toggle-active:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.bg}"
  toggle-inactive:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.mute}"
---

# iii Schematic — design system

This document is a self-contained, portable spec for the iii Schematic UI: a
warm-cream, ink-on-paper, monospace web UI built like an engineering drafting
sheet. Everything needed to reproduce the system in another project lives
inside this file — there are no links to repository sources.

The YAML frontmatter above is the machine-readable token spec. The sections
below translate it into implementation-ready CSS and React. They assume:

- **Tailwind CSS v4** (uses the `@theme` and `@utility` directives).
- **React + TypeScript**.
- A `cn` helper built on `clsx` + `tailwind-merge`.
- Optional: `class-variance-authority` (`cva`) and `@radix-ui/react-slot` for
  the `Button` component below.

To adopt the system: copy §0 (Setup) into a new project, then bring over the
canonical components in §10 as-is.

---

## 0. Setup

### Font

Load Chivo Mono in `index.html`. Weights 400/500/600 cover the entire scale
(body, label-caps, headlines, display).

```html
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link
  rel="stylesheet"
  href="https://fonts.googleapis.com/css2?family=Chivo+Mono:wght@400;500;600&display=swap"
/>
```

### HTML shell

Light is the canonical theme. Dark is opt-in via a `data-theme="dark"`
attribute on `<html>` (you can also wire `prefers-color-scheme` to set it on
load — see §3).

```html
<html lang="en" class="antialiased">
  <body class="bg-bg text-ink font-sans">
    <div id="root"></div>
  </body>
</html>
```

### Theme stylesheet

The full design-token stylesheet — drop this in as your global CSS entrypoint:

```css
@import "tailwindcss";

@theme {
  --font-sans: "Chivo Mono", ui-monospace, SFMono-Regular, Menlo, Monaco,
    Consolas, "Liberation Mono", "Courier New", monospace;
  --font-mono: "Chivo Mono", ui-monospace, SFMono-Regular, Menlo, Monaco,
    Consolas, "Liberation Mono", "Courier New", monospace;

  /* paper / surface ramp (3-step) */
  --color-bg: #f2f0ed;
  --color-panel: #e9e6e2;
  --color-paper-2: #ebe8e3;

  /* ink ramp (3-step) */
  --color-ink: #0a0a0a;
  --color-ink-faint: #6b6865;
  --color-ink-ghost: #a3a09c;

  /* structural lines */
  --color-rule: #d8d5d0;
  --color-rule-2: #e6e3df;

  /* accent (single hero — hot orange) */
  --color-accent: #ff5a1f;
  --color-accent-fg: #f2f0ed;

  /* status */
  --color-alert: #c43e1c;
  --color-warn: #a87a00;

  /* radii — only two are allowed */
  --radius-none: 0px;
  --radius-full: 9999px;

  /* spacing scale (carried from the YAML) */
  --spacing-gutter: 24px;
  --spacing-section-x: 36px;
  --spacing-section-y: 80px;
  --spacing-sheet-max: 1200px;
  --spacing-content-max: 1216px;
}

/* dark theme: invert paper/ink and swap accent to electric blue */
[data-theme="dark"] {
  --color-bg: #111110;
  --color-panel: #1a1916;
  --color-paper-2: #1f1e1c;
  --color-ink: #f2f0ed;
  --color-ink-faint: #9c9893;
  --color-ink-ghost: #5d5a55;
  --color-rule: #2a2926;
  --color-rule-2: #1f1e1c;
  --color-accent: #3ea8ff;
  --color-accent-fg: #111110;
}

@layer base {
  html,
  body,
  #root {
    height: 100%;
  }

  html,
  body {
    scrollbar-gutter: stable;
  }

  html {
    overflow-y: scroll;
  }

  body {
    background-color: var(--color-bg);
    color: var(--color-ink);
    /* explicitly disable decorative ligatures — the schematic feel relies on
       monospace columns, not typographic flourishes */
    font-feature-settings: "liga" 0, "clig" 0, "calt" 0, "dlig" 0;
  }

  ::selection {
    background-color: var(--color-accent);
    color: var(--color-accent-fg);
  }

  ::-webkit-scrollbar {
    width: 10px;
    height: 10px;
  }
  ::-webkit-scrollbar-track {
    background: transparent;
  }
  ::-webkit-scrollbar-thumb {
    background: var(--color-rule);
  }
  ::-webkit-scrollbar-thumb:hover {
    background: var(--color-ink-ghost);
  }
}

@keyframes pulse-dot {
  0% {
    box-shadow: 0 0 0 0 var(--color-accent);
  }
  100% {
    box-shadow: 0 0 0 8px transparent;
  }
}

@utility pulse-dot {
  animation: pulse-dot 1.6s ease-out infinite;
}

@keyframes blink {
  0%, 49% { opacity: 1; }
  50%, 100% { opacity: 0; }
}

@utility blink {
  animation: blink 1s steps(1) infinite;
}

@keyframes wiggle {
  0%, 90%, 100% { transform: rotate(0deg); }
  93% { transform: rotate(-3deg); }
  96% { transform: rotate(3deg); }
}

@utility wiggle {
  animation: wiggle 3s ease-in-out infinite;
}

/* the one sanctioned shadow stack — used only for the "deal" stack
   animation on language cards */
@utility deal-shadow {
  box-shadow: -2px 0 0 var(--color-rule),
    -16px 4px 36px -10px rgba(0, 0, 0, 0.22);
}
```

### `cn` helper

Every component below uses this helper. Install `clsx` and `tailwind-merge`,
then expose it from `lib/utils.ts`:

```ts
import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
```

---

## 1. Philosophy — "iii Schematic"

The app should feel like an **engineering document**, not a SaaS dashboard.

- The page is a **drafting sheet**: cream paper, hairline ink rules,
  monospace voice, an unflinching commitment to lowercase. Nothing is
  rounded; everything sits on a 1px grid.
- Color is rationed: the palette is essentially **black-on-cream**, broken
  only by a single hot orange used for state, focus, and emphasis.
- The personality is **technical but unintimidating** — the same energy as a
  well-kept lab notebook or a hand-drawn architecture diagram. It must feel
  built by engineers, for engineers, and for the agents working alongside
  them.
- Density is deliberate: spec sheets, code, traces, and console panels
  coexist on the same surface without a hierarchy contest.

### Voice

- All UI copy is **lowercase**, including headlines, buttons, and nav items.
- Headlines treat sentence fragments as visual blocks (e.g. *"any task. one
  experience."*).
- Numbers and metadata always use **tabular monospace**, never proportional
  figures.
- The wordmark is pronounced *"three eye"* — every "i" stays lowercase.

> If you removed all the type, the page should still read as a structured
> document. Lines establish hierarchy before color does.

---

## 2. Typography — single typeface (Chivo Mono)

Chivo Mono is wired to **both** `--font-sans` and `--font-mono` in the theme
(see §0), so every default text node is monospaced. There is no secondary
face in this design language.

**Rule:** every UI surface uses Chivo Mono (the default). Don't reach for a
sans-serif or a second monospace stack — variety comes from weight, scale,
case, and letter-spacing, not family.

Decorative ligatures are explicitly disabled in `@layer base`
(`liga 0, clig 0, calt 0, dlig 0`) to preserve the schematic feel.

### Size scale

| Use                      | Token              | Size / weight / extras                         |
| ------------------------ | ------------------ | ---------------------------------------------- |
| Hero headline            | `display-hero`     | 72px / 600 / `tracking-[-0.02em]`              |
| Footer CTA               | `display-foot`     | 48px / 600 / `tracking-[-0.03em]`              |
| Section headline         | `headline-section` | 28px / 500 / `tracking-[-0.01em]`              |
| Card headline            | `headline-card`    | 20px / 500 / `tracking-[-0.02em]`              |
| Cell title               | `title-cell`       | 16px / 600 / `tracking-[-0.01em]`              |
| Body                     | `body-md`          | 14px / 400 / line-height 1.7                   |
| Compact body             | `body-sm`          | 13px / 400 / line-height 1.7                   |
| Code block               | `code-md`          | 13px / 400 / line-height 1.65                  |
| Compact code / terminal  | `code-sm`          | 12.5px / 400 / line-height 1.55                |
| Label (caps, large)      | `label-caps-lg`    | 12px / 500 / UPPER, `tracking-[0.18em]`        |
| Label (caps, medium)     | `label-caps-md`    | 12px / 500 / UPPER, `tracking-[0.14em]`        |
| Label (caps, small)      | `label-caps-sm`    | 11px / 500 / UPPER, `tracking-[0.06em]`        |
| Diagram micro            | `micro`            | 9px / 400 / `tracking-[0.04em]`                |

### Number & label rendering

- Any numeric or timestamp cell uses `tabular-nums` so columns align. See the
  duration column in `Trace` and the version row in `WorkerCard` (§10).
- The **only** uppercase text in the system is the `label-caps-*` set
  (uppercase + tracking). Used for: tab strips, status pills, table headers,
  code-block chrome, section eyebrows. Never capitalize a sentence to "fix" a
  heading — rewrite it instead.

---

## 3. Color tokens

All tokens live in the `@theme` block in §0. Use the Tailwind utility
(`bg-bg`, `text-ink-faint`, `text-accent`, `border-rule`, …) — **never** the
raw CSS variable.

### Paper / surface (3-step neutral scale)

| Token     | Use                                                  |
| --------- | ---------------------------------------------------- |
| `bg`      | Page (warm cream paper) — every default surface      |
| `panel`   | Header strips, focused cards, code-block chrome      |
| `paper-2` | Nested separation when `panel` would be too heavy    |

### Ink (3-step contrast)

| Token       | Use                                                |
| ----------- | -------------------------------------------------- |
| `ink`       | Primary type, wordmark, primary buttons, hairlines |
| `ink-faint` | Body in muted contexts, captions, inactive nav     |
| `ink-ghost` | Line numbers, placeholders, timestamps             |

### Rules (the structural lines)

| Token    | Use                                                          |
| -------- | ------------------------------------------------------------ |
| `rule`   | Default 1px borders — defines every container                |
| `rule-2` | Nested separators (e.g. card head → body) when `rule` is too loud |

### Accent — hot orange (single hero)

`accent`, `accent-fg`. Reserved for: the active state, the live pulse, the
keyword/return value in a code block, the `iii` highlight in a sentence, the
focus underline on the email row, the orange `$` prompt. **Never** for body
text or large fills.

### Status

`alert`, `warn` — desaturated on purpose. Status is expressed through **text
color + a small icon/stripe**, not full-color backgrounds.

| Token    | Use                                       |
| -------- | ----------------------------------------- |
| `accent` | live / running / success                  |
| `alert`  | error states (traces, status panels)      |
| `warn`   | warning states                            |

### Dark theme

Override the same tokens inside a `[data-theme="dark"]` block (see §0). The
ramp inverts (`#111110 → #1a1916 → #1f1e1c` for paper; `#f2f0ed → #9c9893 →
#5d5a55` for ink) and the accent swaps to electric blue (`#3ea8ff`). Same
structural logic — never collapses into pure black.

To follow the OS, set the attribute on load:

```ts
const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches
document.documentElement.dataset.theme = isDark ? 'dark' : 'light'
```

---

## 4. The "lines define structure" rule (key composition pattern)

> **Borders define every container. Add a border before reaching for a
> background fill.**

This is the inverse of a surface-stacking system: the schematic is held
together by 1px ink rules. A new line means a new container. Cards, cells,
panels, code blocks, and inputs are all defined by `border border-rule` — not
by background steps.

### Pattern

```tsx
<div className="border border-rule bg-bg">
  <div className="px-3.5 py-2.5 border-b border-rule bg-panel font-mono text-[12px] uppercase tracking-[0.18em] text-ink-faint">
    title
  </div>
  <div className="p-7">{/* body */}</div>
</div>
```

The single tonal step (`bg → panel`) on the header strip is the only "lift"
allowed in default states (see §5).

### Allowed accents on top of borders

These are not extra borders, they are accents on the existing 1px grid:

- `border-l-2 border-accent` rail on a focused `WorkerCard` (see §10).
- `divide-x divide-rule` for inline grids (e.g. a metrics strip).
- The "feature grid" trick: draw 1px rules on the **top + left** of the grid
  and the **right + bottom** of each cell. This produces a single,
  perfectly-aligned grid of borders with no doubled lines.

### The single-shadow exception

The system carries no default shadows. The **one** allowed transient shadow
is `.deal-shadow` (defined in §0), used only on the language-card stack
when a card slides on top of another — a deliberate tactile cue, not a
default elevation.

**When in doubt:** add a 1px `border-rule`. Don't reach for a background fill
or a shadow.

---

## 5. Surfaces & elevation

The 3-step ramp is the only tonal tool. Think of it as a depth axis with one
step of contrast — borders do the rest.

```
bg            page (cream paper)
 └─ panel        header strips, focused cards, code-block chrome
     └─ paper-2     nested separation when needed
```

Most cards/panels should accept a `focused` (or `variant`) prop that lifts
the body from `bg` to `panel`. That single tonal step is the only "lift"
allowed in default states.

### Where elevation actually shows up

1. **Tonal layering** — the 3-step ramp above.
2. **Hairline rules** — 1px lines in `rule` and `rule-2` do the work that
   shadows would do in other systems.
3. **Reserved transient shadow** — the `.deal-shadow` utility, used only on
   the language-card stack (`HelloCard` flow mode in §10).
4. **Pulse + glow** — the live state uses `.pulse-dot` (1.6s expanding
   `box-shadow` ring) on a 6px accent dot. This is the only "glow" the
   system allows.

Dark mode keeps the same rule-driven hierarchy — the ramp flips but never
collapses into pure black.

---

## 6. Radii & shape

Rectilinear sharpness, period. The system is built from squares and 1px
strokes; the only curves are functional ones (status dots, glyph circles).

| Token          | Value   | Used for                                        |
| -------------- | ------- | ----------------------------------------------- |
| `rounded-none` | 0px     | **Default for everything** (containers, buttons, cards, inputs, code blocks, badges, panels) |
| `rounded-full` | 9999px  | Status dots and glyph circles **only**          |

Never reach for `rounded-sm`, `rounded-md`, `rounded-lg`, or anything in
between. There is no "soft" variant — the YAML aliases `sm/md/lg` to `0px`
on purpose.

### Stroke weight

- 1px is the default for all UI borders.
- SVG diagrams use 1–1.25px strokes; thicker strokes (1.25px `accent`) are
  reserved for emphasized worker connections.
- The wordmark is six rectangles (three "i"s, each a stem + a tittle), all
  sharing the same square unit. The mark is the design system in miniature:
  identical units, no curves, deliberate negative space.

---

## 7. Spacing & layout

The page is a vertical sheet, max 1200px, sitting on cream paper with a 1px
ink-rule outer border. There is no hero card, no rounded container — the
entire site reads as one continuous spec sheet.

- **Sheet:** `min(1200px, 100%)` centered, 1px outer rule (see `Sheet` in
  §10).
- **Sticky nav:** `py-4.5` vertical padding, collapses to `py-2.5` on
  scroll. Bordered bottom only.
- **Section padding:** `px-9` on desktop (36px), `px-4.5` on tablet (18px),
  `px-3.5` on small mobile (14px). Vertical: `py-20` to `py-24` (80–96px)
  between major sections.
- **Hero grid:** Two equal columns (`1fr 1fr`) with a 64px gap on desktop;
  collapses to a single column under 880px.
- **Feature grids:** Always **3-up** on desktop (`grid-cols-3`), divided by
  1px rules drawn on the **top + left** of the grid and the **right +
  bottom** of each cell.
- **Card padding:** 18–28px internal. Cards never carry shadows in their
  default state.
- **Density rhythm:** 4/8 micro-scale for inline gaps (icon-to-text,
  dot-to-label), 12/14/16 for component padding, 24/28 for card padding, 36
  for section gutters, 64–96 for section breathing room.
- **Scroll anchors:** Anchored sections reserve a 90px scroll-margin to
  clear the sticky nav.
- **Responsiveness uses container queries**, not viewport breakpoints. Pages
  set `@container` and grids use `@3xl:` / `@4xl:` to split. This keeps
  panels responsive when embedded in different layouts.

---

## 8. Schematic motifs (utilities)

Reach for these to keep the engineering-document feel consistent:

- `Prompt` (§10) — orange `$` (or `>`) used on terminal/install rows and as
  a page eyebrow above titles.
- `Caret` (§10) — blinking 6×13 ink caret with the `.blink` utility.
- `StatusDot` (§10) — 6px circle, `accent` fill, optional `.pulse-dot` for
  "live" emphasis.
- `.deal-shadow` (§0) — the one allowed transient shadow on stacked cards.
- `.wiggle` (§0) — 1s wiggle every 3s on the wiggle CTA, to draw the eye in
  long flows.
- Lowercase copy is the default; UPPERCASE only via the `label-caps-*`
  styles.

---

## 9. Status & semantics

`StatusPanel` (§10) is the canonical status display — bordered, monospaced,
icon + headline + detail. The body fill stays `bg` in every variant; only
the chrome (border + icon + headline) changes.

| Variant     | Border + icon token | Body fill |
| ----------- | ------------------- | --------- |
| `v-info`    | `rule` (ink icon)   | `bg`      |
| `v-success` | `accent`            | `bg`      |
| `v-warn`    | `warn`              | `bg`      |
| `v-alert`   | `alert`             | `bg`      |

**In tables and traces:** express row severity with a left-edge `border-l-2`
stripe plus a faint tinted background (e.g. `bg-alert/5`). Never with a
full ring or a solid status background. See `Trace` (§10).

For inline "live" emphasis, pair a `StatusDot` with the `pulse-dot`
utility — the only sanctioned glow.

---

## 10. Canonical components

Each component below is the reference implementation for that role. Copy
them verbatim into a new project — they all depend only on `cn` (§0), React,
and (for `Button`) `cva` and `@radix-ui/react-slot`.

### `Prompt`

The orange `$` (or `>`) eyebrow used on terminal/install rows and above page
titles. Renders the prompt symbol in `accent`, with optional inline content
in `ink`.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface PromptProps {
  symbol?: string
  className?: string
  children?: React.ReactNode
}

export function Prompt({ symbol = '$', className, children }: PromptProps) {
  return (
    <span className={cn('font-mono text-accent', className)}>
      {symbol}
      {children !== undefined ? (
        <span className="text-ink ml-2">{children}</span>
      ) : null}
    </span>
  )
}
```

### `Caret`

A blinking 6×13 ink caret. Uses the `.blink` utility from §0. Drop next to a
command to read as a typed query mid-stroke.

```tsx
import { cn } from '@/lib/utils'

interface CaretProps {
  className?: string
}

export function Caret({ className }: CaretProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'blink inline-block w-[6px] h-[13px] bg-ink align-middle',
        className,
      )}
    />
  )
}
```

### `StatusDot`

6px circle. The only place `rounded-full` is allowed besides glyphs.
Optional `.pulse-dot` glow for "live" emphasis.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

type DotTone = 'accent' | 'alert' | 'warn' | 'ink'

const dotTone: Record<DotTone, string> = {
  accent: 'bg-accent',
  alert: 'bg-alert',
  warn: 'bg-warn',
  ink: 'bg-ink',
}

interface StatusDotProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: DotTone
  pulse?: boolean
}

export function StatusDot({
  tone = 'accent',
  pulse,
  className,
  ...props
}: StatusDotProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-block size-1.5 rounded-full shrink-0',
        dotTone[tone],
        pulse && 'pulse-dot',
        className,
      )}
      {...props}
    />
  )
}
```

### `Button`

Variants: `primary` (ink fill, hover inverts to outlined `bg`), `ghost`
(transparent → solid ink on hover), `pill` (compact nav button), `icon`
(30×30 rule-bordered), `terminal` (full-width row with `$` prompt + command
+ optional copy), `wiggle` (primary + corner badge slot + `.wiggle`). Sizes:
`sm` `md` `lg` `icon`. Depends on `class-variance-authority` and
`@radix-ui/react-slot`.

```tsx
import * as React from 'react'
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-x-2 whitespace-nowrap font-mono lowercase rounded-none transition-[background-color,color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:pointer-events-none disabled:opacity-40 select-none',
  {
    variants: {
      variant: {
        primary:
          'bg-ink text-bg border border-ink hover:bg-bg hover:text-ink',
        ghost:
          'bg-transparent text-ink border border-transparent hover:bg-ink hover:text-bg',
        pill:
          'bg-bg text-ink border border-ink hover:bg-ink hover:text-bg',
        icon:
          'bg-bg text-ink-faint border border-rule hover:text-ink',
        terminal:
          'bg-bg text-ink border border-rule justify-start',
        wiggle:
          'wiggle bg-ink text-bg border border-ink hover:bg-bg hover:text-ink relative',
      },
      size: {
        sm: 'h-8 px-3 text-[13px]',
        md: 'h-9 px-5 text-[13px]',
        lg: 'h-11 px-5 text-[14px]',
        icon: 'size-[30px] p-0',
      },
    },
    defaultVariants: {
      variant: 'primary',
      size: 'md',
    },
  },
)

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild, children, ...props }, ref) => {
    const Comp: React.ElementType = asChild ? Slot : 'button'
    return (
      <Comp
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      >
        {children}
      </Comp>
    )
  },
)
Button.displayName = 'Button'

export { buttonVariants }
```

### `StatusPanel`

A row component for system messages: 18px icon slot, 13px Semi-Bold
headline, 12px ink-faint detail. Variants tint the **border + icon +
headline only** — body fill stays `bg`.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

export type StatusVariant = 'info' | 'success' | 'warn' | 'alert'

const variantTone: Record<
  StatusVariant,
  { border: string; icon: string; headline: string }
> = {
  info: {
    border: 'border-rule',
    icon: 'text-ink',
    headline: 'text-ink',
  },
  success: {
    border: 'border-accent',
    icon: 'text-accent',
    headline: 'text-accent',
  },
  warn: {
    border: 'border-warn',
    icon: 'text-warn',
    headline: 'text-warn',
  },
  alert: {
    border: 'border-alert',
    icon: 'text-alert',
    headline: 'text-alert',
  },
}

interface StatusPanelProps {
  variant?: StatusVariant
  icon?: React.ReactNode
  headline: React.ReactNode
  detail?: React.ReactNode
  className?: string
}

export function StatusPanel({
  variant = 'info',
  icon,
  headline,
  detail,
  className,
}: StatusPanelProps) {
  const tone = variantTone[variant]
  return (
    <div
      className={cn(
        'flex items-start gap-x-3 border bg-bg px-3.5 py-3',
        tone.border,
        className,
      )}
    >
      {icon ? (
        <span
          aria-hidden
          className={cn('size-[18px] shrink-0', tone.icon)}
        >
          {icon}
        </span>
      ) : null}
      <div className="min-w-0 flex flex-col gap-y-0.5">
        <div
          className={cn(
            'font-mono text-[13px] font-semibold lowercase',
            tone.headline,
          )}
        >
          {headline}
        </div>
        {detail ? (
          <div className="font-mono text-[12px] text-ink-faint lowercase">
            {detail}
          </div>
        ) : null}
      </div>
    </div>
  )
}
```

### `CodeBlock`

`bg` fill, 1px `rule` border, monospace at 12.5px / line-height 1.55. Light
syntax tinting: comments italic-ghost, strings in `accent`, keywords
bold-ink, numbers in `alert`. The accent orange is reserved for **string
literals and the active call return**, not arbitrary keywords.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface CodeBlockProps extends React.HTMLAttributes<HTMLPreElement> {
  children: React.ReactNode
}

export function CodeBlock({ className, children, ...props }: CodeBlockProps) {
  return (
    <pre
      className={cn(
        'border border-rule bg-bg overflow-x-auto px-5 py-4 font-mono text-[12.5px] leading-[1.55] text-ink',
        className,
      )}
      {...props}
    >
      <code>{children}</code>
    </pre>
  )
}
```

### `Terminal` & `TerminalRow`

Header strip in `panel` with a label-caps title; body in `bg` with an orange
`$` prompt, ink command, and an animated 6×13 ink caret.

```tsx
import * as React from 'react'
import { Prompt } from './Prompt'
import { Caret } from './Caret'
import { cn } from '@/lib/utils'

interface TerminalProps {
  title?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function Terminal({ title, children, className }: TerminalProps) {
  return (
    <div className={cn('border border-rule bg-bg', className)}>
      {title ? (
        <div className="bg-panel px-3.5 py-2 border-b border-rule font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          {title}
        </div>
      ) : null}
      <div className="p-4 font-mono text-[13px] text-ink">{children}</div>
    </div>
  )
}

interface TerminalRowProps {
  command: React.ReactNode
  showCaret?: boolean
  className?: string
}

export function TerminalRow({
  command,
  showCaret,
  className,
}: TerminalRowProps) {
  return (
    <div className={cn('flex items-center gap-x-2', className)}>
      <Prompt symbol="$" />
      <span className="text-ink">{command}</span>
      {showCaret ? <Caret /> : null}
    </div>
  )
}
```

### `Trace`

Header strip + a list of trace rows with a `StatusDot`, op label, duration,
and a label-caps status. A waterfall of horizontal bars follows: `rule-2`
background, `ink` fill, `alert` fill for error spans.

> `TraceStatus` is the consumer's own domain type. The shape assumed here is
> `{ id, op, durationMs, status, startMs?, spanMs? }`. Replace with whatever
> your project uses.

```tsx
import * as React from 'react'
import { StatusDot } from './StatusDot'
import { cn } from '@/lib/utils'

export type TraceStatus = 'ok' | 'warn' | 'err'

interface TraceRow {
  id: string
  op: string
  durationMs: number
  status: TraceStatus
  startMs?: number
  spanMs?: number
}

interface TraceProps {
  title: React.ReactNode
  rows: TraceRow[]
  totalMs?: number
  className?: string
}

const statusTone: Record<
  TraceStatus,
  { dot: 'accent' | 'warn' | 'alert'; label: string; bar: string }
> = {
  ok: { dot: 'accent', label: 'text-accent', bar: 'bg-ink' },
  warn: { dot: 'warn', label: 'text-warn', bar: 'bg-warn' },
  err: { dot: 'alert', label: 'text-alert', bar: 'bg-alert' },
}

export function Trace({ title, rows, totalMs, className }: TraceProps) {
  const span =
    totalMs ??
    Math.max(
      ...rows.map((r) => (r.startMs ?? 0) + (r.spanMs ?? r.durationMs)),
    )
  return (
    <div className={cn('border border-rule bg-bg', className)}>
      <div className="bg-panel px-3.5 py-2 border-b border-rule font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
        {title}
      </div>
      <ul className="divide-y divide-rule-2">
        {rows.map((row) => {
          const tone = statusTone[row.status]
          const start = ((row.startMs ?? 0) / span) * 100
          const width = ((row.spanMs ?? row.durationMs) / span) * 100
          return (
            <li
              key={row.id}
              className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-x-3 px-3.5 py-2 font-mono text-[12px]"
            >
              <StatusDot tone={tone.dot} />
              <span className="text-ink truncate lowercase">{row.op}</span>
              <span className="text-ink-faint tabular-nums">
                {row.durationMs}ms
              </span>
              <span
                className={cn(
                  'text-[11px] uppercase tracking-[0.06em] font-medium',
                  tone.label,
                )}
              >
                {row.status}
              </span>
              <div className="col-span-4 mt-1 h-1 bg-rule-2 relative">
                <div
                  className={cn('absolute top-0 h-full', tone.bar)}
                  style={{ left: `${start}%`, width: `${width}%` }}
                />
              </div>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
```

### `Cell`

A grid-bordered cell — the universal container for short prose blocks
(features, hellos, pull-quotes). 28px padding, `bg` fill, optional 16px
Semi-Bold ink title, 13px ink-faint body capped at ~34ch.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface CellProps {
  title?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function Cell({ title, children, className }: CellProps) {
  return (
    <div className={cn('border border-rule bg-bg p-7', className)}>
      {title ? (
        <div className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink mb-3 lowercase">
          {title}
        </div>
      ) : null}
      <div className="font-mono text-[13px] leading-[1.7] text-ink-faint max-w-[34ch]">
        {children}
      </div>
    </div>
  )
}
```

### `WorkerCard`

400px-wide ticker card with a name + version row, description, a
`panel`-tinted command block, and a footer with a kind tag and check icon.
Focused state switches body fill from `bg` to `panel` and grows a
`border-l-2 border-l-accent` rail on the left edge.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface WorkerCardProps {
  name: string
  version: string
  description: React.ReactNode
  command: React.ReactNode
  kind: string
  focused?: boolean
  className?: string
}

export function WorkerCard({
  name,
  version,
  description,
  command,
  kind,
  focused,
  className,
}: WorkerCardProps) {
  return (
    <article
      className={cn(
        'w-[400px] border border-rule transition-colors',
        focused ? 'bg-panel border-l-2 border-l-accent' : 'bg-bg',
        className,
      )}
    >
      <header className="flex items-center justify-between px-4 py-3 border-b border-rule-2">
        <div className="font-mono text-[16px] font-semibold lowercase text-ink">
          {name}
        </div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost tabular-nums">
          v{version}
        </div>
      </header>
      <div className="px-4 py-3 font-mono text-[13px] leading-[1.7] text-ink-faint">
        {description}
      </div>
      <div className="bg-panel font-mono text-[12.5px] text-ink px-4 py-2 border-t border-rule-2">
        {command}
      </div>
      <footer className="flex items-center justify-between px-4 py-2 border-t border-rule-2">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          {kind}
        </span>
        <span aria-hidden className="text-accent">
          ✓
        </span>
      </footer>
    </article>
  )
}
```

### `HelloCard`

Code card with a head row (icon + meta + step number) and a code body. In
`flow` mode, two language cards stack with a 120px peek and use the
`.deal-shadow` utility (the one allowed transient shadow) when sliding on
top of one another.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface HelloCardItem {
  id: string
  language: string
  step: number
  body: React.ReactNode
}

interface HelloCardProps {
  items: HelloCardItem[]
  flow?: boolean
  className?: string
}

export function HelloCard({ items, flow, className }: HelloCardProps) {
  return (
    <div className={cn('relative', className)}>
      {items.map((item, idx) => {
        const isUnder = flow && idx > 0
        return (
          <div
            key={item.id}
            className={cn(
              'border border-rule bg-bg',
              isUnder && 'absolute inset-x-0 -z-10 deal-shadow',
            )}
            style={isUnder ? { top: `${idx * 120}px` } : undefined}
          >
            <header className="flex items-center justify-between px-4 py-2 border-b border-rule-2 bg-panel font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              <span>{item.language}</span>
              <span className="text-ink-ghost tabular-nums">
                step {item.step}
              </span>
            </header>
            <pre className="bg-bg px-5 py-4 font-mono text-[12.5px] leading-[1.55] text-ink overflow-x-auto">
              <code>{item.body}</code>
            </pre>
          </div>
        )
      })}
    </div>
  )
}
```

### `EmailRow`

No border, no fill — just a 1px ink underline that switches to `accent` on
focus. The submit arrow lives inside the row, right-aligned. Helper text
below is label-caps in `ink-ghost`.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface EmailRowProps
  extends Omit<React.FormHTMLAttributes<HTMLFormElement>, 'children'> {
  helper?: React.ReactNode
  inputProps?: React.InputHTMLAttributes<HTMLInputElement>
}

export function EmailRow({
  helper,
  inputProps,
  className,
  ...formProps
}: EmailRowProps) {
  return (
    <form className={cn('flex flex-col gap-y-2', className)} {...formProps}>
      <div className="flex items-center gap-x-2 border-b border-ink focus-within:border-accent transition-colors">
        <input
          type="email"
          {...inputProps}
          className={cn(
            'flex-1 bg-transparent font-mono text-[13px] text-ink placeholder:text-ink-ghost py-2 outline-none lowercase',
            inputProps?.className,
          )}
        />
        <button
          type="submit"
          aria-label="submit"
          className="text-ink hover:text-accent transition-colors"
        >
          →
        </button>
      </div>
      {helper ? (
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
          {helper}
        </div>
      ) : null}
    </form>
  )
}
```

### `SearchField`

Large 24px display-style text with a blinking 2px `accent` caret. Reads as a
typed query mid-stroke.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface SearchFieldProps extends React.InputHTMLAttributes<HTMLInputElement> {
  showCaret?: boolean
}

export function SearchField({
  showCaret = true,
  className,
  ...props
}: SearchFieldProps) {
  return (
    <label
      className={cn(
        'flex items-center gap-x-2 font-mono text-[24px] text-ink',
        className,
      )}
    >
      <input
        type="search"
        {...props}
        className="flex-1 bg-transparent outline-none placeholder:text-ink-ghost lowercase"
      />
      {showCaret ? (
        <span
          aria-hidden
          className="blink inline-block w-[2px] h-[24px] bg-accent align-middle"
        />
      ) : null}
    </label>
  )
}
```

### `ModeToggle`

Two-up segmented control with a 1px `rule` border and 2px internal padding.
Active button: solid `ink` fill, `bg` text. Inactive: transparent,
`ink-faint` text. Joined left-to-right with internal 1px dividers when more
than two options are passed.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface ModeToggleOption<T extends string> {
  value: T
  label: React.ReactNode
}

interface ModeToggleProps<T extends string> {
  value: T
  onChange: (next: T) => void
  options: ModeToggleOption<T>[]
  className?: string
}

export function ModeToggle<T extends string>({
  value,
  onChange,
  options,
  className,
}: ModeToggleProps<T>) {
  return (
    <div
      role="tablist"
      className={cn('inline-flex border border-rule p-[2px]', className)}
    >
      {options.map((opt) => {
        const active = opt.value === value
        return (
          <button
            key={opt.value}
            type="button"
            role="tab"
            aria-pressed={active}
            onClick={() => onChange(opt.value)}
            className={cn(
              'font-mono text-[13px] px-3 py-1 transition-colors lowercase',
              active
                ? 'bg-ink text-bg'
                : 'bg-transparent text-ink-faint hover:text-ink',
            )}
          >
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
```

### `NumericBadge`

16px tall, 4px horizontal padding, `accent` fill on a `bg` border. Sits on
the top-right corner of a CTA to indicate count or unread.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

interface NumericBadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  count: number | string
}

export function NumericBadge({
  count,
  className,
  ...props
}: NumericBadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center justify-center bg-accent text-bg font-mono text-[11px] font-medium uppercase tracking-[0.06em] px-1 h-4 tabular-nums border border-bg',
        className,
      )}
      {...props}
    >
      {count}
    </span>
  )
}
```

### `Sheet` & `PageHeader`

`Sheet` is the standard page-level wrapper — `min(1200px, 100%)`, centered,
1px outer rule. `PageHeader` carries an `eyebrow` (rendered through
`Prompt`), a `title` (display-hero or headline-section), an optional
description in `ink-faint`, and an `actions` slot on the right.

```tsx
import * as React from 'react'
import { Prompt } from '@/components/terminal/Prompt'
import { cn } from '@/lib/utils'

interface SheetProps {
  children: React.ReactNode
  className?: string
}

export function Sheet({ children, className }: SheetProps) {
  return (
    <div
      className={cn(
        'mx-auto w-full max-w-[1200px] border-x border-rule min-h-screen bg-bg',
        className,
      )}
    >
      {children}
    </div>
  )
}

interface PageHeaderProps {
  eyebrow?: React.ReactNode
  title: React.ReactNode
  description?: React.ReactNode
  actions?: React.ReactNode
  className?: string
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
  className,
}: PageHeaderProps) {
  return (
    <div
      className={cn(
        'flex items-end justify-between flex-wrap gap-6 px-9 py-12',
        className,
      )}
    >
      <div className="min-w-0">
        {eyebrow ? (
          <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-3">
            <Prompt symbol="$">{eyebrow}</Prompt>
          </div>
        ) : null}
        <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
          {title}
        </h1>
        {description ? (
          <p className="mt-3 font-mono text-[14px] leading-[1.7] text-ink-faint max-w-[60ch] lowercase">
            {description}
          </p>
        ) : null}
      </div>
      {actions ? (
        <div className="flex items-center gap-x-3">{actions}</div>
      ) : null}
    </div>
  )
}
```

---

## 11. Do / don't

**Do**

- Keep every text element **lowercase**, including headlines and buttons.
  The only uppercase text is the `label-caps-*` styles with explicit
  tracking.
- Lean on **1px hairline rules** to define structure. Add a border before
  reaching for a background fill.
- **Ration the orange accent.** One accent moment per visible region — the
  rest is ink, faint, and ghost.
- Use the **`panel` tone** for headers and focused states. It is the only
  "elevation" the system permits.
- Keep the `iii` wordmark in lowercase, with all six rectangles in `ink`
  (never colored).
- Use **Chivo Mono for everything**, including body copy. Switching faces
  breaks the schematic.
- Use the **Tailwind utility** (`bg-bg`, `text-ink-faint`, `border-rule`,
  `text-accent`) — never the raw CSS variable.
- Use **container queries** (`@container`, `@3xl:`, `@4xl:`) for panel-level
  responsiveness. Pages set `@container` and grids split with `@3xl:`.
- Use **`tabular-nums`** on every number, timestamp, and KPI.
- Use **Lucide icons** (`lucide-react`) for every UI icon — checkmarks,
  carets, funnels, status glyphs. One icon family keeps stroke weight and
  optical size consistent across the console.

**Don't**

- Don't introduce **rounded corners** on containers, buttons, inputs, or
  cards. Reserve `rounded-full` for true symbols (dots, glyphs).
- Don't add **drop shadows** for default elevation. The `.deal-shadow`
  stack-card animation is the only sanctioned shadow.
- Don't use the orange accent for **body text, large fills, or decorative
  blocks.** It loses meaning the moment it stops being rare.
- Don't mix proportional and tabular figures. Numbers are always tabular
  monospace.
- Don't use **gradients** or color stops anywhere. The system is flat by
  design.
- Don't capitalize a sentence to "fix" a heading — rewrite it instead.
- Don't stack two headings of the same scale. Headlines are always paired
  with an ink-faint continuation, never another headline.
- Don't add a **second typeface**, even for code or numbers. The
  single-face constraint is load-bearing.
- Don't paint **full-color status backgrounds** — use text color plus a
  small icon, dot, or left stripe.
- Don't introduce **viewport breakpoints** for panel layouts — use
  container queries.
- Don't hand-write **inline `<svg>` elements or text-glyph icons** (`✓`,
  `×`, arrows) in components — use the matching Lucide icon instead. The
  wordmark and data visualizations (trace bars, diagrams) are the only
  sanctioned hand-drawn SVG.

---

## 12. App-level patterns

These patterns are not part of the canonical primitive set in §10 but
are app-level compositions that the iii worker registry relies on. They are
documented here so that any other surface adopting the iii Schematic can
pick them up consistently.

### Theme toggle

Light is canonical. To let users switch themes (without losing the
schematic feel), expose a two-segment `ModeToggle` in the sticky nav strip
labelled `light` / `dark`. Selection persists to `localStorage` under the
key `iii-theme` and is applied by setting `data-theme="dark"` (or
`"light"`) on `<html>`. To avoid a flash of the wrong theme, run a tiny
inline script in `<head>` before paint:

```html
<script>
  try {
    var t = localStorage.getItem('iii-theme');
    document.documentElement.dataset.theme =
      t === 'dark' || t === 'light' ? t : 'light';
  } catch (_) {
    document.documentElement.dataset.theme = 'light';
  }
</script>
```

The toggle component itself is the standard `ModeToggle` from §10, with
`light` and `dark` as its two options. It is the **only** decoration in the
nav strip's right slot, so the structure stays a clean `[wordmark] —
[toggle]` row. There is no auto OS-follow: the user picks once and that
choice sticks.

### Hybrid registry layout

For listing pages that have both a small set of "highlighted" items and a
much longer browseable index (the worker registry being the canonical
example), use a **hybrid layout** inside a single `Sheet`:

1. **Featured row** — a 3-up grid of `WorkerCard` (the spec component from
   §10), shown only when no search filter is active. The grid uses
   container queries (`grid-cols-1 @3xl:grid-cols-2 @5xl:grid-cols-3`) so
   it stays responsive within the `Sheet`. Eyebrow above the grid is a
   `label-caps-lg` heading sitting on a `border-t border-rule` separator.
2. **Search + index** — the spec's `SearchField` (24px display caret)
   immediately above a `border border-rule bg-bg` container with rows
   separated by `divide-y divide-rule-2`. Each row is a compact
   row-shaped variant of `WorkerCard` (no command block; just name +
   version + description + meta). Hover states use the same
   `bg-panel border-l-2 border-l-accent` rail that focused `WorkerCard`s
   use in §10.
3. **Search behaviour** — when the search field has a non-empty query, the
   featured row collapses entirely; only the filtered index is shown so
   that "featured" content never competes with results.
4. **Empty state** — the `Cell` from §10, with a `title` that names the
   missing thing and a one-line ink-faint body. Never a centered
   illustration; never an oversize fill.

This pattern keeps the **same surface tone** for both the showcase and the
index — the `Sheet` is unbroken from top to bottom — and uses the existing
primitives without inventing a new "hero card" shape.

### API reference panel

For documenting a worker's registered functions and triggers (the "api
reference" tab on the worker detail page), use a stacked, expanded-by-default
"datasheet" composition rather than a sidebar+console pattern. The whole
surface should read top-to-bottom like a printed engineering spec, with no
accordions or modals at the row level.

The composition has three nesting levels and uses one tonal step per level:

```
section eyebrow (label-caps)            <- bg page
  card        bg-panel head + bg body   <- per function/trigger
    pane      bg-paper-2 head + bg body <- per schema (request/response)
      tree    flat type-table           <- per field
```

1. **Sections** — two `Section`s (functions, triggers), each with a
   `label-caps-lg` eyebrow on the left and a small ink-ghost count on the
   right. Same chrome already used by other tabs on the worker page; do not
   wrap the section bodies in an additional border.
2. **Card** — a bordered `article` per function/trigger. The `bg-panel` head
   strip carries the name (`title-cell`) on the left, any `metadata.tags`
   rendered as `rule-2`-bordered label-caps-sm pills next to it, and the
   kind label (`FUNCTION` / `TRIGGER`) in label-caps on the right. Function
   and trigger cards share the exact same head; the only difference is
   what the name represents. **Triggers do not carry a separate "type"
   chip** — a trigger card *is* the definition of a trigger type, so the
   trigger's name (e.g. `http-endpoint`, `cron`, `queue`) is the type. Each
   card carries a stable `id` (`fn-<name>` / `trigger-<name>`) plus
   `scroll-mt-20` so that anchor navigation lands cleanly below the sticky
   site header.
3. **Pane** — the two schemas inside a card sit in a `@2xl:grid-cols-2`
   grid, separated by a 1px `bg-rule-2` gutter. Each pane is a
   `border-rule` block with a `bg-paper-2` head strip carrying a
   label-caps-sm eyebrow (`request` / `response` for functions,
   `invocation` / `return` for triggers). The `bg-paper-2` step makes the
   pane read as one tonal level "down" from the card's `bg-panel` head.
4. **Tree** — fields render as a flat type-table inside the pane body, one
   row per field. Each row is `name  type  required-marker  enum  constraints`
   on one line, with the description on a second line in
   `text-ink-faint text-[12px]`. Nested objects, arrays-of-objects, and
   union variants indent under the parent with a `border-l border-rule-2`
   guide. Nesting deeper than three levels collapses behind a native
   `<details>` row labelled "… expand N nested" so the surface stays
   client-JS-free.

**Rendering rule.** Schemas in this surface render as type tables — never
as raw JSON dumps. If a schema construct can't be expressed in the table
(e.g. JSON Schema `$ref` or vendor extensions), surface it as an inline
`label-caps` chip rather than dropping in a `<pre>` block.

**Accent rationing (§3).** The required-field marker (`*`) is the only
accent inside a schema tree. There is no accent on the card head — every
trigger row stays ink-on-paper, just like a function row.

**Empty states.** Use `Cell` for "no functions registered" / "no triggers
registered" within a present section, and a `StatusPanel variant="info"`
when the worker registers neither — same convention as the rest of the
worker page.

**Sidebar summary.** Pair the panel with an `api` card in the worker
page's right-hand `aside` (placed between `details` and `author`) listing
every function and trigger the worker exposes. The card uses the same
chrome as the other sidebar cards (`border-rule` block + `bg-panel` head
+ `label-caps-lg` title) and contains two sub-sections — `functions` and
`triggers` — each headed by a `label-caps-sm` eyebrow with a tabular
count on the right. Each row is a Next.js `Link` whose `href` always
includes `?tab=api#<anchor>`, where the anchor is the same id stamped on
the corresponding card. This makes the same link work from any tab:
clicking from `readme` switches to `api` and scrolls to the target,
clicking from `api` just scrolls. **Always show the card** when the
worker has any API surface — it doubles as a table of contents on the
api tab and as a discovery hint on the other tabs. Names render in their
original casing (e.g. `transcodeVideo`) — the lowercase rule does not
apply to identifiers.

**Sticky aside.** The sidebar `aside` is sticky inside the page's main
scroll container at `@4xl` and above:
`@4xl:sticky @4xl:top-4 @4xl:self-start @4xl:max-h-[calc(100dvh-2rem)] @4xl:overflow-y-auto`.
`self-start` is required so the grid cell sizes to its content (sticky
needs the element to be smaller than its containing block). The
`max-h` + internal `overflow-y-auto` is defensive — for very long
sidebars (e.g. a worker with 50 functions) the `api` card scrolls
internally instead of overflowing the viewport. At narrower widths the
aside stacks below the main column and is not sticky.

**Selection echo.** When a sidebar link is clicked, the corresponding
function/trigger card lights up with the canonical `border-l-2
border-l-accent` rail (the same recipe used by `WorkerCard` focused
state in §10 and the `versions` Row's `aria-current` style) and the
clicked sidebar link itself switches to `text-accent`. This gives the
user a clear "you are here" signal that survives any subsequent
scrolling. The 1px → 2px border shift on the left edge is the
sanctioned accent on the existing rule grid (§4 "Allowed accents on top
of borders").

**Why CSS `:target` is not enough.** Next.js `Link` performs same-page
hash navigation through `history.pushState`, which does **not** fire
`hashchange` and which most browsers do **not** treat as a `:target`
re-evaluation trigger. As a result, plain
`target:border-l-2 target:border-l-accent` only highlights on full page
load and back/forward — *not* on a soft sidebar click. The api panel
must drive the highlight from JS instead.

**Shared hash hub.** Both the sidebar links and the cards subscribe to
a single client-side hash hub
([`use-active-hash.ts`](app/src/components/api-reference/use-active-hash.ts)) — a module-level
`Set<Subscriber>` plus one global pair of `hashchange` + `popstate`
listeners that broadcast the latest `window.location.hash` to every
mounted subscriber. The hub also exports `setActiveHash(hash)` so click
handlers can dispatch optimistically the moment the user clicks, before
the URL has changed. There is exactly one source of truth for "what is
selected", and it is shared across the sidebar (which lives in the
`aside`) and the card grid (which lives in the main content column),
without any context provider crossing that distance.

Wiring:

- **Cards** wrap their `<article>` in a tiny client component
  ([`hash-card.tsx`](app/src/components/api-reference/hash-card.tsx))
  that calls `useActiveHash()` and conditionally adds
  `border-l-2 border-l-accent` when the hash matches its `id`. The card
  body (header, description, schema panes) is still a server-rendered
  React tree passed in as `children`, so the only JS that runs in the
  browser is the wrapper.
- **Sidebar links** ([`summary-list.tsx`](app/src/components/api-reference/summary-list.tsx))
  call `useActiveHash()` for their visual state and
  `setActiveHash(target)` from `onClick`, which broadcasts to the
  card subscribers immediately so the rail and the link light up in
  the same frame as the click — even though Next.js will not have
  fired `hashchange` yet.

This is also why the sidebar uses a single `SummaryList` client
component for *both* `functions` and `triggers` sub-sections rather
than one client component per section: with the hub providing a single
source of truth, splitting state would only cause stale highlights when
the user crossed sub-sections.

**Accent rationing.** Per §3, the selection still costs only one accent
moment per visible region: in the sidebar it is the active link; on the
panel it is the rail on the targeted card. The required-marker accent
inside the schema tree continues to coexist because it lives at a
different scale (a single character per row).
