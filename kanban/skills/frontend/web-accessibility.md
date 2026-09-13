---
title: web-accessibility
description: Build accessible interfaces to WCAG 2.2 AA — semantics first, keyboard and focus management, ARIA only when needed, forms and live regions, and how to verify with automated plus manual passes.
type: how-to
---

# Web accessibility

Target **WCAG 2.2 level AA**. Anything less is a bug filed against your users, and it is
cheapest to build in.

Standard: <https://www.w3.org/WAI/WCAG22/quickref/> · patterns:
<https://www.w3.org/WAI/ARIA/apg/patterns/> (APG). Read the APG pattern before hand-rolling any
composite widget — dialog, tabs, menu, combobox, listbox, tree, grid.

## Rule zero: use the element that already does it

A native element arrives with role, keyboard behavior, focus, and screen-reader
announcements. ARIA only *describes*; it never implements.

| Need | Use | Not |
| --- | --- | --- |
| do something | `<button type="button">` | `<div onClick>`, `<a>` with no `href` |
| go somewhere | `<a href>` | `<div role="link">` |
| labeled input | `<label for>` + `<input id>` | placeholder-as-label |
| group of choices | `<fieldset><legend>` | a styled `<div>` with a heading |
| navigation | `<nav>`, `<main>`, `<header>`, `<footer>`, `<aside>` | nested `<div>`s with class names |
| headings | `<h1>`–`<h6>` in order | `<div class="title">` |

The first ARIA rule: **if a native element or attribute can do it, do not use ARIA.** A wrong
`role` is worse than no `role` — it overwrites correct implicit semantics.

## Keyboard: the whole interface, no mouse

Test every change by unplugging the mouse mentally and walking the flow with Tab / Shift+Tab /
Enter / Space / Escape / arrows.

- Everything interactive is reachable and operable by keyboard, in a **logical order that
  follows the visual order**. `tabindex` values greater than 0 are almost always wrong.
- **Focus is always visible.** Never `outline: none` without an equally visible replacement.
  Style `:focus-visible` so mouse users do not see rings and keyboard users always do.
- Escape closes the topmost overlay; focus returns to **the element that opened it**.
- Modals trap focus **and** make the rest of the page inert — `<dialog>` with `showModal()`,
  or the `inert` attribute. Closing must restore focus deliberately; the browser will not.
- Composite widgets follow the APG's key contract: roving tabindex or `aria-activedescendant`
  for tabs and menus, arrow keys to move, Home/End to jump, typeahead in listboxes.
- A skip-to-content link as the first focusable element on full-page layouts.
- Do not hijack scroll or single-key shortcuts without an off switch (WCAG 2.1.4).
- **Route changes need focus management.** On client-side navigation move focus to the new
  `<h1>` (or a container with `tabIndex={-1}`) and update `document.title`, or a screen-reader
  user hears nothing change.

## Forms

- Every control has a programmatic label. Placeholder text is a hint, never a label.
- Group related controls with `<fieldset>`/`<legend>`; a group of radios needs both.
- **Errors:** `aria-invalid` on the field, an error message associated with `aria-describedby`,
  and an error summary at the top of a long form linking to each offending field. Validate on
  blur or submit, not on every keystroke.
- Required fields: `required` plus a visible marker, not color alone.
- Help the input method: `type="email"`, `inputMode="numeric"`, `autoComplete="one-time-code"`,
  and generous touch targets.
- Never disable the submit button as the only feedback — show what is missing.
- Autofill and paste must work; do not block them.

## Images, color, motion

- **`alt` is required on every `<img>`.** Informative: describe the *content's purpose*. Text in
  an image: put the text in `alt`. Decorative: `alt=""` (an empty string, not a missing
  attribute). A complex chart needs a text alternative, not an `alt` paragraph.
- SVG: `role="img"` + `<title>` when meaningful, `aria-hidden="true"` when decorative. An
  icon-only button needs an accessible name: `aria-label` on the button, not on the `<svg>`.
- Contrast: **4.5:1** for body text, **3:1** for large text and for UI-component boundaries
  and focus indicators. Placeholder text and disabled text are the usual failures.
- **Never encode meaning in color alone** (status, validation, chart series) — pair it with
  text, an icon, or a pattern.
- Respect `prefers-reduced-motion: reduce`: disable parallax, autoplay, and large movement.
- WCAG 2.2 additions: focus must not be entirely hidden by sticky headers (2.4.11), and
  dragging interactions need a single-pointer alternative (2.5.7).

## Live regions and async UI

- Announce results that appear without a focus change: `role="status"`
  (`aria-live="polite"`) for counts, saves, and toasts; `role="alert"` (`assertive`) only for
  errors that need interruption.
- The live region container must exist **in the DOM before** you write into it. Inserting the
  container and the text in the same commit announces nothing.
- Loading states need an accessible name (`aria-busy`, a visually-hidden "Loading posts"), not
  just a spinner.
- Do not steal focus to announce.

## Verifying — automated tools catch about a third

1. **Automated:** `eslint-plugin-jsx-a11y` in CI, plus axe (`@axe-core/react` in dev,
   `vitest-axe`/`jest-axe` in tests, or the browser extension). Fix everything it reports, then
   remember how much it cannot see.
2. **Keyboard pass** on the real page: complete the primary flow with no mouse. Note the focus
   ring at every step.
3. **Zoom and reflow:** 200% browser zoom, and a 320 px-wide viewport, with no loss of content
   or function and no horizontal scrolling (1.4.10 / 1.4.4).
4. **Screen reader smoke test:** VoiceOver (⌘F5) or NVDA. Check the page title, the landmark
   list, that headings outline the page, that each control announces a name/role/state, and
   that dynamic updates are announced.
5. **Testing Library asserts by role and name** — `getByRole('button', { name: 'Save' })` fails
   when the accessible name is missing, which is exactly the regression you want caught.

## The short list to refuse

- `div` with `onClick` instead of a button.
- `outline: none` with no focus style.
- Placeholder as a label.
- `aria-label` on a non-interactive element to "give it a name".
- A modal that does not trap focus, or that drops focus on close.
- A route change that moves nothing and announces nothing.
- `title` attributes as the only explanation.
- Autoplay media, or animation that ignores reduced-motion.
- Accessibility "checked later": retrofitting semantics costs multiples of building it in.
