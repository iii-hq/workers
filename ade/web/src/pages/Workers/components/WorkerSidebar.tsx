import { Boxes, Layers, Search, X } from 'lucide-react'
import * as React from 'react'
import {
  type ReactNode,
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Input } from '@/components/shadcn/input'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import {
  searchWorkers,
  splitRanges,
  suggestWorker,
} from '../lib/search-workers'
import type { ListGroup, ListItem } from '../model'
import { toneFor } from '../model'

const SKELETON_ROWS = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']

export type RowFlags = {
  update: string | null
  dirty: boolean
  awaiting: boolean
}

export interface WorkerSidebarProps {
  loaded: boolean
  composed: boolean
  groups: ListGroup[]
  query: string
  onQueryChange: (next: string) => void
  selection: string
  onSelect: (name: string) => void
  /** The project row's summary and its failure count. */
  project: { ready: number; total: number; updates: number; failing: number }
  flags: (name: string) => RowFlags
  filterRef: React.RefObject<HTMLInputElement | null>
  listRef: React.RefObject<HTMLElement | null>
}

/** The project id the list uses for its first row. */
export const PROJECT = 'project'

/**
 * The navigation column: a search box, then the project row and every
 * worker grouped by where it comes from. Typing swaps the groups for one
 * ranked result list — name first, then version, state and group — with
 * the matched letters marked. The arrows walk the results from the box,
 * Enter opens the active one, Escape clears, and `/` focuses the box from
 * anywhere on the page. Rows change colour instantly: a list this busy
 * animates nothing.
 */
