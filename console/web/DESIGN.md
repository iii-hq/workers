---
version: beta
name: iii Schematic
description: A minimal, engineering-document design system for the iii engine. The system draws no lines — hierarchy comes entirely from layered surfaces and alpha-gray fills; warm cream paper in light, neutral grays from black in dark, with a single rationed accent (burnt orange on cream, electric blue on dark), one 6px corner radius everywhere, and a mono voice for technical data. The only sanctioned strokes are the focus ring and the very subtle `edge` frame around the floating workspace panels.
colors:
  bg: "#f2f0ed"
  sidebar: "#edeae5"
  panel: "lab(98.26% 0 0)"
  panel-raised: "#f7f5f2"
  paper: "#f2f0ed"
  paper-2: "#ebe8e3"
  surface: "rgba(20, 16, 8, 0.055)"
  surface-hover: "rgba(20, 16, 8, 0.085)"
  surface-selected: "rgba(184, 66, 15, 0.12)"
  surface-active: "rgba(20, 16, 8, 0.12)"
  ink: "#0a0a0a"
  ink-2: "#1a1a1a"
  ink-soft: "#1a1a1a"
  ink-faint: "#6b6865"
  ink-ghost: "#a3a09c"
  ink-disabled: "#b8b4ae"
  mute: "#6b6865"
  mute-2: "#a3a09c"
  rule: "transparent"
  rule-2: "transparent"
  rule-strong: "transparent"
  rule-focus: "rgba(184, 66, 15, 0.6)"
  edge: "rgba(20, 16, 8, 0.08)"
  accent: "#b8420f"
  accent-fg: "#f2f0ed"
  accent-hover: "#a53a0c"
  accent-muted: "rgba(184, 66, 15, 0.1)"
  accent-border: "rgba(184, 66, 15, 0.35)"
  alert: "#ff0026"
  alert-muted: "rgba(255, 0, 38, 0.08)"
  warn: "#a87a00"
  warn-muted: "rgba(168, 122, 0, 0.12)"
  ok: "#356f3d"
  ok-muted: "rgba(53, 111, 61, 0.12)"
  bg-dark: "#0a0a0a"
  sidebar-dark: "#0e0e0e"
  panel-dark: "#111111"
  panel-raised-dark: "#171717"
  paper-2-dark: "#171717"
  surface-dark: "rgba(255, 255, 255, 0.055)"
  surface-hover-dark: "rgba(255, 255, 255, 0.085)"
  surface-selected-dark: "rgba(40, 168, 247, 0.14)"
  surface-active-dark: "rgba(255, 255, 255, 0.12)"
  ink-dark: "#ededed"
  ink-faint-dark: "#a6a6a6"
  ink-ghost-dark: "#6f6f6f"
  ink-disabled-dark: "#4d4d4d"
  rule-dark: "transparent"
  rule-2-dark: "transparent"
  rule-strong-dark: "transparent"
  rule-focus-dark: "rgba(40, 168, 247, 0.7)"
  edge-dark: "rgba(255, 255, 255, 0.07)"
  accent-dark: "#28a8f7"
  accent-hover-dark: "#46b6fa"
  accent-muted-dark: "rgba(40, 168, 247, 0.12)"
  accent-border-dark: "rgba(40, 168, 247, 0.35)"
  alert-dark: "#f05d68"
  warn-dark: "#f5a524"
  ok-dark: "#36c98f"
typography:
  logo:
    fontFamily: Geist Mono
    fontSize: 18px
    fontWeight: 600
    lineHeight: 1
    letterSpacing: -0.02em
  display-hero:
    fontFamily: Geist
    fontSize: 72px
    fontWeight: 600
    lineHeight: 1.02
    letterSpacing: -0.02em
  display-foot:
    fontFamily: Geist
    fontSize: 48px
    fontWeight: 600
    lineHeight: 1.05
    letterSpacing: -0.03em
  headline-section:
    fontFamily: Geist
    fontSize: 28px
    fontWeight: 500
    lineHeight: 1.25
    letterSpacing: -0.01em
  headline-card:
    fontFamily: Geist
    fontSize: 20px
    fontWeight: 500
    lineHeight: 1.1
    letterSpacing: -0.02em
  title-cell:
    fontFamily: Geist Mono
    fontSize: 16px
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: -0.01em
  body-md:
    fontFamily: Geist
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.7
  body-sm:
    fontFamily: Geist
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.7
  code-md:
    fontFamily: Geist Mono
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.65
  code-sm:
    fontFamily: Geist Mono
    fontSize: 12.5px
    fontWeight: 400
    lineHeight: 1.55
  label-caps-lg:
    fontFamily: Geist Mono
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.18em
  label-caps-md:
    fontFamily: Geist Mono
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.14em
  label-caps-sm:
    fontFamily: Geist Mono
    fontSize: 11px
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0.06em
  micro:
    fontFamily: Geist Mono
    fontSize: 9px
    fontWeight: 400
    lineHeight: 1
    letterSpacing: 0.04em
