import type { ComponentProps } from 'react'
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioItem,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/shadcn/dropdown-menu'
import { cn } from '@/lib/utils'

/*
 * The chat's menus (a conversation row, a project, a reply): rows on the
 * console's popover canon, Inter 14px and 36px tall, an icon then its label,
 * with a generous inset so the list breathes. The shadcn dropdown parts with
 * that look; callers pass layout only (a min width).
 */
const ROW = 'gap-2.5 rounded-lg px-2.5 py-2 text-sm'
const PANEL = 'p-1.5'

export function ChatMenuContent({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuContent>) {
  return <DropdownMenuContent className={cn(PANEL, className)} {...props} />
}

export function ChatMenuSubContent({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuSubContent>) {
  return <DropdownMenuSubContent className={cn(PANEL, className)} {...props} />
}

export function ChatMenuItem({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuItem>) {
  return <DropdownMenuItem className={cn(ROW, className)} {...props} />
}

export function ChatMenuRadioItem({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuRadioItem>) {
  return <DropdownMenuRadioItem className={cn(ROW, className)} {...props} />
}

export function ChatMenuSubTrigger({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuSubTrigger>) {
  return <DropdownMenuSubTrigger className={cn(ROW, className)} {...props} />
}
