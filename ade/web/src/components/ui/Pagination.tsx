import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/shadcn/button'
import {
  Pagination as BasePagination,
  PaginationContent,
  PaginationItem,
} from '@/components/shadcn/pagination'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import { cn } from '@/lib/utils'

interface PaginationProps {
  currentPage: number
  totalPages: number
  totalItems: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  pageSizeOptions?: number[]
  className?: string
}

/**
 * Page controls on the shadcn pagination parts: the page-size select, the
 * range shown, then previous / next around the page count. Every number is
 * tabular so nothing shifts as the page changes.
 */
export function Pagination({
  currentPage,
  totalPages,
  totalItems,
  pageSize,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [25, 50, 100],
  className,
}: PaginationProps) {
  const start = totalItems === 0 ? 0 : (currentPage - 1) * pageSize + 1
  const end = Math.min(currentPage * pageSize, totalItems)
  return (
    <BasePagination
      className={cn(
        'mx-0 w-auto justify-between gap-4 text-xs text-muted-foreground',
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <span className="text-meta font-medium">Show</span>
        <Select
          value={String(pageSize)}
          onValueChange={(next) => {
            if (next !== null) onPageSizeChange(Number(next))
          }}
        >
          <SelectTrigger size="sm" aria-label="Rows per page">
            <SelectValue />
          </SelectTrigger>
          <SelectContent align="start" alignItemWithTrigger={false}>
            {pageSizeOptions.map((opt) => (
              <SelectItem key={opt} value={String(opt)}>
                {opt}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <PaginationContent className="gap-2 tabular-nums">
        <PaginationItem>
          {start}–{end} of {totalItems}
        </PaginationItem>
        <PaginationItem>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Previous page"
            disabled={currentPage <= 1}
            onClick={() => onPageChange(currentPage - 1)}
          >
            <ChevronLeft className="size-4" />
          </Button>
        </PaginationItem>
        <PaginationItem className="text-meta font-medium">
          page {currentPage} of {totalPages}
        </PaginationItem>
        <PaginationItem>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Next page"
            disabled={currentPage >= totalPages}
            onClick={() => onPageChange(currentPage + 1)}
          >
            <ChevronRight className="size-4" />
          </Button>
        </PaginationItem>
      </PaginationContent>
    </BasePagination>
  )
}