rounded:
  none: 0px
  xs: 6px
  sm: 6px
  md: 6px
  lg: 6px
  xl: 6px
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
    rounded: "{rounded.md}"
    padding: 12px 20px
  button-primary-hover:
    backgroundColor: "{colors.ink}"
    opacity: 0.9
  button-ghost:
    backgroundColor: transparent
    textColor: "{colors.ink-faint}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 12px 20px
  button-ghost-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.ink}"
  button-pill:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 6px 12px
  button-pill-hover:
    backgroundColor: "{colors.surface-hover}"
  button-icon:
    backgroundColor: transparent
    textColor: "{colors.ink-faint}"
    rounded: "{rounded.sm}"
    size: 30px
  button-icon-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.ink}"
  nav-link:
    typography: "{typography.body-sm}"
    textColor: "{colors.mute}"
    padding: 6px 0
  nav-link-hover:
    textColor: "{colors.ink}"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.md}"
    padding: 20px
  card-focus:
    backgroundColor: "{colors.panel-raised}"
  card-head:
    backgroundColor: "{colors.panel-raised}"
    typography: "{typography.label-caps-lg}"
    textColor: "{colors.ink-faint}"
    padding: 10px 14px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 10px 12px
  input-placeholder:
    textColor: "{colors.ink-ghost}"
  input-hover:
    backgroundColor: "{colors.surface-hover}"
  input-focus:
    borderColor: "{colors.rule-focus}"
    ring: "3px {colors.accent-muted}"
  badge-numeric:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-fg}"
    typography: "{typography.label-caps-sm}"
    rounded: "{rounded.xs}"
    padding: 0 4px
    height: 16px
  status-dot:
    backgroundColor: "{colors.accent}"
    rounded: "{rounded.full}"
    size: 6px
  code-block:
    backgroundColor: "{colors.bg}"
    typography: "{typography.code-sm}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    padding: 18px 20px
  terminal-button:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 10px 14px
  terminal-prompt:
    textColor: "{colors.accent}"
  toggle-track:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.sm}"
    padding: 2px
  toggle-active:
    backgroundColor: "{colors.accent-muted}"
    textColor: "{colors.ink}"
    rounded: "{rounded.xs}"
  toggle-inactive:
    backgroundColor: transparent
    textColor: "{colors.mute}"
---

# iii Schematic — design system

This document is a self-contained, portable spec for the iii Schematic UI:
an engineering-document web UI built from layered surfaces and alpha-gray
fills — warm cream paper in light, neutral grays from black in dark — with
Geist for UI text, Geist Mono for technical data, one 6px corner radius
everywhere, and a single rationed accent. The system draws no lines: the
only sanctioned strokes are the focus ring and the subtle `edge` frame
around the floating workspace panels. Everything needed to reproduce the
system in another project lives inside this file — there are no links to
repository sources.

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

### Fonts

Two families, loaded via `@fontsource` at the top of the global stylesheet:
**Geist** (the UI sans — navigation, buttons, body copy, headings) and
**Geist Mono** (the technical voice — trace names, IDs, timestamps, metrics,
code). Chivo Mono stays in the mono fallback stack for legacy glyph parity.
Weights 400/500/600 cover the entire scale.

