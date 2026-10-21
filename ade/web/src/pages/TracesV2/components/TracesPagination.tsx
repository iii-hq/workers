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

/** The list's footer: rows per page, the window shown, and page arrows. */
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
  return (
    <div className="flex items-center justify-between gap-4 text-xs text-muted-foreground">
      <Select
        value={String(pageSize)}
        onValueChange={(value) => onPageSizeChange(Number(value))}
        items={items}
      >
        <SelectTrigger size="sm" aria-label="rows per page">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {items.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <div className="flex items-center gap-1">
        <span className="mr-2 font-mono tabular-nums">
          {start}–{end} of {totalItems}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="previous page"
          disabled={currentPage <= 1}
          onClick={() => onPageChange(currentPage - 1)}
        >
          <ChevronLeft />
        </Button>
        <span className="tabular-nums">
          page {currentPage} of {Math.max(1, totalPages)}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight />
        </Button>
      </div>
    </div>
  )
}