export function WorkerSidebar({
  loaded,
  composed,
  groups,
  query,
  onQueryChange,
  selection,
  onSelect,
  project,
  flags,
  filterRef,
  listRef,
}: WorkerSidebarProps) {
  const deferredQuery = useDeferredValue(query)
  const searching = query.trim().length > 0
  const listId = useId()
  const hits = useMemo(
    () => (searching ? searchWorkers(groups, deferredQuery) : []),
    [groups, deferredQuery, searching],
  )
  const suggestion = useMemo(
    () =>
      searching && hits.length === 0
        ? suggestWorker(groups, deferredQuery)
        : null,
    [groups, deferredQuery, hits.length, searching],
  )
  const total = groups.reduce((sum, group) => sum + group.items.length, 0)

  const [active, setActive] = useState(0)
  // A new result set starts on its best match.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset per result set.
  useEffect(() => setActive(0), [hits])
  const activeHit = hits[Math.min(active, Math.max(hits.length - 1, 0))]
  const optionId = (index: number) => `${listId}-option-${index}`
  const activeRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: 'nearest' })
  }, [])

  // `/` focuses the search from anywhere on the page that is not a field.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey)
        return
      const target = event.target as HTMLElement | null
      if (
        target?.closest(
          'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="textbox"]',
        )
      )
        return
      const input = filterRef.current
      if (!input?.isConnected) return
      event.preventDefault()
      input.focus()
      input.select()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [filterRef])

  const clear = () => {
    onQueryChange('')
    filterRef.current?.focus()
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="relative shrink-0 px-3 pt-1 pb-2">
        <Search
          className="pointer-events-none absolute left-[22px] top-1/2 size-4 shrink-0 -translate-y-1/2 text-neutral-400 dark:text-neutral-500"
          aria-hidden
        />
        <Input
          ref={filterRef}
          type="search"
          name="workers-search"
          aria-label="Filter workers"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={
            searching && activeHit
              ? optionId(hits.indexOf(activeHit))
              : undefined
          }
          aria-keyshortcuts="/"
          placeholder="Search workers"
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(event) => onQueryChange(event.currentTarget.value)}
          className="wk-search h-8 rounded-lg border-neutral-200 bg-white pl-8 pr-8 text-[13px] shadow-none placeholder:text-neutral-400 pointer-coarse:h-11 dark:border-neutral-800 dark:bg-neutral-900/70 dark:placeholder:text-neutral-500 [&::-webkit-search-cancel-button]:hidden"
          onKeyDown={(event) => {
            if (event.key === 'Escape' && query) {
              event.preventDefault()
              event.stopPropagation()
              onQueryChange('')
              return
            }
            if (!searching || hits.length === 0) return
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault()
              const step = event.key === 'ArrowDown' ? 1 : -1
              setActive(
                (current) => (current + step + hits.length) % hits.length,
              )
            } else if (event.key === 'Enter' && activeHit) {
              event.preventDefault()
              onSelect(activeHit.item.name)
            }
          }}
        />
        {query ? (
          <button
            type="button"
            aria-label="Clear"
            onClick={clear}
            className="absolute right-4 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-neutral-400 outline-none transition-colors duration-150 hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-50"
          >
            <span
              className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            <X className="size-4" aria-hidden />
          </button>
        ) : (
          <kbd
            className="pointer-events-none absolute right-5 top-1/2 flex h-5 min-w-5 -translate-y-1/2 items-center justify-center rounded border border-neutral-200 bg-neutral-50 px-1 font-inter text-[11px] text-neutral-400 pointer-coarse:hidden dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-500"
            aria-hidden
          >
            /
          </kbd>
        )}
      </div>

      {!loaded ? (
        <div
          className="flex flex-col gap-1 px-3"
          role="status"
          aria-busy="true"
          aria-label="Loading workers"
        >
          {SKELETON_ROWS.map((id) => (
            <Skeleton key={id} className="h-10 rounded-lg" />
          ))}
        </div>
      ) : (
        <nav
          ref={listRef}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-4"
          aria-label="Project and workers"
        >
          {searching ? (
            hits.length > 0 ? (
              <Group
                label="Results"
                aside={
                  <span
                    className="tabular-nums text-neutral-400 dark:text-neutral-500"
                    aria-live="polite"
                    aria-atomic="true"
                  >
                    <span className="sr-only">
                      {hits.length === 1
                        ? '1 worker'
                        : `${hits.length} workers`}{' '}
                      match
                    </span>
                    <span aria-hidden>{hits.length}</span>
                  </span>
                }
                listbox={{ id: listId, label: 'Matching workers' }}
              >
                {hits.map((hit, index) => {
                  const isActive = hit === activeHit
                  return (
                    <Row
                      key={hit.item.name}
                      ref={isActive ? activeRef : undefined}
                      id={optionId(index)}
                      item={hit.item}
                      flags={flags(hit.item.name)}
                      selected={selection === hit.item.name}
                      active={isActive}
                      group={hit.groupLabel}
                      nameRanges={hit.nameRanges}
                      detailRanges={hit.detailRanges}
                      onClick={() => onSelect(hit.item.name)}
                      onHover={() => setActive(index)}
                    />
                  )
                })}
              </Group>
            ) : (
              <NoMatch
                query={query}
                suggestion={suggestion}
                onSuggest={(name) => onQueryChange(name)}
                onClear={clear}
              />
            )
          ) : (
            <>
              {composed ? (
                <nav aria-label="Project" className="mb-3">
                  <button
                    type="button"
                    aria-current={selection === PROJECT ? 'page' : undefined}
                    onClick={() => onSelect(PROJECT)}
                    className={cn(ROW, selection === PROJECT && ROW_SELECTED)}
                  >
                    <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-neutral-200/70 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                      <Layers className="size-4" aria-hidden />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] font-medium leading-5">
                        Project
                      </span>
                      <span className="block truncate text-[12px] leading-4 text-neutral-600 dark:text-neutral-400">
                        {project.ready} of {project.total} ready
                        {project.updates
                          ? ` · ${project.updates} update${project.updates === 1 ? '' : 's'}`
                          : ''}
                      </span>
                    </span>
                    {project.failing ? (
                      <Pill tone="alert">{project.failing} failed</Pill>
                    ) : null}
                  </button>
                </nav>
              ) : null}
              {groups.map((group) => (
                <Group
                  key={group.id}
                  label={group.label}
                  aside={
                    <span className="tabular-nums text-neutral-400 dark:text-neutral-500">
                      {group.items.length}
                    </span>
                  }
                >
                  {group.items.map((item) => (
                    <Row
                      key={item.name}
                      item={item}
                      flags={flags(item.name)}
                      selected={selection === item.name}
                      onClick={() => onSelect(item.name)}
                    />
                  ))}
                </Group>
              ))}
              {total === 0 ? (
                <p className="px-3 pt-6 text-center text-[13px] text-neutral-500 dark:text-neutral-400">
                  No workers yet.
                </p>
              ) : null}
            </>
          )}
        </nav>
      )}
    </div>
  )
}