```css
@import "@fontsource/geist/400.css";
@import "@fontsource/geist/500.css";
@import "@fontsource/geist/600.css";
@import "@fontsource/geist-mono/400.css";
@import "@fontsource/geist-mono/500.css";
@import "@fontsource/geist-mono/600.css";
@import "@fontsource/chivo-mono/400.css";
@import "@fontsource/chivo-mono/500.css";
@import "@fontsource/chivo-mono/600.css";
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
  --font-sans:
    "Geist", ui-sans-serif, system-ui, sans-serif, "Apple Color Emoji",
    "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji";
  --font-mono:
    "Geist Mono", "Chivo Mono", ui-monospace, SFMono-Regular, Menlo, Monaco,
    Consolas, "Liberation Mono", "Courier New", monospace;

  /* ── Surface ramp ─────────────────────────────────────────────────────
     Layered surfaces are the ONLY structural tool — the system draws no
     lines: bg (canvas) → sidebar → panel → panel-raised, then the
     component fills surface → surface-hover → surface-selected →
     surface-active. The base layers are solid; the component fills are
     alpha grays so one step reads identically over any base layer.
     `paper-2` is the legacy alias for panel-raised. */
  --color-bg: #f2f0ed;
  --color-sidebar: #edeae5;
  --color-panel: lab(98.26% 0 0);
  --color-panel-raised: #f7f5f2;
  --color-paper-2: #ebe8e3;
  --color-surface: rgba(20, 16, 8, 0.055);
  --color-surface-hover: rgba(20, 16, 8, 0.085);
  --color-surface-selected: rgba(184, 66, 15, 0.12);
  --color-surface-active: rgba(20, 16, 8, 0.12);

  /* ── Ink ramp ──────────────────────────────────────────────────────── */
  --color-ink: #0a0a0a;
  --color-ink-faint: #6b6865;
  --color-ink-ghost: #a3a09c;
  --color-ink-disabled: #b8b4ae;

  /* ── Border ramp ──────────────────────────────────────────────────────
     The system draws no lines: rule/rule-2/rule-strong resolve to
     transparent in both themes (legacy `border-rule*` utilities become
     inert 1px transparent borders, so layout never shifts). Hierarchy is
     carried by the surface fills above. The single exception is
     rule-focus — the focus indicator on inputs and controls, which must
     stay visible for accessibility. */
  --color-rule: transparent;
  --color-rule-2: transparent;
  --color-rule-strong: transparent;
  --color-rule-focus: rgba(184, 66, 15, 0.6);

  /* The one structural stroke the system keeps: a VERY subtle edge on the
     main workspace panels (the tab columns), so the floating panels
     read against the canvas. Never used inside a panel. */
  --color-edge: rgba(20, 16, 8, 0.08);

  /* accent (single hero — burnt orange on cream, blue on dark) */
  --color-accent: #b8420f;
  --color-accent-fg: #f2f0ed;
  --color-accent-hover: #a53a0c;
  --color-accent-muted: rgba(184, 66, 15, 0.1);
  --color-accent-border: rgba(184, 66, 15, 0.35);

  /* status (each with a muted fill for tinted backgrounds) */
  --color-alert: #ff0026;
  --color-alert-muted: rgba(255, 0, 38, 0.08);
  --color-warn: #a87a00;
  --color-warn-muted: rgba(168, 122, 0, 0.12);
  --color-ok: #356f3d;
  --color-ok-muted: rgba(53, 111, 61, 0.12);

  /* ── Radii ────────────────────────────────────────────────────────────
     One radius everywhere: every step of the Tailwind scale resolves to
     6px, so badges, buttons, cards, panels, and modals share the same
     corner. Only `none` (main columns' outer frame edge cases) and `full`
     (dots, round action buttons) differ. */
  --radius-none: 0px;
  --radius-xs: 6px;
  --radius-sm: 6px;
  --radius-md: 6px;
  --radius-lg: 6px;
  --radius-xl: 6px;
  --radius-full: 9999px;

  /* ── Elevation (the only two sanctioned shadows) ────────────────────── */
  --shadow-raised:
    0 1px 0 rgba(255, 255, 255, 0.025) inset, 0 8px 24px rgba(0, 0, 0, 0.18);
  --shadow-floating:
    0 1px 0 rgba(255, 255, 255, 0.03) inset, 0 12px 32px rgba(0, 0, 0, 0.28);

  /* ── Motion ─────────────────────────────────────────────────────────── */
  --ease-glide: cubic-bezier(0.2, 0.8, 0.2, 1);

  /* spacing scale (carried from the YAML) */
  --spacing-gutter: 24px;
  --spacing-section-x: 36px;
  --spacing-section-y: 80px;
  --spacing-sheet-max: 1200px;
  --spacing-content-max: 1216px;
}

[data-theme="dark"] {
  /* Neutral grays derived from black — no blue cast in the base ramp. The
     component fills (surface*) are white-alpha so a step reads identically
     over any base layer; the ONLY chromatic surface is surface-selected,
     the blue selection tint. Borders are gone: rule/rule-2/rule-strong
     resolve to transparent, and hierarchy is carried entirely by fills. */
  --color-bg: #0a0a0a;
  --color-sidebar: #0e0e0e;
  --color-panel: #111111;
  --color-panel-raised: #171717;
  --color-paper-2: #171717;
  --color-surface: rgba(255, 255, 255, 0.055);
  --color-surface-hover: rgba(255, 255, 255, 0.085);
  --color-surface-selected: rgba(40, 168, 247, 0.14);
  --color-surface-active: rgba(255, 255, 255, 0.12);
  --color-ink: #ededed;
  --color-ink-faint: #a6a6a6;
  --color-ink-ghost: #6f6f6f;
  --color-ink-disabled: #4d4d4d;
  --color-rule: transparent;
  --color-rule-2: transparent;
  --color-rule-strong: transparent;
  --color-rule-focus: rgba(40, 168, 247, 0.7);
  --color-edge: rgba(255, 255, 255, 0.07);
  --color-accent: #28a8f7;
  --color-accent-fg: #070909;
  --color-accent-hover: #46b6fa;
  --color-accent-muted: rgba(40, 168, 247, 0.12);
  --color-accent-border: rgba(40, 168, 247, 0.35);
  --color-alert: #f05d68;
  --color-alert-muted: rgba(240, 93, 104, 0.12);
  --color-warn: #f5a524;
  --color-warn-muted: rgba(245, 165, 36, 0.12);
  --color-ok: #36c98f;
  --color-ok-muted: rgba(54, 201, 143, 0.12);
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
  /* rule is transparent (no-lines system) — the thumb needs its own
     visible alpha gray. */
  ::-webkit-scrollbar-thumb {
    background: color-mix(in oklab, var(--color-ink) 22%, transparent);
  }
  ::-webkit-scrollbar-thumb:hover {
    background: color-mix(in oklab, var(--color-ink) 38%, transparent);
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

/* transient shadow stack — used only for the "deal" stack animation on
   language cards (default elevation goes through shadow-raised /
   shadow-floating from the theme) */
@utility deal-shadow {
  box-shadow: -2px 0 0 var(--color-rule),
    -16px 4px 36px -10px rgba(0, 0, 0, 0.22);
}

/* Function/tool-call card chrome — an accent-tinted raised surface (a soft
   gradient wash over panel-raised, no border) so calls read as special
   without any outline. Derived from the accent token, so it tints blue in
   dark mode and burnt orange in light mode. */
@utility fcall-chrome {
  background:
    linear-gradient(
      180deg,
      color-mix(in oklab, var(--color-accent) 7%, transparent),
      color-mix(in oklab, var(--color-accent) 3%, transparent)
    ),
    var(--color-panel-raised);
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

The app should feel like an **engineering document**, not a SaaS dashboard —
technical, precise, dense without feeling cramped, structured without
looking boxed.

- The page is built from **layered surfaces**: hierarchy comes from a
  one-step background difference, full stop. The system draws **no lines**
  — no outlines on controls, no dividers between rows or regions. Exactly
  two strokes are sanctioned: the focus indicator (`rule-focus`
  border/ring) on a focused control, and the very subtle `edge` frame
  around the floating workspace panels.
- Dark mode is a **first-class layered system** — neutral grays derived
  from black (`#0a0a0a → #171717`, no blue cast) with white-alpha component
  fills, not an inverted paper ramp.
- One corner radius: **6px everywhere** (every Tailwind radius step
  resolves to it); the shapes stay disciplined, not soft or
  consumer-playful. Lowercase voice throughout.
- Color is rationed: the palette is essentially **ink-on-surface**, broken
  by a single accent (burnt orange on cream, electric blue on dark) reserved
  for selected, focused, active, and live states.
- The personality is **technical but unintimidating** — the same energy as a
  well-kept lab notebook or a hand-drawn architecture diagram. It must feel
  built by engineers, for engineers, and for the agents working alongside
  them.
