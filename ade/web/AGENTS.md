# ADE console: design system rules

After changing anything under `src/`, run `pnpm lint:design` and fix every
error. CI runs the same command (the `ade-web` job), so a red local run is a
red PR.

- **Colours** come from tokens in `src/index.css` (`bg-card`, `text-ink-faint`,
  `border-border`, `bg-scrim`, …). No Tailwind palette (`neutral-*`, `zinc-*`,
  `white`, `black`), no hex, no `bg-[#…]`.
- **Sizes** are on the scale: `text-meta` 11 / `text-xs` 12 / `text-body` 13 /
  `text-sm` 14 / `text-base` 16, spacing in 0.25 steps (`p-1.75`, not
  `p-[7px]`). Theme variables use the shorthand `duration-(--motion-duration-control)`,
  never `duration-[var(--…)]`.
- **Components** come from `src/components/shadcn` and `src/components/ui`.
  Callers pass layout only (width, margin, flex). A new look is a variant or a
  component in `components/ui`, not a `className` on someone else's component.
  To give a Base UI trigger a design-system look, use `render`:
  `<PopoverTrigger render={<ToolbarPill />}>`.
- **No inline styles.** A dynamic value is a CSS variable:
  `style={{ '--thumb-x': '24px' }}` + `translate-x-(--thumb-x)`.

Code written before the gate is counted in `eslint-suppressions.json`. A file
can keep its count but never grow it. After fixing a legacy finding run
`pnpm lint:design:prune`; ESLint fails while a suppression is left over. Never
run `eslint --suppress-all` to turn a build green.

Worker stylesheets (`<worker>/ui/styles.css`) read the same tokens through
`var()`: `var(--color-ink)`, `var(--text-sm)`, `var(--radius-md)`. The public
list is `packages/console-ui/token-names.mjs`.
