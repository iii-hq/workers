import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/shadcn/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'

interface TracesPaginationProps {
  currentPage: number
  totalPages: number
  totalItems: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  pageSizeOptions?: number[]
}

/** The list's footer: rows per page, the window shown, and page arrows —
 *  quiet by design, secondary to the search above it. */
export function TracesPagination({
  currentPage,
  totalPages,
  totalItems,
  pageSize,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [25, 50, 100],
}: TracesPaginationProps) {
  const start = totalItems === 0 ? 0 : (currentPage - 1) * pageSize + 1
  const end = Math.min(currentPage * pageSize, totalItems)
  const items = pageSizeOptions.map((size) => ({
    value: String(size),
    label: `${size} per page`,
  }))
  const pages = Math.max(1, totalPages)
  return (
    <nav
      aria-label="trace pages"
      className="flex h-8 items-center justify-between gap-3 font-inter text-[11px] text-muted-foreground"
    >
      <Select
        value={String(pageSize)}
        onValueChange={(value) => onPageSizeChange(Number(value))}
        items={items}
      >
        <SelectTrigger
          size="sm"
          aria-label="rows per page"
          className="h-7 rounded-md border-transparent bg-transparent px-1.5 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent align="start" className="rounded-lg">
          {items.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <div className="flex items-center gap-1">
        <span className="font-mono tabular-nums">
          {start.toLocaleString('en-US')}–{end.toLocaleString('en-US')} of{' '}
          {totalItems.toLocaleString('en-US')}
        </span>
        <span aria-hidden className="mx-1 text-border">
          |
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="previous page"
          disabled={currentPage <= 1}
          onClick={() => onPageChange(currentPage - 1)}
        >
          <ChevronLeft />
        </Button>
        <span className="font-mono tabular-nums">
          {currentPage} / {pages}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight />
        </Button>
      </div>
    </nav>
  )
}