- Density is deliberate: spec sheets, code, traces, and console panels
  coexist on the same surface without a hierarchy contest. Trace
  visualizations stay the most colorful area of the UI.

### Voice

- All UI copy is **lowercase**, including headlines, buttons, and nav items.
- Headlines treat sentence fragments as visual blocks (e.g. *"any task. one
  experience."*).
- Numbers and metadata always use **tabular monospace**, never proportional
  figures.
- The wordmark is pronounced *"three eye"* — every "i" stays lowercase.

> If you removed all the type, the page should still read as a structured
> document. Surfaces establish hierarchy; color marks state. Lines never
> appear.

---

## 2. Typography — two families (Geist + Geist Mono)

**Geist** (`--font-sans`) carries the UI: navigation, conversation titles,
buttons, inputs, chat content, empty states, headings, labels. **Geist
Mono** (`--font-mono`) carries everything technical: trace names, worker
names, function names, IDs, timestamps, metrics, span labels, filter
expressions, code-like values.

**Rule:** if a human wrote it, it's sans; if the machine produced it (or a
machine will parse it), it's mono. Don't add a third family — variety comes
from weight, scale, case, and letter-spacing, not more fonts. Chat and
configuration surfaces remap incidental `.font-mono` chrome back to Geist
(see the `.chat-surface` / `.configuration-surface` rules in `index.css`);
function-trigger cards run Geist Mono throughout.

Decorative ligatures are explicitly disabled on mono surfaces
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
(`bg-bg`, `text-ink-faint`, `text-accent`, `bg-surface`, …) — **never** the
raw CSS variable.

### Surface ramp (the depth axis)

The base layers (`bg` → `panel-raised`) are solid neutral tones; the
component fills (`surface*`) are **alpha grays**, so one step reads
identically over any base layer. `surface-selected` is the only chromatic
surface — the accent-tinted selection fill (blue in dark, burnt orange in
light).

| Token              | Use                                                        |
| ------------------ | ---------------------------------------------------------- |
| `bg`               | Application canvas — the deepest layer                     |
| `sidebar`          | Left navigation/sidebar (one step off the canvas)          |
| `panel`            | Main chat and traces columns                               |
| `panel-raised`     | Composer, trace details, popovers, active tool cards       |
| `paper-2`          | Legacy alias for `panel-raised` (kept for existing code)   |
| `surface`          | Inputs, controls, pills, chips, secondary cards            |
| `surface-hover`    | Hover state on rows, items, and ghost controls             |
| `surface-selected` | Selected conversation, trace row, or list item (accent tint) |
| `surface-active`   | Strong active/pressed state                                |

### Ink (4-step contrast)

| Token          | Use                                                |
| -------------- | -------------------------------------------------- |
| `ink`          | Primary type, wordmark, primary buttons            |
| `ink-faint`    | Body in muted contexts, captions, inactive nav     |
| `ink-ghost`    | Line numbers, placeholders, timestamps             |
| `ink-disabled` | Disabled labels (paired with reduced opacity)      |

### Rules (there are none)

`rule`, `rule-2`, and `rule-strong` resolve to `transparent` in both
themes. They exist only so legacy `border-rule*` / `divide-rule*`
utilities stay inert (1px transparent — layout never shifts) instead of
breaking. Never design with them.

| Token         | Value        | Meaning                                       |
| ------------- | ------------ | --------------------------------------------- |
| `rule`        | transparent  | Legacy default border — draws nothing         |
| `rule-2`      | transparent  | Legacy subtle divider — draws nothing         |
| `rule-strong` | transparent  | Legacy emphasis border — draws nothing        |
| `rule-focus`  | accent ~60–70% alpha | The interactive stroke: the focus indicator on inputs and controls |
| `edge`        | ink ~7–8% alpha | The structural stroke: the VERY subtle frame around the floating workspace panels (the tab columns) — never used inside a panel |

### Accent (single hero — burnt orange on cream, electric blue on dark)

`accent`, `accent-fg`, `accent-hover`, `accent-muted` (10–12%-alpha fill),
`accent-border` (35%-alpha, legacy — prefer `accent-muted` fills). Reserved
for: selected navigation and segmented controls (`accent-muted` fill),
focused inputs (`rule-focus`), active filters, live state, the selected
conversation/trace (`surface-selected` fill), the primary technical action,
the `$` prompt. **Never** for body text, large fills, or outlines.

### Status

`alert`, `warn`, `ok` — each with a `-muted` 8–12%-alpha fill for tinted
backgrounds. Status is expressed through **text color + a small icon/dot on
a muted tinted fill**, never a full-saturation background and never a
stripe or outline.

| Token    | Use                                       |
| -------- | ----------------------------------------- |
| `accent` | live / running / focused / selected       |
| `ok`     | success, completed calls, diff additions  |
| `alert`  | error states (traces, status panels)      |
| `warn`   | warning states, pending approval          |

### Dark theme

Override the same tokens inside a `[data-theme="dark"]` block (see §0). Dark
is a neutral gray ramp derived from black (`#0a0a0a → #0e0e0e → #111111 →
#171717` — no blue cast), white-alpha component fills
(`rgba(255,255,255,0.055 → 0.12)`), neutral light ink (`#ededed → #a6a6a6 →
#6f6f6f`), and the accent swapped to electric blue (`#28a8f7`) — which also
tints `surface-selected`. Same structural logic; the blue lives only in
state, never in the grays.

To follow the OS, set the attribute on load:

```ts
const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches
document.documentElement.dataset.theme = isDark ? 'dark' : 'light'
```

---

## 4. The "surfaces, not borders" rule (key composition pattern)

