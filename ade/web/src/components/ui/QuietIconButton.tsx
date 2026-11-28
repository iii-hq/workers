import { cva, type VariantProps } from 'class-variance-authority'
import type { ComponentProps } from 'react'
import { cn } from '@/lib/utils'

const quietIconButton = cva(
  'flex shrink-0 items-center justify-center rounded-sm text-ink-faint transition duration-(--motion-duration-control) hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none',
  {
    variants: {
      size: {
        /** 28px: a row's or a header's action on a pointer. */
        default: 'size-7',
        /** 44px: the same action in a narrow pane or on a phone row. */
        touch: 'size-11',
        /** 48px: a phone page's header action. */
        page: 'size-12',
      },
    },
    defaultVariants: { size: 'default' },
  },
)

/**
 * A quiet square icon action for rows and headers: the glyph in the faint ink,
 * lifting to the hover surface, with the focus ring. It is the "+" and "⋯" of
 * a project header and the ⋯ of a side chat. Callers pass layout only; hand it
 * to a Base UI trigger through `render`, e.g.
 * `<DropdownMenuTrigger render={<QuietIconButton aria-label="…" />}>`.
 */
export function QuietIconButton({
  className,
  size,
  type = 'button',
  ...props
}: ComponentProps<'button'> & VariantProps<typeof quietIconButton>) {
  return (
    <button
      type={type}
      data-slot="quiet-icon-button"
      className={cn(quietIconButton({ size }), className)}
      {...props}
    />
  )
}
