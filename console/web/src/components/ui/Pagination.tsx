import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/Button'
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
    <div
      className={cn(
        'flex items-center justify-between gap-4 font-mono text-[12px] text-ink-faint lowercase',
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <span className="uppercase tracking-[0.06em] text-[11px]">show</span>
        <select
          value={pageSize}
          onChange={(e) => onPageSizeChange(Number(e.target.value))}
          className="rounded-sm border border-transparent bg-surface text-ink font-mono text-[12px] px-1.5 py-0.5 focus:outline-none focus:border-rule-focus"
        >
          {pageSizeOptions.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      </div>
      <div className="flex items-center gap-2 tabular-nums">
        <span>
          {start}–{end} of {totalItems}
        </span>
        <Button
          variant="icon"
          size="icon"
          aria-label="previous page"
          disabled={currentPage <= 1}
          onClick={() => onPageChange(currentPage - 1)}
        >
          <ChevronLeft className="w-3.5 h-3.5" />
        </Button>
        <span className="uppercase tracking-[0.06em] text-[11px]">
          page {currentPage} of {totalPages}
        </span>
        <Button
          variant="icon"
          size="icon"
          aria-label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight className="w-3.5 h-3.5" />
        </Button>
      </div>
    </div>
  )
}
