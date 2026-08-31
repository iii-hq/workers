import { Search, X } from 'lucide-react'
import { Input } from '@/components/ui/Input'
import { cn } from '@/lib/utils'
import type {
  WorkerManagementKind,
  WorkerRow,
  WorkersFilterState,
} from '../types'
import {
  distinctManagement,
  distinctRuntimes,
  distinctTags,
  MANAGEMENT_LABEL,
} from '../types'

interface WorkersFiltersProps {
  rows: WorkerRow[]
  filters: WorkersFilterState
  onFilterChange: (next: Partial<WorkersFilterState>) => void
  onClear: () => void
  className?: string
}

export function WorkersFilters({
  rows,
  filters,
  onFilterChange,
  onClear,
  className,
}: WorkersFiltersProps) {
  const tags = distinctTags(rows)
  const runtimes = distinctRuntimes(rows)
  const management = distinctManagement(rows)
  const hasActive =
    filters.search.trim() !== '' ||
    filters.tag !== null ||
    filters.runtime !== null ||
    filters.management !== null

  return (
    <div className={cn('flex flex-col gap-3', className)}>
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative flex-1 min-w-[200px] max-w-md">
          <Search
            aria-hidden
            className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-ink-faint pointer-events-none"
          />
          <Input
            value={filters.search}
            onChange={(next) => onFilterChange({ search: next })}
            placeholder="search workers…"
            aria-label="search workers"
            preserveCase
            className="pl-9"
          />
        </div>
        {hasActive ? (
          <button
            type="button"
            onClick={onClear}
            className="inline-flex items-center gap-1 font-mono text-[12px] text-ink-faint hover:text-ink lowercase"
          >
            <X className="size-4" aria-hidden />
            clear filters
          </button>
        ) : null}
      </div>
      {(management.length > 1 || tags.length > 0 || runtimes.length > 0) && (
        <div className="flex flex-wrap items-center gap-2">
          {management.length > 1 ? (
            <FilterChipGroup
              label="managed by"
              options={management}
              labelFor={(kind) =>
                MANAGEMENT_LABEL[kind as WorkerManagementKind]
              }
              selected={filters.management}
              onSelect={(kind) =>
                onFilterChange({
                  management:
                    filters.management === kind
                      ? null
                      : (kind as WorkerManagementKind),
                })
              }
            />
          ) : null}
          {tags.length > 0 ? (
            <FilterChipGroup
              label="tag"
              options={tags}
              selected={filters.tag}
              onSelect={(tag) =>
                onFilterChange({ tag: filters.tag === tag ? null : tag })
              }
            />
          ) : null}
          {runtimes.length > 0 ? (
            <FilterChipGroup
              label="runtime"
              options={runtimes}
              selected={filters.runtime}
              onSelect={(runtime) =>
                onFilterChange({
                  runtime: filters.runtime === runtime ? null : runtime,
                })
              }
            />
          ) : null}
        </div>
      )}
    </div>
  )
}

interface FilterChipGroupProps {
  label: string
  options: string[]
  selected: string | null
  onSelect: (value: string) => void
  labelFor?: (value: string) => string
}

function FilterChipGroup({
  label,
  options,
  selected,
  onSelect,
  labelFor = (value) => value,
}: FilterChipGroupProps) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost mr-1">
        {label}
      </span>
      {options.map((opt) => {
        const active = selected === opt
        return (
          <button
            key={opt}
            type="button"
            onClick={() => onSelect(opt)}
            aria-pressed={active}
            className={cn(
              'font-mono text-[11px] lowercase border px-2 py-0.5 transition-colors',
              active
                ? 'bg-ink text-bg border-ink'
                : 'bg-bg text-ink-faint border-rule hover:border-ink hover:text-ink',
            )}
          >
            {labelFor(opt)}
          </button>
        )
      })}
    </div>
  )
}
