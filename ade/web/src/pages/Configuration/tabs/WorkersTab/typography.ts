/**
 * Global Settings text scale — one step up from the console baseline so a
 * worker configuration form reads comfortably at arm's length. Import these
 * instead of hard-coded pixel sizes so the whole surface scales together.
 */
export const wt = {
  /** 12px — ghost ids, required badges, warn hints */
  micro: 'font-sans text-[12px]',
  /** 13px — field labels, captions, helper text */
  caption: 'font-sans text-[13px]',
  /** 14px — status text, descriptions, pre blocks */
  bodySm: 'font-sans text-[14px]',
  /** 15px — list items, section titles, control values */
  body: 'font-sans text-[15px]',
  /** 16px — empty-state titles, emphasis */
  bodyLg: 'font-sans text-[16px]',
  /** 18px — editor header */
  heading: 'font-sans text-[18px]',
  /** Pair with h-9 inputs/selects inside this surface. */
  control: 'font-sans text-[15px]',
  /** Scale segmented toggles (ModeToggle) used in this surface. */
  toggle: '[&_button]:font-sans [&_button]:text-[15px]',
} as const
