import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

/**
 * The eyebrow recipe as utilities, for markup that cannot be an `Eyebrow`
 * (a button, a `summary`, a grid head) but needs a colour or hover override:
 * `.iii-ui-eyebrow` in ui-recipes.css is unlayered and would win over any
 * Tailwind utility placed beside it. Keep the three in step.
 */
export const eyebrowClassName =
  'font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint'

/** The wide-tracked section eyebrow (`label-caps-md`): same size, 0.14em. */
export const eyebrowLgClassName =
  'font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-ink-faint'

export interface EyebrowProps extends HTMLAttributes<HTMLElement> {
  as?: 'span' | 'div' | 'p' | 'h2' | 'h3' | 'h4' | 'header' | 'dt' | 'legend'
  /** `lg` is the section eyebrow: the same 11px, tracked 0.14em. */
  size?: 'md' | 'lg'
}

/**
 * The mono caps label: section eyebrows, pane labels, key/value keys.
 * Workers without Tailwind get the same look as `uiClasses.eyebrow`
 * (`.iii-ui-eyebrow` in ui-recipes.css) — keep the two in step. This is
 * the only sanctioned `uppercase` in the console: never transform authored
 * copy or machine identifiers elsewhere.
 */
export function Eyebrow({
  as: Tag = 'span',
  size = 'md',
  className,
  ...props
}: EyebrowProps) {
  return (
    <Tag
      className={cn(
        size === 'lg' ? eyebrowLgClassName : eyebrowClassName,
        className,
      )}
      {...props}
    />
  )
}
