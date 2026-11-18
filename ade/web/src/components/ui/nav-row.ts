import { cva, type VariantProps } from 'class-variance-authority'

/**
 * One row recipe for every navigation column (chats, workers, settings):
 * the shadcn sidebar menu button's look — the `sidebar-accent` wash on
 * hover and on the active row, `sidebar-ring` focus — without its
 * provider, because the console's columns live inside `PageSidebar`
 * (resizable, collapsible, a drawer on phones), which the shadcn shell
 * does not do. Mark the current row with `data-active`.
 */
export const navRowVariants = cva(
  'group/row flex w-full min-w-0 items-center gap-2 overflow-hidden rounded-md px-2 text-left text-sm text-muted-foreground outline-none transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 data-active:bg-sidebar-accent data-active:font-medium data-active:text-sidebar-accent-foreground [&_svg]:shrink-0 [&_svg:not([class*=size-])]:size-4',
  {
    variants: {
      size: {
        sm: 'h-7 text-xs',
        default: 'h-8 text-sm',
        /** Rows of two lines, or a touch surface. */
        lg: 'h-12 text-base',
        /** Height from the content (a title with a detail line under it). */
        auto: 'min-h-8 py-1.5',
      },
    },
    defaultVariants: { size: 'default' },
  },
)

export type NavRowVariants = VariantProps<typeof navRowVariants>
