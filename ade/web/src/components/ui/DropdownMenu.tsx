import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import { Check, ChevronRight } from 'lucide-react'
import type * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

/**
 * Radix dropdown-menu adapted to the console design system — the shadcn
 * dropdown-menu anatomy (Root / Trigger / Content / Group / Label /
 * CheckboxItem / Separator, plus Sub / RadioGroup below) restyled with the
 * house tokens (sans interface
 * text and ink/bg/rule), mirroring how `Select.tsx` adapts radix-select.
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
      <PortalScope>
        <DropdownMenuPrimitive.Content
          sideOffset={sideOffset}
          collisionPadding={8}
          className={cn(
            'iii-ui-motion-dropdown z-50 min-w-[10rem] overflow-hidden rounded-md bg-panel-raised p-1 font-sans text-[12px] text-ink shadow-floating',
            className,
          )}
          {...props}
        />
      </PortalScope>
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
        'px-2 pt-1.5 pb-1 text-[12px] font-semibold text-ink-faint',
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
          <Check aria-hidden className="size-4" strokeWidth={2.5} />
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

/*
 * Nested menus and single-choice groups — the rest of the shadcn anatomy
 * (Sub / SubTrigger / SubContent / RadioGroup / RadioItem), styled like the
 * parts above so a row that pushes into a submenu reads as one more item.
 */

export const DropdownMenuSub = DropdownMenuPrimitive.Sub
export const DropdownMenuRadioGroup = DropdownMenuPrimitive.RadioGroup

export function DropdownMenuSubTrigger({
  className,
  children,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.SubTrigger>) {
  return (
    <DropdownMenuPrimitive.SubTrigger
      className={cn(
        'relative flex cursor-pointer items-center rounded-xs px-2 py-1.5 outline-none select-none',
        'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
        'data-[state=open]:bg-surface-hover data-[state=open]:text-ink',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-40',
        className,
      )}
      {...props}
    >
      {children}
      {/* The tree row's disclosure caret, verbatim: a 16px glyph centred in a
          20px control box, hugging the text it discloses at 2px. No flex gap
          here — children space themselves, the caret always sits tight. */}
      <ChevronRight
        aria-hidden
        className="ml-0.5 size-5 shrink-0 p-0.5 text-ink-ghost"
      />
    </DropdownMenuPrimitive.SubTrigger>
  )
}

export function DropdownMenuSubContent({
  className,
  sideOffset = 2,
  alignOffset = -4,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.SubContent>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <PortalScope>
        <DropdownMenuPrimitive.SubContent
          sideOffset={sideOffset}
          /* -4 lines the first item up with the row that opened it: the
             content's own padding would otherwise push it one step down. */
          alignOffset={alignOffset}
          collisionPadding={8}
          className={cn(
            'iii-ui-motion-dropdown z-50 min-w-[10rem] overflow-hidden rounded-md bg-panel-raised p-1 font-sans text-[12px] text-ink shadow-floating',
            className,
          )}
          {...props}
        />
      </PortalScope>
    </DropdownMenuPrimitive.Portal>
  )
}

export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.RadioItem>) {
  return (
    <DropdownMenuPrimitive.RadioItem
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
        <Check aria-hidden className="size-4" strokeWidth={2.5} />
      </DropdownMenuPrimitive.ItemIndicator>
      {children}
    </DropdownMenuPrimitive.RadioItem>
  )
}
