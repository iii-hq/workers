/**
 * The redesigned chrome (header, sidebar, chat pane) reads the console's
 * ink and surface tokens through Tailwind's neutral ramp. Put this on a
 * surface's root and every shared recipe inside it — tree rows, chips,
 * cards, the composer's colour-mixes — resolves to neutral without any
 * component knowing. One string, so the two themes cannot drift apart.
 */
export const NEUTRAL_SURFACE =
  '[--color-ink:var(--color-neutral-900)] [--color-ink-faint:var(--color-neutral-500)] [--color-ink-ghost:var(--color-neutral-400)] [--color-panel:var(--color-white)] [--color-panel-raised:var(--color-neutral-50)] [--color-surface:var(--color-neutral-100)] [--color-surface-hover:var(--color-neutral-200)] [--color-surface-selected:var(--color-neutral-200)] dark:[--color-ink:var(--color-neutral-50)] dark:[--color-ink-faint:var(--color-neutral-400)] dark:[--color-ink-ghost:var(--color-neutral-500)] dark:[--color-panel:var(--color-neutral-950)] dark:[--color-panel-raised:var(--color-neutral-900)] dark:[--color-surface:var(--color-neutral-900)] dark:[--color-surface-hover:var(--color-neutral-800)] dark:[--color-surface-selected:var(--color-neutral-800)]'
