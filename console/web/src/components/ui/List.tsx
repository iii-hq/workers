import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

export const List = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.list, className)} {...props} />
))
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
      ...props
    },
    ref,
  ) => (
    <button
      ref={ref}
      type={type}
      data-selected={selected || undefined}
      aria-pressed={props['aria-pressed'] ?? selected}
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
    </button>
  ),
)
ListItem.displayName = 'ListItem'