const ROW =
  'group/row flex w-full min-w-0 items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left font-inter text-neutral-700 transition-colors duration-150 ease-out outline-none hover:bg-neutral-200/60 hover:text-neutral-900 focus-visible:ring-2 focus-visible:ring-neutral-500 pointer-coarse:min-h-12 dark:text-neutral-300 dark:hover:bg-neutral-900 dark:hover:text-neutral-50 dark:focus-visible:ring-neutral-400'
const ROW_SELECTED =
  'bg-neutral-200/80 text-neutral-900 hover:bg-neutral-200/80 dark:bg-neutral-800/80 dark:text-neutral-50 dark:hover:bg-neutral-800/80'
const ROW_ACTIVE =
  'bg-neutral-200/60 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50'

function Group({
  label,
  aside,
  listbox,
  children,
}: {
  label: string
  aside?: ReactNode
  listbox?: { id: string; label: string }
  children: ReactNode
}) {
  return (
    <section className="mb-3">
      <div className="flex items-center justify-between px-3 pb-1 pt-1 text-[12px] font-medium text-neutral-500 dark:text-neutral-400">
        <span>{label}</span>
        {aside}
      </div>
      <div
        className="flex flex-col gap-px"
        {...(listbox
          ? { id: listbox.id, role: 'listbox', 'aria-label': listbox.label }
          : {})}
      >
        {children}
      </div>
    </section>
  )
}

const Row = React.forwardRef<
  HTMLButtonElement,
  {
    id?: string
    item: ListItem
    flags: RowFlags
    selected: boolean
    active?: boolean
    group?: string
    nameRanges?: readonly (readonly [number, number])[]
    detailRanges?: readonly (readonly [number, number])[]
    onClick: () => void
    onHover?: () => void
  }
>(function Row(
  {
    id,
    item,
    flags,
    selected,
    active,
    group,
    nameRanges = [],
    detailRanges = [],
    onClick,
    onHover,
  },
  ref,
) {
  const tone = toneFor(item.state)
  return (
    <button
      ref={ref}
      id={id}
      type="button"
      {...(id ? { role: 'option', 'aria-selected': !!active } : {})}
      aria-current={selected ? 'page' : undefined}
      data-list-item
      onClick={onClick}
      onPointerMove={onHover}
      className={cn(ROW, active && ROW_ACTIVE, selected && ROW_SELECTED)}
    >
      <span className="flex size-6 shrink-0 items-center justify-center">
        <span
          aria-hidden
          className={cn(
            'size-2 rounded-full',
            DOT[tone.dot],
            item.state === 'starting' && 'pulse-dot',
          )}
        />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-medium leading-5">
          <Highlight text={item.name} ranges={nameRanges} />
        </span>
        <span
          className="iii-ui-list-item__description wk-row-detail block truncate text-[12px] leading-4"
          data-failed={item.failed || undefined}
        >
          {item.detail === item.state ? (
            item.state
          ) : (
            <>
              {item.state} ·{' '}
              <Highlight text={item.detail} ranges={detailRanges} />
            </>
          )}
          {group ? (
            <span className="text-neutral-400 dark:text-neutral-600">
              {' '}
              · {group}
            </span>
          ) : null}
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-1">
        {flags.dirty ? <Pill>unsaved</Pill> : null}
        {flags.awaiting ? <Pill>pending</Pill> : null}
        {flags.update ? (
          <Pill tone="accent" aria-label={`Update available: ${flags.update}`}>
            {flags.update}
          </Pill>
        ) : null}
      </span>
    </button>
  )
})