> **A one-step surface difference defines a region. The system draws no
> lines — exactly two strokes are sanctioned: the focus ring, and the
> `edge` frame around the floating workspace panels.**

Structure comes from the layered surface ramp (§3): a new region means a new
background step, never an outline and never a divider. Controls are
alpha-gray fills; rows separate by their hover/selected fills; regions
separate by base-layer steps; overlays separate by `panel-raised` +
`shadow-floating`.

Two strokes are allowed, each with one job:

1. **Focus** — a focused field swaps its (transparent) border to
   `rule-focus` and gains a soft 3px accent ring; keyboard focus on
   buttons uses the same `ring-rule-focus`.
2. **Panel edges** — the main workspace panels (each
   workspace-tab column) float on the canvas as
   `rounded-sm border border-edge bg-panel` with 6px gutters; the `edge`
   stroke is a VERY subtle ink-alpha frame that keeps a panel readable
   against the canvas. It is never used inside a panel — interior
   hierarchy stays fill-only.

Nothing else in the chrome may draw a line. (Data visualizations are
exempt: charts may draw connector and grid lines with explicit alpha-ink
fills, e.g. `bg-ink/15` elbows and `bg-ink/8` time-grid guides in the
trace timeline.)

### Pattern

```tsx
<div className="rounded-md bg-surface overflow-hidden">
  <div className="px-3.5 py-2.5 bg-panel-raised font-mono text-[12px] uppercase tracking-[0.18em] text-ink-faint">
    title
  </div>
  <div className="p-5">{/* body */}</div>
</div>
```

The header strip is one surface step off the body — that difference IS the
separator. No divider, no outer outline.

### Selection and severity

- Selected row, card, conversation, or trace: `bg-surface-selected` — the
  accent-tinted fill. **No left rail, no outline.**
- Active segment / tab / filter: `bg-accent-muted` fill.
- Row severity: the status's `-muted` tinted fill (e.g. `bg-alert-muted`)
  plus the status text color and dot — no stripe.
- Legacy `divide-y divide-rule-2` / `border-b border-rule-2` classes are
  inert (transparent) — don't add new ones.

### Shadows

Two sanctioned elevation shadows live in the theme: `shadow-raised` (raised
cards — composer, trace detail) and `shadow-floating` (popovers, dialogs,
dropdowns, tooltips). `.deal-shadow` remains a transient animation cue on
the language-card stack. No heavy glows — the only glow is the live
`pulse-dot`.

**When in doubt:** step the background one level. Never reach for a border.

---

## 5. Surfaces & elevation

The surface ramp is the primary tonal tool. Think of it as a depth axis:
every step up reads as "closer" without a single stroke.

```
bg                  application canvas
 └─ sidebar            left navigation
 └─ panel              main chat / traces columns
     └─ panel-raised      composer, trace detail, popovers, tool cards
         └─ surface          inputs, controls, pills, secondary cards
             └─ surface-hover     hover
             └─ surface-selected  selected row / trace / conversation
             └─ surface-active    pressed / segmented-control selection
```

Interactive containers walk the state sub-ramp (`surface →
surface-hover → surface-selected → surface-active`); nothing about their
edges ever changes except the focus ring.

### Where elevation actually shows up

1. **Tonal layering** — the ramp above; a one-step difference is the only
   separator between regions. The alpha-gray component fills compose over
   any base layer.
2. **Sanctioned shadows** — `shadow-raised` on raised in-flow cards
   (composer, expanded trace detail) and `shadow-floating` on overlays
   (dropdowns, popovers, dialogs, tooltips, hover cards). Both carry a 1px
   white inset highlight so raised surfaces catch light in dark mode.
3. **Accent-tinted chrome** — the `.fcall-chrome` utility (§0): a soft
   accent gradient wash over `panel-raised` (no border), reserved for
   function/tool-call cards so agent actions read as special.
4. **Pulse + glow** — the live state uses `.pulse-dot` (1.6s expanding
   `box-shadow` ring) on a 6px accent dot. This is the only "glow" the
   system allows.

Dark mode keeps the same surface-driven hierarchy — neutral grays stepping
up from black. The only visible edge anywhere is the workspace panels'
`edge` frame (§4).

---

## 6. Radii & shape

**One radius: 6px, globally.** Every step of the Tailwind scale
(`rounded-xs` through `rounded-xl`) resolves to 6px, so badges, chips,
buttons, inputs, sidebar rows, cards, tool/function-call cards, popovers,
dropdowns, dialogs, the composer, and the floating workspace panels all
share the same corner. There is no per-component scale to choose from —
write whichever step reads naturally (`rounded-sm` is the conventional
spelling) and it renders 6px.

| Token                 | Value  | Used for                                 |
| --------------------- | ------ | ---------------------------------------- |
| `rounded-none`        | 0px    | Full-bleed edge cases only               |
| `rounded-xs` … `-xl`  | 6px    | Everything — one corner everywhere       |
| `rounded-full`        | 9999px | Status dots, pills, round action buttons |

Don't invent in-between values, and don't reach for `rounded-full` on
anything that isn't genuinely circular.

### Stroke weight

- UI chrome draws no strokes, with two exceptions: the focus indicator — a
  1px `rule-focus` border (inputs) or a 2px `ring-rule-focus` (buttons) —
  and the 1px `edge` frame around the floating workspace panels (§4).
- SVG **data** diagrams use 1–1.25px strokes; thicker strokes (1.25px
  `accent`) are reserved for emphasized worker connections. Charts draw
  their connectors/grids with explicit alpha-ink fills (`bg-ink/15`,
  `bg-ink/8`).
