/**
 * The chrome (header, sidebar, chat pane, the shadcn sheets) once remapped
 * the console's ink and surface tokens onto Tailwind's raw neutral ramp
 * here. Every token now aliases the design-system palette in
 * `src/index.css` (`--background`, `--card`, `--muted`, …), so the chrome
 * reads it like everything else and there is nothing left to remap. The
 * name stays so a surface root can still carry it; it applies no classes.
 */
export const NEUTRAL_SURFACE = ''

/**
 * The inverse pill (a tooltip: ink behind, page colour on top) captures its
 * two colours under its own names, so the remap below can refer to them
 * without reading tokens it is about to change.
 */
export const INVERSE_SURFACE_VARS =
  '[--inverse-bg:var(--foreground)] [--inverse-fg:var(--background)]'

/**
 * Inside an inverse pill every ink token points the other way: `ink` is
 * the page colour, `ink-faint`/`ink-ghost` are that colour thinned, the
 * surfaces and `edge` are thin washes of it, and the panel tokens are the
 * pill itself. The shadcn aliases (`foreground`, `muted-foreground`,
 * `popover-foreground`) follow. Put it on a `display: contents` wrapper
 * around the pill's children — never on the pill, whose own `bg-foreground`
 * would otherwise turn into the page colour — and content written for the
 * page (`text-ink`, a worker's `color: var(--color-ink-faint)`, a `Kbd` on
 * `bg-surface`) reads on the pill in both themes instead of vanishing
 * white-on-white in the dark one.
 */
export const INVERSE_INK =
  '[--color-ink:var(--inverse-fg)] [--color-ink-faint:color-mix(in_oklab,var(--inverse-fg)_72%,transparent)] [--color-ink-ghost:color-mix(in_oklab,var(--inverse-fg)_56%,transparent)] [--color-ink-disabled:color-mix(in_oklab,var(--inverse-fg)_40%,transparent)] [--color-muted-foreground:color-mix(in_oklab,var(--inverse-fg)_72%,transparent)] [--color-edge:color-mix(in_oklab,var(--inverse-fg)_22%,transparent)] [--color-surface:color-mix(in_oklab,var(--inverse-fg)_14%,transparent)] [--color-surface-hover:color-mix(in_oklab,var(--inverse-fg)_20%,transparent)] [--color-surface-selected:color-mix(in_oklab,var(--inverse-fg)_26%,transparent)] [--color-panel:var(--inverse-bg)] [--color-panel-raised:var(--inverse-bg)] [--foreground:var(--inverse-fg)] [--muted-foreground:color-mix(in_oklab,var(--inverse-fg)_72%,transparent)] [--popover-foreground:var(--inverse-fg)] [--card-foreground:var(--inverse-fg)]'
