import { useMemo, useState } from 'react'
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
}

/**
 * Left rail for the worker configuration master-detail editor. Lists every
 * registered configuration, filtered by a small inline search field. Selection
 * is fully parent-owned so the rail stays a pure presentational view.
 *
 * Visual vocabulary mirrors the rest of the console: monospace, lowercase,
 * single-pixel rules, no rounded corners. Active row uses the inverse
 * ink/bg pair already used by the header `⚙` button so selection reads
 * as "this is the active surface", not as a callout.
 */
export function WorkersList({
  configurations,
  selectedId,
  onSelect,
  isLoading,
  isError,
  errorMessage,
}: WorkersListProps) {
  const [filter, setFilter] = useState('')
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
      className="w-72 shrink-0 border-r border-rule flex flex-col min-h-0"
      aria-label="worker configurations"
    >
      <div className="px-3 py-3 border-b border-rule shrink-0">
        <Input
          value={filter}
          onChange={setFilter}
          placeholder="filter…"
          aria-label="filter configurations"
          preserveCase
          className={wt.control}
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {isLoading ? <LoadingRows /> : null}
        {isError ? (
          <p className={cn('px-3 py-4', wt.bodySm, 'text-alert')}>
            {errorMessage ?? 'failed to load configurations'}
          </p>
        ) : null}
        {!isLoading && !isError && configurations.length === 0 ? (
          <p className={cn('px-3 py-4', wt.bodySm, 'text-ink-faint')}>
            no configurations registered yet.
          </p>
        ) : null}
        {!isLoading &&
        !isError &&
        configurations.length > 0 &&
        filtered.length === 0 ? (
          <p className={cn('px-3 py-4', wt.bodySm, 'text-ink-faint')}>
            no matches for "{filter}".
          </p>
        ) : null}
        <ul className="py-1">
          {filtered.map((entry) => (
            <li key={entry.id}>
              <ConfigurationRow
                entry={entry}
                selected={entry.id === selectedId}
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
  onSelect: () => void
}

function ConfigurationRow({
  entry,
  selected,
  onSelect,
}: ConfigurationRowProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? 'true' : undefined}
      className={cn(
        'w-full text-left px-3 py-2 rounded-sm transition-colors focus:outline-none focus-visible:bg-surface-hover',
        selected ? 'bg-surface-selected' : 'hover:bg-surface-hover',
      )}
    >
      <div className="flex items-baseline gap-2">
        <span className={cn(wt.body, 'text-ink truncate')}>
          {entry.name || entry.id}
        </span>
        {entry.name && entry.name !== entry.id ? (
          <span className={cn(wt.micro, 'text-ink-ghost truncate')}>
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
        <li key={i} className="px-3 py-2 space-y-2">
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-3 w-48" />
        </li>
      ))}
    </ul>
  )
}
