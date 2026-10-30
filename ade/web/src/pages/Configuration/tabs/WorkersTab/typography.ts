/**
 * Global Settings text scale: Inter, never below 14px and never above 18px.
 * Hierarchy comes from weight and the neutral ramp, not from shrinking type.
 * Import these instead of hard-coded pixel sizes so the whole surface scales
 * together.
 */
export const wt = {
  /** 14px — ghost ids, required badges, warn hints */
  micro: 'font-inter text-sm leading-5',
  /** 14px — field labels, captions, helper text */
  caption: 'font-inter text-sm leading-5',
  /** 14px — status text, descriptions, pre blocks */
  bodySm: 'font-inter text-sm leading-5',
  /** 14px — list items, control values */
  body: 'font-inter text-sm leading-5',
  /** 16px — section titles, empty-state titles, emphasis */
  bodyLg: 'font-inter text-base leading-6',
  /** 18px — editor header */
  heading: 'font-inter text-lg leading-6',
  /** Pair with h-9 inputs/selects inside this surface. */
  control: 'font-inter text-sm',
  /** Scale segmented toggles (ModeToggle) used in this surface. */
  toggle: '[&_button]:font-inter [&_button]:text-sm',
} as const
