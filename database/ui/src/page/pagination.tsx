/**
 * Pagination bar for the table data panel — ported from the console's
 * `ui/Pagination`. Prev/next use the shared `Button` (icon variant); the
 * page-size select is a raw `<select>` styled from design tokens.
 */

import { Button } from '@iii-dev/console-ui'
import { ChevronLeft, ChevronRight } from './icons'

interface PaginationProps {
  currentPage: number
  totalPages: number
  totalItems: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  pageSizeOptions?: number[]
}

export function Pagination({
  currentPage,
  totalPages,
  totalItems,
  pageSize,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [25, 50, 100],
}: PaginationProps) {
  const start = totalItems === 0 ? 0 : (currentPage - 1) * pageSize + 1
  const end = Math.min(currentPage * pageSize, totalItems)
  return (
    <div className="db-pager">
      <div className="db-pager-group">
        <span className="db-pager-cap">show</span>
        <select value={pageSize} onChange={(e) => onPageSizeChange(Number(e.target.value))} aria-label="rows per page">
          {pageSizeOptions.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      </div>
      <div className="db-pager-group">
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
          <ChevronLeft size={14} />
        </Button>
        {/* A value phrase, not a label — it stays in the pager's lowercase
            voice ("show" above is a label and keeps the caps). */}
        <span>
          page {currentPage} of {totalPages}
        </span>
        <Button
          variant="icon"
          size="icon"
          aria-label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight size={14} />
        </Button>
      </div>
    </div>
  )
}
