import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

const LIST_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End']

function nextListItem(
  items: HTMLElement[],
  index: number,
  key: string,
): HTMLElement | undefined {
  if (key === 'Home') return items[0]
  if (key === 'End') return items[items.length - 1]
  return items[index + (key === 'ArrowDown' ? 1 : -1)]
}

/** The arrows walk the list's items; Home and End jump to the ends. */
function onListKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
  if (!LIST_KEYS.includes(event.key)) return
  const target = event.target as HTMLElement
  if (!target.matches('[data-list-item]')) return
  const items = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      '[data-list-item]:not([disabled])',
    ),
  )
  const index = items.indexOf(target)
  if (index === -1) return
  const next = nextListItem(items, index, event.key)
  if (!next) return
  event.preventDefault()
  next.focus()
}

/**
 * One Tab stop per list (a roving tabindex): the row last focused, else the
 * selected one, else the first, is the list's stop; every other row is
 * reached with the arrows. A list of fifteen scopes was fifteen Tab presses
 * to leave. Rows that set their own `tabIndex` are the caller's to manage
 * and are left alone, as are rows of a list nested inside this one.
 */
function applyRovingStop(
  root: HTMLElement | null,
  current: HTMLElement | null,
): void {
  if (!root) return
  const items = Array.from(
    root.querySelectorAll<HTMLElement>('[data-list-roving]'),
  ).filter(
    (item) =>
      item.closest('[data-list-root]') === root &&
      !item.matches(':disabled, [aria-disabled="true"]'),
  )
  if (items.length === 0) return
  const stop =
    (current && items.includes(current) ? current : undefined) ??
    items.find((item) => item.hasAttribute('data-selected')) ??
    items[0]
  for (const item of items) item.tabIndex = item === stop ? 0 : -1
}

export const List = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, onKeyDown, onFocus, ...props }, ref) => {
  const root = React.useRef<HTMLDivElement | null>(null)
  const focused = React.useRef<HTMLElement | null>(null)
  const setRef = React.useCallback(
    (element: HTMLDivElement | null) => {
      root.current = element
      if (typeof ref === 'function') ref(element)
      else if (ref) ref.current = element
    },
    [ref],
  )
  // Rows come and go with every render (a filter, a live update): the stop
  // is re-chosen each time, so the list never ends up with none or many.
  React.useLayoutEffect(() => {
    applyRovingStop(root.current, focused.current)
  })
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the handler only moves focus between the list's own buttons
    <div
      ref={setRef}
      data-list-root=""
      className={cn(uiClasses.list, className)}
      onKeyDown={(event) => {
        onKeyDown?.(event)
        if (!event.defaultPrevented) onListKeyDown(event)
      }}
      onFocus={(event) => {
        onFocus?.(event)
        const target = event.target as HTMLElement
        if (!target.matches('[data-list-roving]')) return
        if (target.closest('[data-list-root]') !== root.current) return
        focused.current = target
        applyRovingStop(root.current, target)
      }}
      {...props}
    />
  )
})
List.displayName = 'List'

export const ListGroup = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.listGroup, className)} {...props} />
))
ListGroup.displayName = 'ListGroup'

export const ListGroupLabel = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(uiClasses.listGroupLabel, className)}
    {...props}
  />
))
ListGroupLabel.displayName = 'ListGroupLabel'

export interface ListItemProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  selected?: boolean
  leading?: React.ReactNode
  label?: React.ReactNode
  description?: React.ReactNode
  trailing?: React.ReactNode
  /**
   * `div` when the row holds its own buttons (a download's cancel, a
   * row's menu) — a button cannot contain buttons. Selection is then
   * `aria-selected`; the caller owns `role`/`tabIndex`/keys.
   */
  as?: 'button' | 'div'
}

/**
 * The shared selectable row. Selection is deliberately neutral: selected
 * items use the surface/edge/ink ramp, never the theme accent.
 */
export const ListItem = React.forwardRef<HTMLButtonElement, ListItemProps>(
  (
    {
      className,
      selected,
      leading,
      label,
      description,
      trailing,
      children,
      type = 'button',
      as = 'button',
      ...props
    },
    ref,
  ) => (
    <ListItemTag
      tag={as}
      ref={ref as unknown as React.Ref<HTMLElement>}
      type={as === 'button' ? type : undefined}
      data-list-item=""
      // The list's roving Tab stop moves this row's tabindex, unless the
      // caller set one or the row is a `div` (then the caller manages it).
      data-list-roving={
        as === 'button' && props.tabIndex === undefined ? '' : undefined
      }
      data-selected={selected || undefined}
      aria-pressed={
        as === 'button' ? (props['aria-pressed'] ?? selected) : undefined
      }
      aria-selected={
        as === 'div' ? (props['aria-selected'] ?? selected) : undefined
      }
      className={cn(uiClasses.listItem, className)}
      {...props}
    >
      {leading ? (
        <span className={uiClasses.listItemIcon}>{leading}</span>
      ) : null}
      {label !== undefined || description !== undefined ? (
        <span className={uiClasses.listItemContent}>
          {label !== undefined ? (
            <span className={uiClasses.listItemTitle}>{label}</span>
          ) : null}
          {description !== undefined ? (
            <span className={uiClasses.listItemDescription}>{description}</span>
          ) : null}
        </span>
      ) : (
        children
      )}
      {trailing ? (
        <span className={uiClasses.listItemMeta}>{trailing}</span>
      ) : null}
    </ListItemTag>
  ),
)
ListItem.displayName = 'ListItem'

const ListItemTag = React.forwardRef<
  HTMLElement,
  // biome-ignore lint/suspicious/noExplicitAny: button or div attributes
  { tag: 'button' | 'div' } & Record<string, any>
>(({ tag, ...props }, ref) =>
  tag === 'div' ? (
    <div ref={ref as React.Ref<HTMLDivElement>} {...props} />
  ) : (
    <button ref={ref as React.Ref<HTMLButtonElement>} {...props} />
  ),
)
ListItemTag.displayName = 'ListItemTag'
