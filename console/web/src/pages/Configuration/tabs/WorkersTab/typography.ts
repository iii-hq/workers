/**
 * Workers tab text scale — one step up from the console baseline so the
 * configuration form reads comfortably at arm's length. Import these
 * instead of hard-coded pixel sizes so the whole tab scales together.
 */
export const wt = {
  /** 12px — ghost ids, required badges, warn hints */
  micro: 'font-mono text-[12px]',
  /** 13px — field labels, captions, helper text */
  caption: 'font-mono text-[13px]',
  /** 14px — status text, descriptions, pre blocks */
  bodySm: 'font-mono text-[14px]',
  /** 15px — list items, section titles, control values */
  body: 'font-mono text-[15px]',
  /** 16px — empty-state titles, emphasis */
  bodyLg: 'font-mono text-[16px]',
  /** 18px — editor header */
  heading: 'font-mono text-[18px]',
  /** Pair with h-9 inputs/selects inside this tab. */
  control: 'font-mono text-[15px]',
  /** Scale segmented toggles (ModeToggle) used in this tab. */
  toggle: '[&_button]:font-mono [&_button]:text-[15px]',
} as const