const DOT: Record<string, string> = {
  accent: 'bg-ok',
  alert: 'bg-alert',
  warn: 'bg-warn',
  ink: 'bg-neutral-400 dark:bg-neutral-600',
  ok: 'bg-ok',
  ghost: 'bg-neutral-300 dark:bg-neutral-700',
}

function Pill({
  tone = 'default',
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & {
  tone?: 'default' | 'accent' | 'alert'
}) {
  return (
    <span
      className={cn(
        'inline-flex h-6 items-center rounded-lg border px-2 font-inter text-[12px] font-medium tabular-nums',
        tone === 'default' &&
          'border-neutral-200 bg-neutral-100 text-neutral-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300',
        // An update: the neutral chip with the version in green, so the
        // list reads calm and the new version still stands out.
        tone === 'accent' &&
          'border-neutral-200 bg-neutral-100 text-emerald-700 dark:border-neutral-800 dark:bg-neutral-900 dark:text-emerald-400',
        tone === 'alert' && 'border-alert/25 bg-alert-muted text-alert-strong',
        className,
      )}
      {...props}
    />
  )
}

function Highlight({
  text,
  ranges,
}: {
  text: string
  ranges: readonly (readonly [number, number])[]
}) {
  if (ranges.length === 0) return <>{text}</>
  return (
    <>
      {splitRanges(text, ranges).map((piece, index) =>
        piece.hit ? (
          <mark
            // biome-ignore lint/suspicious/noArrayIndexKey: static split of one string
            key={index}
            className="rounded-[2px] bg-transparent text-neutral-900 underline decoration-neutral-400 decoration-1 underline-offset-2 dark:text-neutral-50 dark:decoration-neutral-500"
          >
            {piece.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: static split of one string
          <span key={index}>{piece.text}</span>
        ),
      )}
    </>
  )
}

function NoMatch({
  query,
  suggestion,
  onSuggest,
  onClear,
}: {
  query: string
  suggestion: string | null
  onSuggest: (name: string) => void
  onClear: () => void
}) {
  return (
    <div className="flex flex-col items-center gap-2 px-3 pt-8 text-center">
      <span className="flex size-9 items-center justify-center rounded-lg bg-neutral-200/60 text-neutral-500 dark:bg-neutral-900 dark:text-neutral-400">
        <Boxes className="size-4" aria-hidden />
      </span>
      <p className="text-[13px] font-medium text-neutral-900 dark:text-neutral-50">
        No worker matches
      </p>
      <p className="text-[12px] text-neutral-500 dark:text-neutral-400">
        Nothing is called “{query.trim()}”.
      </p>
      {suggestion ? (
        <button
          type="button"
          onClick={() => onSuggest(suggestion)}
          className="mt-1 rounded-md px-2 py-1 text-[12px] text-neutral-700 underline decoration-neutral-400 underline-offset-2 transition-colors duration-150 hover:text-neutral-900 dark:text-neutral-300 dark:hover:text-neutral-50"
        >
          Did you mean {suggestion}?
        </button>
      ) : null}
      <button
        type="button"
        onClick={onClear}
        className="mt-1 h-7 rounded-md px-2.5 text-[12px] font-medium text-neutral-600 transition-colors duration-150 hover:bg-neutral-200/70 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-900 dark:hover:text-neutral-50"
      >
        Clear search
      </button>
    </div>
  )
}