- The wordmark is six rectangles (three "i"s, each a stem + a tittle), all
  sharing the same square unit. The mark is the design system in miniature:
  identical units, no curves, deliberate negative space.

---

## 7. Spacing & layout

### Console workspace

The console shell is a set of **floating panels over the canvas**: the
header holds server-persisted closable tabs (stored in the `console`
configuration entry under `workspace.tabs` — model in
`lib/workspace-tabs.ts`), and each tab shows one or two screens (any page
or the chat view) as equal columns; an unattached column renders an
attach affordance instead of a page. Each column renders as `rounded-sm border border-edge bg-panel` with 6px
gutters between panels and against the viewport, so the canvas shows
through as the seam. That gutter + edge frame is the entire column
chrome; panels draw no other lines.

### Sheet pages

The page is a vertical sheet, max 1200px, sitting on the canvas surface.
There is no hero card — the entire site reads as one continuous spec sheet.

- **Sheet:** `min(1200px, 100%)` centered (see `Sheet` in §10). The sheet
  itself is unbordered; the surface step against the canvas defines it.
- **Sticky nav:** `py-4.5` vertical padding, collapses to `py-2.5` on
  scroll. No border — it reads as chrome by sitting on its own surface.
- **Section padding:** `px-9` on desktop (36px), `px-4.5` on tablet (18px),
  `px-3.5` on small mobile (14px). Vertical: `py-20` to `py-24` (80–96px)
  between major sections.
- **Hero grid:** Two equal columns (`1fr 1fr`) with a 64px gap on desktop;
  collapses to a single column under 880px.
- **Feature grids:** Always **3-up** on desktop (`grid-cols-3`), each cell
  a `rounded-md bg-surface` card separated by grid `gap` — never by drawn
  rules.
- **Card padding:** 12px compact, 20–24px default. In-flow cards carry no
  shadow unless raised (`shadow-raised`); overlays use `shadow-floating`.
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

`StatusPanel` (§10) is the canonical status display — a rounded row on a
muted tinted fill, monospaced, icon + headline + detail. No stripe, no
outline in any variant; the tint + icon + headline color carry the
severity.

| Variant     | Icon + headline | Body fill     |
| ----------- | --------------- | ------------- |
| `v-info`    | `ink`           | `surface`     |
| `v-success` | `ok`            | `ok-muted`    |
| `v-warn`    | `warn`          | `warn-muted`  |
| `v-alert`   | `alert`         | `alert-muted` |

**In tables and traces:** the same recipe — the status's `-muted` tinted
background plus its text color and dot. Never a stripe, ring, or solid
status background. See `Trace` (§10).

**Dot semantics:** green (`ok`) means completed; blue (`accent`) is reserved
for live/running (animated) and selected states; `warn` for pending; `alert`
for failed. For inline "live" emphasis, pair a `StatusDot` with the
`pulse-dot` utility — the only sanctioned glow.

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

6px circle. Optional `.pulse-dot` glow for "live" emphasis. Tone semantics
per §9: `ok` = completed, `accent` = live/selected, `warn` = pending,
`alert` = failed.

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

type DotTone = 'accent' | 'alert' | 'warn' | 'ink' | 'ok'

