/**
 * The console's first dropdown-menu API (Radix-shaped: `asChild` on the
 * trigger, `onSelect` on items, an `indicator` / `box` on checkbox rows),
 * kept as a thin layer over the shadcn (Base UI) dropdown menu so worker
 * UIs that import these parts from `@iii-dev/console-ui` get the same
 * sheet, rows and motion as the console. New console code reaches for
 * `components/shadcn/dropdown-menu` directly.
 *
 * Behavior (open on click, typeahead, arrow keys, Escape) is Base UI's. A
 * row that must keep the menu open (a multi-select toggle) passes
 * `closeOnClick={false}`; the Radix `onSelect={(e) => e.preventDefault()}`
 * idiom has no effect here.
 */

import { Check } from 'lucide-react'
import {
  type ComponentProps,
  isValidElement,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
  type SyntheticEvent,
} from 'react'
import {
  DropdownMenu as BaseDropdownMenu,
  DropdownMenuCheckboxItem as BaseDropdownMenuCheckboxItem,
  DropdownMenuContent as BaseDropdownMenuContent,
  DropdownMenuGroup as BaseDropdownMenuGroup,
  DropdownMenuItem as BaseDropdownMenuItem,
  DropdownMenuLabel as BaseDropdownMenuLabel,
  DropdownMenuRadioGroup as BaseDropdownMenuRadioGroup,
  DropdownMenuRadioItem as BaseDropdownMenuRadioItem,
  DropdownMenuSeparator as BaseDropdownMenuSeparator,
  DropdownMenuSub as BaseDropdownMenuSub,
  DropdownMenuSubContent as BaseDropdownMenuSubContent,
  DropdownMenuSubTrigger as BaseDropdownMenuSubTrigger,
  DropdownMenuTrigger as BaseDropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { cn } from '@/lib/utils'

export const DropdownMenu = BaseDropdownMenu
export const DropdownMenuGroup = BaseDropdownMenuGroup
export const DropdownMenuSub = BaseDropdownMenuSub
export const DropdownMenuRadioGroup = BaseDropdownMenuRadioGroup
export const DropdownMenuSeparator = BaseDropdownMenuSeparator
export const DropdownMenuSubTrigger = BaseDropdownMenuSubTrigger

export interface DropdownMenuTriggerProps
  extends ComponentProps<typeof BaseDropdownMenuTrigger> {
  /** Radix: the single child element is the trigger (Base UI `render`). */
  asChild?: boolean
  children?: ReactNode
}

export function DropdownMenuTrigger({
  asChild,
  children,
  ...props
}: DropdownMenuTriggerProps) {
  if (asChild && isValidElement(children)) {
    return (
      <BaseDropdownMenuTrigger
        {...props}
        render={children as ReactElement<Record<string, unknown>>}
      />
    )
  }
  return (
    <BaseDropdownMenuTrigger {...props}>{children}</BaseDropdownMenuTrigger>
  )
}

export interface DropdownMenuContentProps
  extends ComponentProps<typeof BaseDropdownMenuContent> {
  /** Radix name; Base UI keeps the sheet inside the viewport on its own. */
  collisionPadding?: number
}

/** The sheet: `min-w-[10rem]`, as the first API sized it. */
export function DropdownMenuContent({
  className,
  collisionPadding: _collisionPadding,
  ...props
}: DropdownMenuContentProps) {
  return (
    <BaseDropdownMenuContent
      className={cn('min-w-[10rem] text-sm', className)}
      {...props}
    />
  )
}

export function DropdownMenuLabel({
  className,
  ...props
}: ComponentProps<typeof BaseDropdownMenuLabel>) {
  return <BaseDropdownMenuLabel className={cn('px-2', className)} {...props} />
}

type SelectHandler = (event: SyntheticEvent<HTMLDivElement>) => void

/** Radix `onSelect` becomes Base UI's `onClick`; both may be given. */
function withSelect<P extends { onClick?: unknown }>(
  onSelect: SelectHandler | undefined,
  props: P,
): P {
  if (!onSelect) return props
  const onClick = props.onClick as SelectHandler | undefined
  return {
    ...props,
    onClick: (event: MouseEvent<HTMLDivElement>) => {
      onClick?.(event)
      onSelect(event)
    },
  }
}

export interface DropdownMenuItemProps
  extends Omit<ComponentProps<typeof BaseDropdownMenuItem>, 'onSelect'> {
  /** Radix name for the row's activation; Base UI's `onClick`. */
  onSelect?: SelectHandler
}

export function DropdownMenuItem({
  className,
  onSelect,
  ...props
}: DropdownMenuItemProps) {
  return (
    <BaseDropdownMenuItem
      className={cn('cursor-pointer gap-2 px-2', className)}
      {...withSelect(onSelect, props)}
    />
  )
}

export interface DropdownMenuCheckboxItemProps
  extends Omit<
    ComponentProps<typeof BaseDropdownMenuCheckboxItem>,
    'onSelect'
  > {
  onSelect?: SelectHandler
  /** Checked-state icon (a Lucide icon, per the design system); defaults to a checkmark. */
  indicator?: ReactNode
  /**
   * Draw the box too, empty while off (the shared Checkbox look): for a
   * setting that stays on, where a lone check mark leaves "off" looking
   * like a plain action.
   */
  box?: boolean
}

/** A toggle row: the mark sits at the end of the row (shadcn), or in a box
 in the left gutter with `box`. */
export function DropdownMenuCheckboxItem({
  className,
  children,
  onSelect,
  indicator,
  box = false,
  ...props
}: DropdownMenuCheckboxItemProps) {
  const mark = indicator ?? (
    <Check aria-hidden className="size-4" strokeWidth={2.5} />
  )
  return (
    <BaseDropdownMenuCheckboxItem
      className={cn(
        'cursor-pointer',
        box &&
          'group/check pl-8 [&>[data-slot=dropdown-menu-checkbox-item-indicator]]:hidden',
        className,
      )}
      {...withSelect(onSelect, props)}
    >
      {box ? (
        <span
          aria-hidden
          data-menu-checkbox=""
          className="absolute top-1/2 left-2 flex size-[18px] -translate-y-1/2 items-center justify-center rounded-sm border border-input bg-muted text-primary-foreground group-data-checked/check:border-primary group-data-checked/check:bg-primary [&>svg]:hidden group-data-checked/check:[&>svg]:block"
        >
          {mark}
        </span>
      ) : null}
      {children}
    </BaseDropdownMenuCheckboxItem>
  )
}

export interface DropdownMenuRadioItemProps
  extends Omit<ComponentProps<typeof BaseDropdownMenuRadioItem>, 'onSelect'> {
  onSelect?: SelectHandler
}

export function DropdownMenuRadioItem({
  className,
  onSelect,
  ...props
}: DropdownMenuRadioItemProps) {
  return (
    <BaseDropdownMenuRadioItem
      className={cn('cursor-pointer', className)}
      {...withSelect(onSelect, props)}
    />
  )
}

export function DropdownMenuSubContent({
  className,
  ...props
}: ComponentProps<typeof BaseDropdownMenuSubContent>) {
  return (
    <BaseDropdownMenuSubContent
      className={cn('min-w-[10rem] text-sm', className)}
      {...props}
    />
  )
}
