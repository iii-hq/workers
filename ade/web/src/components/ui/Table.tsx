import * as React from 'react'
import {
  TableBody as BaseTableBody,
  TableCaption as BaseTableCaption,
  TableCell as BaseTableCell,
  TableFooter as BaseTableFooter,
  TableHead as BaseTableHead,
  TableHeader as BaseTableHeader,
  TableRow as BaseTableRow,
} from '@/components/shadcn/table'
import { cn } from '@/lib/utils'

export type TableDensity = 'comfortable' | 'compact'

/** The scrolling box around a table: horizontal overflow, nothing else. */
export const TableViewport = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="table-container"
    className={cn('relative w-full overflow-x-auto', className)}
    {...props}
  />
))
TableViewport.displayName = 'TableViewport'

/** A table in a card: the `border` ring and the card's corners. */
export const TableFrame = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="table-frame"
    className={cn('overflow-hidden rounded-lg border border-border', className)}
    {...props}
  />
))
TableFrame.displayName = 'TableFrame'

export interface TableProps
  extends React.TableHTMLAttributes<HTMLTableElement> {
  density?: TableDensity
  /** Keep the first/last cell padding — for a table inside a card or panel. */
  inset?: boolean
}

/**
 * The shadcn table under the first console API. `density` sets the row
 * height (40px, or 32px compact) on every cell below; the table is put in
 * a `TableViewport` or `TableFrame` by the caller, so it never scrolls on
 * its own.
 */
export const Table = React.forwardRef<HTMLTableElement, TableProps>(
  ({ className, density = 'comfortable', inset, ...props }, ref) => (
    <table
      ref={ref}
      data-slot="table"
      data-density={density}
      data-inset={inset || undefined}
      className={cn(
        'w-full caption-bottom text-sm',
        density === 'compact' && '[&_td]:py-1 [&_th]:h-8',
        // Flush with the page text when the table stands alone; inside a
        // `TableFrame` the first and last cells keep a 12px gutter from the
        // card's edge, so "Status" never touches the ring.
        !inset &&
          'not-in-data-[slot=table-frame]:[&_td:first-child]:pl-0 not-in-data-[slot=table-frame]:[&_th:first-child]:pl-0',
        'in-data-[slot=table-frame]:[&_td:first-child]:pl-3 in-data-[slot=table-frame]:[&_th:first-child]:pl-3 in-data-[slot=table-frame]:[&_td:last-child]:pr-3 in-data-[slot=table-frame]:[&_th:last-child]:pr-3',
        className,
      )}
      {...props}
    />
  ),
)
Table.displayName = 'Table'

export const TableHeader = BaseTableHeader
export const TableBody = BaseTableBody
export const TableFooter = BaseTableFooter
export const TableHead = BaseTableHead
export const TableCell = BaseTableCell
export const TableCaption = BaseTableCaption

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

/** A row; `interactive` rows join the tab order and answer the keyboard. */
export const TableRow = React.forwardRef<HTMLTableRowElement, TableRowProps>(
  ({ className, interactive, selected, onKeyDown, ...props }, ref) => (
    <BaseTableRow
      ref={ref}
      data-interactive={interactive || undefined}
      data-selected={selected || undefined}
      data-state={selected ? 'selected' : undefined}
      // A row that answers a click answers the keyboard too: it joins the
      // tab order once, and the arrows move between rows from there.
      tabIndex={props.tabIndex ?? (interactive ? 0 : undefined)}
      onKeyDown={(event) => {
        onKeyDown?.(event)
        if (interactive && !event.defaultPrevented)
          onInteractiveRowKeyDown(event)
      }}
      className={cn(
        interactive &&
          'cursor-pointer focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-inset focus-visible:outline-none',
        className,
      )}
      {...props}
    />
  ),
)
TableRow.displayName = 'TableRow'
