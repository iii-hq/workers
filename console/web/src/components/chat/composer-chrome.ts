import { cn } from '@/lib/utils'

/**
 * The composer's visual vocabulary, shared with the strips that stack above
 * it (SessionTriggers) so the whole footer reads as one instrument: the same
 * card material, the same quiet round icon buttons, the same phone/desktop
 * sizing. Anything that sits in the footer should draw from here rather than
 * restate the classes.
 */

/** The card: a rounded, lifted instrument surface (see `--shadow-lift`). */
export const composerCardClass =
  'overflow-hidden rounded-xl bg-panel-raised shadow-lift'

/**
 * Quiet round icon button for a toolbar's secondary actions (attach, clear,
 * dismiss). Phone-sized for touch, tightened at `sm` where a pointer aims.
 */
export const toolbarIconButtonClass = cn(
  'inline-flex size-12 shrink-0 items-center justify-center rounded-full text-ink-faint',
  'hover:bg-surface-hover hover:text-ink sm:size-8',
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
  'disabled:pointer-events-none disabled:opacity-40',
)
