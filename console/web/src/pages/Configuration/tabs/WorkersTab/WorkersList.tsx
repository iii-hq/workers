import { Search, X } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import type { ConfigurationSchemaView } from './api'
import { wt } from './typography'

interface WorkersListProps {
  configurations: ConfigurationSchemaView[]
  selectedId: string | null
  onSelect: (id: string | null) => void
  isLoading: boolean
  isError: boolean
  errorMessage?: string
  /** Drill-in mode: the list fills the pane and rows get touch sizing. */
  narrow?: boolean
}

/**
 * Navigation sidebar for the worker configuration master-detail editor
 * (mirrors the directory worker's entry list): search with a clear
 * affordance, a live count line, and rows where selection reads as a
 * wash + accent indicator + stronger title, not color alone. Selection
 * is fully parent-owned so the rail stays a pure presentational view.
 */
export function WorkersList({
  configurations,
  selectedId,
  onSelect,
  isLoading,
  isError,
  errorMessage,
  narrow = false,
}: WorkersListProps) {
  const [filter, setFilter] = useState('')
  const inputRef = useRef<HTMLInputElement | null>(null)
  const trimmed = filter.trim().toLowerCase()

  const filtered = useMemo(() => {
    if (!trimmed) return configurations
    return configurations.filter((c) => {
      const haystack = `${c.id} ${c.name} ${c.description}`.toLowerCase()
      return haystack.includes(trimmed)
    })
  }, [configurations, trimmed])

  return (
    <aside
      className={cn(
        'flex flex-col min-h-0 bg-sidebar',
        narrow ? 'flex-1 min-w-0' : 'w-72 shrink-0',
      )}
      aria-label="worker configurations"
    >
      <div className="px-3 pt-3 pb-2 shrink-0 space-y-2">
        <div className="relative">
          <Search
            className="absolute left-2.5 top-1/2 -translate-y-1/2 size-3.5 text-ink-ghost pointer-events-none"
            aria-hidden
          />
          <Input
            ref={inputRef}
            value={filter}
            onChange={setFilter}
            placeholder="filter workers…"
            aria-label="filter configurations"
            preserveCase
            className={cn(wt.control, 'pl-8 pr-8')}
            onKeyDown={(e) => {
              if (e.key === 'Escape' && filter) setFilter('')
            }}
          />
          {filter ? (
            <button
              type="button"
              aria-label="clear filter"
              onClick={() => {
                setFilter('')
                inputRef.current?.focus()
              }}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 flex items-center justify-center size-6 rounded-sm text-ink-ghost hover:text-ink hover:bg-surface-hover transition-colors"
            >
              <X className="size-3.5" aria-hidden />
            </button>
          ) : null}
        </div>
        {!isLoading && !isError ? (
          <p
            className="px-0.5 font-mono text-[12px] text-ink-ghost tabular-nums"
            aria-live="polite"
          >
            {trimmed
              ? `${filtered.length} of ${configurations.length} workers`
              : `${configurations.length} worker${configurations.length === 1 ? '' : 's'}`}
          </p>
        ) : null}
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto px-1.5 pb-3">
        {isLoading ? <LoadingRows /> : null}
        {isError ? (
          <p className={cn('px-2 py-4', wt.bodySm, 'text-alert')}>
            {errorMessage ?? 'failed to load configurations'}
          </p>
        ) : null}
        {!isLoading && !isError && configurations.length === 0 ? (
          <p className={cn('px-2 py-6', wt.bodySm, 'text-ink-faint')}>
            no configurations registered yet.
          </p>
        ) : null}
        {!isLoading &&
        !isError &&
        configurations.length > 0 &&
        filtered.length === 0 ? (
          <div className="px-2 py-6 space-y-2">
            <p className={cn(wt.bodySm, 'text-ink-faint')}>
              no matches for "{filter.trim()}".
            </p>
            <Button variant="ghost" size="sm" onClick={() => setFilter('')}>
              clear filter
            </Button>
          </div>
        ) : null}
        <ul className="flex flex-col gap-px py-1">
          {filtered.map((entry) => (
            <li key={entry.id}>
              <ConfigurationRow
                entry={entry}
                selected={entry.id === selectedId}
                narrow={narrow}
                onSelect={() => onSelect(entry.id)}
              />
            </li>
          ))}
        </ul>
      </div>
    </aside>
  )
}

interface ConfigurationRowProps {
  entry: ConfigurationSchemaView
  selected: boolean
  narrow: boolean
  onSelect: () => void
}

function ConfigurationRow({
  entry,
  selected,
  narrow,
  onSelect,
}: ConfigurationRowProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? 'true' : undefined}
      className={cn(
        'relative w-full text-left rounded-md pl-3.5 pr-2.5 transition-colors',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus',
        // Touch-sized targets in the drill-in flow.
        narrow ? 'py-3' : 'py-2',
        selected ? 'bg-surface-selected' : 'hover:bg-surface-hover',
        // Selection = wash + accent indicator, not color alone.
        selected &&
          "before:content-[''] before:absolute before:left-1 before:top-2 before:bottom-2 before:w-0.5 before:rounded-full before:bg-accent",
      )}
    >
      <div className="flex items-baseline gap-2 min-w-0">
        <span
          className={cn(
            wt.body,
            'text-ink truncate',
            selected ? 'font-semibold' : 'font-medium',
          )}
        >
          {entry.name || entry.id}
        </span>
        {entry.name && entry.name !== entry.id ? (
          <span className="font-mono text-[12px] text-ink-ghost truncate">
            {entry.id}
          </span>
        ) : null}
      </div>
      {entry.description ? (
        <p className={cn(wt.caption, 'text-ink-faint mt-0.5 line-clamp-2')}>
          {entry.description}
        </p>
      ) : null}
    </button>
  )
}

function LoadingRows() {
  return (
    <ul className="py-1">
      {[0, 1, 2].map((i) => (
        <li key={i} className="px-2 py-2.5 space-y-2">
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-3 w-48" />
        </li>
      ))}
    </ul>
  )
}
