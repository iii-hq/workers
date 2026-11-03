/**
 * The data panel's pager, laid out the way a table editor reads it:
 * ← Page [n] of N →, then the page size. The page number is typed (Enter or
 * leaving the field jumps; Escape restores), prev/next are ghost icon
 * buttons, the size is the shadcn Select.
 */

import {
  Button,
  ShadcnInput,
  ShadcnSelect as Select,
  ShadcnSelectContent as SelectContent,
  ShadcnSelectItem as SelectItem,
  ShadcnSelectTrigger as SelectTrigger,
  ShadcnSelectValue as SelectValue,
} from '@iii-dev/console-ui'
import { ArrowLeft, ArrowRight } from 'lucide-react'
import { useEffect, useState } from 'react'

interface PaginationProps {
  currentPage: number
  totalPages: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  pageSizeOptions?: number[]
}

export function Pagination({
  currentPage,
  totalPages,
  pageSize,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [25, 50, 100],
}: PaginationProps) {
  const [draft, setDraft] = useState(String(currentPage))
  useEffect(() => setDraft(String(currentPage)), [currentPage])

  const commit = () => {
    const n = Number.parseInt(draft, 10)
    if (Number.isFinite(n) && n >= 1 && n <= totalPages && n !== currentPage) onPageChange(n)
    else setDraft(String(currentPage))
  }

  const sizes = pageSizeOptions.map((opt) => ({ value: String(opt), label: `${opt} rows` }))

  return (
    <div className="db-pager">
      <Button
        variant="ghost"
        size="icon"
        aria-label="previous page"
        disabled={currentPage <= 1}
        onClick={() => onPageChange(currentPage - 1)}
      >
        <ArrowLeft size={16} aria-hidden />
      </Button>
      <span className="db-pager-cap">Page</span>
      <ShadcnInput
        className="db-pager-input"
        inputMode="numeric"
        value={draft}
        aria-label={`page, 1 to ${totalPages}`}
        onChange={(e) => setDraft(e.target.value.replace(/[^0-9]/g, ''))}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commit()
          else if (e.key === 'Escape') setDraft(String(currentPage))
        }}
      />
      <span className="db-pager-cap">of {totalPages.toLocaleString()}</span>
      <Button
        variant="ghost"
        size="icon"
        aria-label="next page"
        disabled={currentPage >= totalPages}
        onClick={() => onPageChange(currentPage + 1)}
      >
        <ArrowRight size={16} aria-hidden />
      </Button>
      <Select items={sizes} value={String(pageSize)} onValueChange={(next) => next && onPageSizeChange(Number(next))}>
        <SelectTrigger size="sm" className="db-pager-size" aria-label="rows per page">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {sizes.map((s) => (
            <SelectItem key={s.value} value={s.value}>
              {s.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}