const dotTone: Record<DotTone, string> = {
  accent: 'bg-accent',
  alert: 'bg-alert',
  warn: 'bg-warn',
  ink: 'bg-ink',
  ok: 'bg-ok',
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

Variants: `primary` (ink fill, 6px radius — reads as a light chip in dark
mode), `ghost` (borderless, faint text → `surface-hover` fill on hover),
`pill` (compact borderless `surface` chip), `icon` (30×30 borderless,
`surface-hover` on hover), `terminal` (full-width borderless `surface` row
with `$` prompt + command + optional copy), `wiggle` (primary + corner badge
slot + `.wiggle`). Sizes: `sm` `md` `lg` `icon`. Focus is a 2px
`rule-focus` ring — the only stroke a button ever shows. Depends on
`class-variance-authority` and `@radix-ui/react-slot`.

```tsx
import * as React from 'react'
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-x-2 whitespace-nowrap font-mono lowercase rounded-sm transition-[background-color,color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 select-none',
  {
    variants: {
      variant: {
        primary:
          'bg-ink text-bg border border-transparent hover:bg-ink/90 rounded-md',
        ghost:
          'bg-transparent text-ink-faint border border-transparent hover:bg-surface-hover hover:text-ink',
        pill:
          'bg-surface text-ink border border-transparent hover:bg-surface-hover',
        icon:
          'bg-transparent text-ink-faint border border-transparent hover:bg-surface-hover hover:text-ink',
        terminal:
          'bg-surface text-ink border border-transparent justify-start hover:bg-surface-hover',
        wiggle:
          'wiggle bg-ink text-bg border border-transparent hover:bg-ink/90 rounded-md relative',
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
headline, 12px ink-faint detail. Variants are a **muted tinted fill** —
no stripe, no outline (§9).

```tsx
import * as React from 'react'
import { cn } from '@/lib/utils'

export type StatusVariant = 'info' | 'success' | 'warn' | 'alert'

const variantTone: Record<
  StatusVariant,
  { fill: string; icon: string; headline: string }
> = {
  info: {
    fill: 'bg-surface',
    icon: 'text-ink',
    headline: 'text-ink',
  },
  success: {
    fill: 'bg-ok-muted',
    icon: 'text-ok',
    headline: 'text-ok',
  },
  warn: {
    fill: 'bg-warn-muted',
    icon: 'text-warn',
    headline: 'text-warn',
  },
  alert: {
    fill: 'bg-alert-muted',
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
        'flex items-start gap-x-3 rounded-md px-3.5 py-3',
        tone.fill,
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

A `bg` well — the canvas tone punching through the surface it sits on, no
border — monospace at 12.5px / line-height 1.55. Light syntax tinting:
comments italic-ghost, strings in `accent`, keywords bold-ink, numbers in
`alert`. The accent orange is reserved for **string literals and the active
call return**, not arbitrary keywords.

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
        'rounded-sm bg-bg overflow-x-auto px-5 py-4 font-mono text-[12.5px] leading-[1.55] text-ink',
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

Header strip in `panel-raised` with a label-caps title; `surface` body with
an orange `$` prompt, ink command, and an animated 6×13 ink caret. The
head/body surface step is the separator — no divider.

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
    <div className={cn('rounded-md bg-surface overflow-hidden', className)}>
      {title ? (
        <div className="bg-panel-raised px-3.5 py-2 font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
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
and a label-caps status. A waterfall of horizontal bars follows: `surface`
track, `ink` fill, `alert` fill for error spans. Rows separate by spacing —
no dividers.

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
  { dot: 'ok' | 'warn' | 'alert'; label: string; bar: string }
> = {
  ok: { dot: 'ok', label: 'text-ok', bar: 'bg-ink' },
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
    <div className={cn('rounded-md bg-surface overflow-hidden', className)}>
      <div className="bg-panel-raised px-3.5 py-2 font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
        {title}
      </div>
      <ul>
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
              <div className="col-span-4 mt-1 h-1 bg-surface relative">
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

The universal container for short prose blocks (features, hellos,
pull-quotes, empty states): a borderless `surface` card with a 6px radius
and 20px padding, optional 16px Semi-Bold ink title, 13px ink-faint body
capped at ~34ch.

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
    <div className={cn('rounded-md bg-surface p-5', className)}>
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
Focused state switches the body fill from `surface` to `surface-selected`
(the accent tint) — no rail, no outline.

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
        'w-[400px] rounded-md overflow-hidden transition-colors',
        focused ? 'bg-surface-selected' : 'bg-surface',
        className,
      )}
    >
      <header className="flex items-center justify-between px-4 py-3">
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
      <div className="bg-panel font-mono text-[12.5px] text-ink px-4 py-2">
        {command}
      </div>
      <footer className="flex items-center justify-between px-4 py-2">
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
              'rounded-md bg-panel-raised overflow-hidden',
              isUnder && 'absolute inset-x-0 -z-10 deal-shadow',
            )}
            style={isUnder ? { top: `${idx * 120}px` } : undefined}
          >
            <header className="flex items-center justify-between px-4 py-2 bg-panel font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
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

The standard input recipe: a rounded `surface` fill whose only stroke is
the `rule-focus` underline while focused. The submit arrow lives inside the
row, right-aligned. Helper text below is label-caps in `ink-ghost`.

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
      <div className="flex items-center gap-x-2 rounded-sm bg-surface px-3 border-b border-transparent focus-within:border-rule-focus transition-colors">
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

Compact segmented control on a `surface` track (no border, 6px radius, 2px
internal padding). Active segment: `accent-muted` fill (the selection
tint), `ink` text, 6px radius. Inactive: transparent, `ink-faint` text.

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
      className={cn(
        'inline-flex rounded-sm bg-surface p-[2px] gap-[2px]',
        className,
      )}
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
              'font-mono text-[13px] px-3 py-1 rounded-xs transition-colors lowercase',
              active
                ? 'bg-accent-muted text-ink'
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

16px tall, 4px horizontal padding, `accent` fill, 6px radius. Sits on
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
        'inline-flex items-center justify-center rounded-xs bg-accent text-accent-fg font-mono text-[11px] font-medium uppercase tracking-[0.06em] px-1 h-4 tabular-nums',
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
defined by its surface against the canvas (no outer rule). `PageHeader`
carries an `eyebrow` (rendered through
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
        'mx-auto w-full max-w-[1200px] min-h-screen bg-bg',
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
  tracking — used sparingly for small section labels.
- Lean on **layered surfaces** to define structure — a one-step background
  difference is the only separator. The sole visible stroke anywhere in the
  chrome is the focus indicator (`rule-focus`).
- **Ration the accent** (burnt orange on cream, electric blue on dark) to
  selected, focused, active, and live states. One accent moment per visible
  region — the rest is ink, faint, and ghost.
- Mark selection with the **`surface-selected` accent-tinted fill** — the
  canonical "you are here" signal. No rails, no outlines.
- Use the **one 6px radius everywhere** (§6) — every Tailwind radius step
  resolves to it, so there is no per-component choice to make.
- Keep the `iii` wordmark in lowercase, with all six rectangles in `ink`
  (never colored).
- Use **Geist for UI text and Geist Mono for technical data** (trace names,
  IDs, timestamps, metrics, code). Two families, never a third.
- Use the **Tailwind utility** (`bg-surface`, `text-ink-faint`,
  `text-accent`) — never the raw CSS variable.
- Use **container queries** (`@container`, `@3xl:`, `@4xl:`) for panel-level
  responsiveness. Pages set `@container` and grids split with `@3xl:`.
- Use **`tabular-nums`** on every number, timestamp, and KPI.
- Keep motion minimal and functional: 100–220ms, `--ease-glide`, honoring
  `prefers-reduced-motion`.

**Don't**

- Don't draw **any border or divider for structure or selection** — no
  outlines on controls, no row dividers, no section rules, no left rails.
  The chrome may show exactly two lines: the focus indicator
  (`rule-focus`) and the workspace panels' `edge` frame (§4); charts may
  draw data lines with explicit alpha-ink fills (`bg-ink/15`, `bg-ink/8`).
- Don't use `edge` **inside** a panel — it frames the floating workspace
  panels only; interior hierarchy stays fill-only.
- Don't deviate from the **single 6px radius** — no in-between values, and
  `rounded-full` only on genuinely circular elements.
- Don't add **drop shadows** outside the two theme tokens (`shadow-raised`
  in-flow, `shadow-floating` for overlays) and the transient `.deal-shadow`.
  No heavy glows — `pulse-dot` is the only glow.
- Don't use the accent for **body text, large fills, or decorative
  blocks.** It loses meaning the moment it stops being rare.
- Don't mix proportional and tabular figures. Numbers are always tabular
  monospace.
- Don't use **gradients** decoratively. The two sanctioned exceptions are
  the `.fcall-chrome` accent wash on function-call cards and the
  `.thinking-shimmer` streaming-text mask.
- Don't capitalize a sentence to "fix" a heading — rewrite it instead.
- Don't stack two headings of the same scale. Headlines are always paired
  with an ink-faint continuation, never another headline.
- Don't set technical data (IDs, metrics, span labels) in the sans face —
  and don't drop mono below 11px.
- Don't paint **full-color status backgrounds** — use text color plus a
  small icon, dot, or left stripe on the status's `-muted` tint.
- Don't introduce **viewport breakpoints** for panel layouts — use
  container queries.

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
   `label-caps-lg` heading with section spacing — no separator rule.
2. **Search + index** — the spec's `SearchField` (24px display caret)
   immediately above a `rounded-md bg-surface` container; rows separate by
   hover/selected surfaces (no per-row dividers). Each row is a compact
   row-shaped variant of `WorkerCard` (no command block; just name +
   version + description + meta). Hover uses `bg-surface-hover`; the
   focused/selected row uses the same `bg-surface-selected` tint as the
   focused `WorkerCard` in §10.
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
section eyebrow (label-caps)                    <- bg page
  card    bg-panel-raised head + surface body   <- per function/trigger
    pane    bg-paper-2 head + bg body           <- per schema (request/response)
      tree    flat type-table                   <- per field
```

1. **Sections** — two `Section`s (functions, triggers), each with a
   `label-caps-lg` eyebrow on the left and a small ink-ghost count on the
   right. Same chrome already used by other tabs on the worker page; do not
   wrap the section bodies in an additional border.
2. **Card** — a `rounded-md bg-surface` `article` per function/trigger. The
   `bg-panel-raised` head strip carries the name (`title-cell`) on the left,
   any `metadata.tags` rendered as borderless `bg-surface` label-caps-sm
   pills next to it, and the
   kind label (`FUNCTION` / `TRIGGER`) in label-caps on the right. Function
   and trigger cards share the exact same head; the only difference is
   what the name represents. **Triggers do not carry a separate "type"
   chip** — a trigger card *is* the definition of a trigger type, so the
   trigger's name (e.g. `http-endpoint`, `cron`, `queue`) is the type. Each
   card carries a stable `id` (`fn-<name>` / `trigger-<name>`) plus
   `scroll-mt-20` so that anchor navigation lands cleanly below the sticky
   site header.
3. **Pane** — the two schemas inside a card sit in a `@2xl:grid-cols-2`
   grid separated by grid `gap`. Each pane is a
   `rounded-sm` block with a `bg-paper-2` head strip carrying a
   label-caps-sm eyebrow (`request` / `response` for functions,
   `invocation` / `return` for triggers). The `bg-paper-2` step makes the
   pane read as one tonal level "down" from the card's `bg-panel-raised`
   head.
4. **Tree** — fields render as a flat type-table inside the pane body, one
   row per field. Each row is `name  type  required-marker  enum  constraints`
   on one line, with the description on a second line in
   `text-ink-faint text-[12px]`. Nested objects, arrays-of-objects, and
   union variants indent under the parent by indentation alone (no guide
   line). Nesting deeper than three levels collapses behind a native
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
chrome as the other sidebar cards (`rounded-md bg-surface` block +
`bg-panel-raised` head + `label-caps-lg` title) and contains two
sub-sections — `functions` and
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
function/trigger card lights up with the canonical `bg-surface-selected`
tint (the same recipe used by `WorkerCard` focused state in §10 and the
`versions` Row's `aria-current` style) and the clicked sidebar link itself
switches to `text-accent`. This gives the user a clear "you are here"
signal that survives any subsequent scrolling. The tinted fill is the
sanctioned selection accent (§4 "Selection and severity").

**Why CSS `:target` is not enough.** Next.js `Link` performs same-page
hash navigation through `history.pushState`, which does **not** fire
`hashchange` and which most browsers do **not** treat as a `:target`
re-evaluation trigger. As a result, plain
`target:bg-surface-selected` only highlights on full page
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
  `bg-surface-selected` when the hash matches its `id`. The card
  body (header, description, schema panes) is still a server-rendered
  React tree passed in as `children`, so the only JS that runs in the
  browser is the wrapper.
- **Sidebar links** ([`summary-list.tsx`](app/src/components/api-reference/summary-list.tsx))
  call `useActiveHash()` for their visual state and
  `setActiveHash(target)` from `onClick`, which broadcasts to the
  card subscribers immediately so the tint and the link light up in
  the same frame as the click — even though Next.js will not have
  fired `hashchange` yet.

This is also why the sidebar uses a single `SummaryList` client
component for *both* `functions` and `triggers` sub-sections rather
than one client component per section: with the hub providing a single
source of truth, splitting state would only cause stale highlights when
the user crossed sub-sections.

**Accent rationing.** Per §3, the selection still costs only one accent
moment per visible region: in the sidebar it is the active link; on the
panel it is the tinted fill on the targeted card. The required-marker
accent inside the schema tree continues to coexist because it lives at a
different scale (a single character per row).
