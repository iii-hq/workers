/**
 * Pagination bar for the table data panel: prev/next as the shared icon
 * buttons, the page-size picker as the shared `Select`.
 */

import { IconButton, Select } from '@iii-dev/console-ui'
import { ChevronLeft, ChevronRight } from 'lucide-react'

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
        <span className="db-pager-cap">Show</span>
        <Select
          value={String(pageSize)}
          onChange={(next) => onPageSizeChange(Number(next))}
          options={pageSizeOptions.map((opt) => ({
            value: String(opt),
            label: String(opt),
          }))}
          aria-label="rows per page"
        />
      </div>
      <div className="db-pager-group">
        <span>
          {start}–{end} of {totalItems}
        </span>
        <IconButton
          label="previous page"
          disabled={currentPage <= 1}
          onClick={() => onPageChange(currentPage - 1)}
        >
          <ChevronLeft size={16} />
        </IconButton>
        <span>
          Page {currentPage} of {totalPages}
        </span>
        <IconButton
          label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight size={16} />
        </IconButton>
      </div>
    </div>
  )
}
