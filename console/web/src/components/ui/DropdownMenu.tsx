import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import { Check } from 'lucide-react'
import type * as React from 'react'
import { cn } from '@/lib/utils'

/**
 * Radix dropdown-menu adapted to the console design system — the shadcn
 * dropdown-menu anatomy (Root / Trigger / Content / Group / Label /
 * CheckboxItem / Separator) restyled with the house tokens (font-mono,
 * lowercase, ink/bg/rule), mirroring how `Select.tsx` adapts radix-select.
 *
 * Behavior (open on click, typeahead, arrow keys, Escape) is Radix's;
 * consumers that want hover-expansion control `open` themselves (see
 * `SpanFilterMenu`).
 */

export const DropdownMenu = DropdownMenuPrimitive.Root
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger
export const DropdownMenuGroup = DropdownMenuPrimitive.Group

export function DropdownMenuContent({
  className,
  sideOffset = 4,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Content>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        sideOffset={sideOffset}
        className={cn(
          'z-50 min-w-[10rem] overflow-hidden rounded-md bg-panel-raised p-1 text-ink font-mono text-[12px] lowercase shadow-floating',
          'data-[state=open]:animate-in data-[state=closed]:animate-out',
          'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
          className,
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  )
}

export function DropdownMenuLabel({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Label>) {
  return (
    <DropdownMenuPrimitive.Label
      className={cn(
        'px-2 pt-1.5 pb-1 text-[10px] uppercase tracking-[0.12em] text-ink-faint',
        className,
      )}
      {...props}
    />
  )
}

export function DropdownMenuItem({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Item>) {
  return (
    <DropdownMenuPrimitive.Item
      className={cn(
        'relative flex cursor-pointer items-center gap-2 rounded-xs px-2 py-1.5 outline-none select-none',
        'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-40',
        className,
      )}
      {...props}
    />
  )
}

export function DropdownMenuCheckboxItem({
  className,
  children,
  indicator,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.CheckboxItem> & {
  /** Checked-state icon (a Lucide icon, per the design system); defaults to a checkmark. */
  indicator?: React.ReactNode
}) {
  return (
    <DropdownMenuPrimitive.CheckboxItem
      className={cn(
        'relative flex cursor-pointer items-center rounded-xs py-1.5 pr-2 pl-7 outline-none select-none',
        'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
        'data-[state=checked]:text-ink',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-40',
        className,
      )}
      {...props}
    >
      <DropdownMenuPrimitive.ItemIndicator className="absolute top-1/2 left-2 -translate-y-1/2 text-ink">
        {indicator ?? (
          <Check aria-hidden className="h-3 w-3" strokeWidth={2.5} />
        )}
      </DropdownMenuPrimitive.ItemIndicator>
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  )
}

export function DropdownMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Separator>) {
  return (
    <DropdownMenuPrimitive.Separator
      className={cn('-mx-1 my-1 h-px bg-edge', className)}
      {...props}
    />
  )
}
