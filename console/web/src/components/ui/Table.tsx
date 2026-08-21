import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

export type TableDensity = 'comfortable' | 'compact'

export const TableViewport = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(uiClasses.tableViewport, className)}
    {...props}
  />
))
TableViewport.displayName = 'TableViewport'

export const TableFrame = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.tableFrame, className)} {...props} />
))
TableFrame.displayName = 'TableFrame'

export interface TableProps
  extends React.TableHTMLAttributes<HTMLTableElement> {
  density?: TableDensity
}

export const Table = React.forwardRef<HTMLTableElement, TableProps>(
  ({ className, density = 'comfortable', ...props }, ref) => (
    <table
      ref={ref}
      data-density={density}
      className={cn(uiClasses.table, className)}
      {...props}
    />
  ),
)
Table.displayName = 'Table'

export const TableHeader = React.forwardRef<
  HTMLTableSectionElement,
  React.HTMLAttributes<HTMLTableSectionElement>
>(({ className, ...props }, ref) => (
  <thead
    ref={ref}
    className={cn(uiClasses.tableHeader, className)}
    {...props}
  />
))
TableHeader.displayName = 'TableHeader'

export const TableBody = React.forwardRef<
  HTMLTableSectionElement,
  React.HTMLAttributes<HTMLTableSectionElement>
>(({ className, ...props }, ref) => (
  <tbody ref={ref} className={cn(uiClasses.tableBody, className)} {...props} />
))
TableBody.displayName = 'TableBody'

export const TableFooter = React.forwardRef<
  HTMLTableSectionElement,
  React.HTMLAttributes<HTMLTableSectionElement>
>(({ className, ...props }, ref) => (
  <tfoot
    ref={ref}
    className={cn(uiClasses.tableFooter, className)}
    {...props}
  />
))
TableFooter.displayName = 'TableFooter'

export interface TableRowProps
  extends React.HTMLAttributes<HTMLTableRowElement> {
  interactive?: boolean
  selected?: boolean
}

/** Arrow keys walk interactive rows; Enter and Space activate the row. */
function onInteractiveRowKeyDown(
  event: React.KeyboardEvent<HTMLTableRowElement>,
): void {
  if (event.target !== event.currentTarget) return
  const row = event.currentTarget
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    row.click()
    return
  }
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  const rows = Array.from(
    row
      .closest('table')
      ?.querySelectorAll<HTMLTableRowElement>('tr[data-interactive]') ?? [],
  )
  const index = rows.indexOf(row)
  if (index === -1) return
  const next = rows[index + (event.key === 'ArrowDown' ? 1 : -1)]
  if (!next) return
  event.preventDefault()
  next.focus()
}

export const TableRow = React.forwardRef<HTMLTableRowElement, TableRowProps>(
  ({ className, interactive, selected, onKeyDown, ...props }, ref) => (
    <tr
      ref={ref}
      data-interactive={interactive || undefined}
      data-selected={selected || undefined}
      // A row that answers a click answers the keyboard too: it joins the
      // tab order once, and the arrows move between rows from there.
      tabIndex={props.tabIndex ?? (interactive ? 0 : undefined)}
      onKeyDown={(event) => {
        onKeyDown?.(event)
        if (interactive && !event.defaultPrevented)
          onInteractiveRowKeyDown(event)
      }}
      className={cn(
        uiClasses.tableRow,
        interactive &&
          'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent',
        className,
      )}
      {...props}
    />
  ),
)
TableRow.displayName = 'TableRow'

export const TableHead = React.forwardRef<
  HTMLTableCellElement,
  React.ThHTMLAttributes<HTMLTableCellElement>
>(({ className, ...props }, ref) => (
  <th ref={ref} className={cn(uiClasses.tableHead, className)} {...props} />
))
TableHead.displayName = 'TableHead'

export const TableCell = React.forwardRef<
  HTMLTableCellElement,
  React.TdHTMLAttributes<HTMLTableCellElement>
>(({ className, ...props }, ref) => (
  <td ref={ref} className={cn(uiClasses.tableCell, className)} {...props} />
))
TableCell.displayName = 'TableCell'

export const TableCaption = React.forwardRef<
  HTMLTableCaptionElement,
  React.HTMLAttributes<HTMLTableCaptionElement>
>(({ className, ...props }, ref) => (
  <caption
    ref={ref}
    className={cn(uiClasses.tableCaption, className)}
    {...props}
  />
))
TableCaption.displayName = 'TableCaption'
