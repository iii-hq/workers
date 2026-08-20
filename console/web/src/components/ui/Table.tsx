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

export const TableRow = React.forwardRef<HTMLTableRowElement, TableRowProps>(
  ({ className, interactive, selected, ...props }, ref) => (
    <tr
      ref={ref}
      data-interactive={interactive || undefined}
      data-selected={selected || undefined}
      className={cn(uiClasses.tableRow, className)}
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
