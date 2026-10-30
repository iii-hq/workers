import {
  ArrowLeft,
  CornerDownLeft,
  type LucideIcon,
  Search,
  SearchX,
  Settings2,
  X,
} from 'lucide-react'
import {
  type ReactNode,
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/shadcn/button'
import { Input } from '@/components/shadcn/input'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import {
  highlightRanges,
  type SettingsSearchField,
  type SettingsSearchIndex,
  type SettingsSearchResult,
  searchSettings,
  suggestSettingsQuery,
} from '../lib/settings-search'

export interface SettingsNavEntry {
  id: string
  label: string
  description: string
  icon: LucideIcon
}

/** The search doc id of the General page. */
export const GENERAL_SEARCH_ID = 'general'

interface SettingsNavigationProps {
  entries: SettingsNavEntry[]
  generalSelected: boolean
  selectedId: string | null
  onSelectGeneral: () => void
  onSelectWorker: (configurationId: string) => void
  /** Every page and its fields; `searchIndex.docs` covers General too. */
  searchIndex: SettingsSearchIndex
  /** A search result was chosen: a page, or one field on it. */
  onSelectResult: (docId: string, field?: SettingsSearchField) => void
  isLoading: boolean
  isError: boolean
  errorMessage?: string
  /** The sidebar is the whole page (narrow flow). */
  narrow: boolean
  /** Leave settings and return to the workspace. */
  onClose?: () => void
}

/** Matched fields listed under a page before the rest fold into a count. */
const FIELDS_PER_RESULT = 3

type SearchOption =
  | { kind: 'page'; key: string; result: SettingsSearchResult }
  | {
      kind: 'field'
      key: string
      result: SettingsSearchResult
      field: SettingsSearchField
    }

/**
 * The settings sidebar, after Linear: "Back to app" at the top, a compact
 * search, then grouped single-line links with a small group label. Typing
 * swaps the groups for ranked results: each page that matches, with the
 * fields on it that matched, every one a direct link. Arrow keys move
 * through the results from the box, Enter opens the active one, Escape
 * clears, and `/` focuses the box from anywhere in settings. Nothing here
 * animates; selection and hover are instant colour changes.
 */
export function SettingsNavigation({
  entries,
  generalSelected,
  selectedId,
  onSelectGeneral,
  onSelectWorker,
  searchIndex,
  onSelectResult,
  isLoading,
  isError,
  errorMessage,
  narrow,
  onClose,
}: SettingsNavigationProps) {
  const [query, setQuery] = useState('')
  const deferredQuery = useDeferredValue(query)
  const searchRef = useRef<HTMLInputElement | null>(null)
  const listId = useId()
  const searching = query.trim().length > 0

  const results = useMemo(
    () => searchSettings(searchIndex, deferredQuery),
    [searchIndex, deferredQuery],
  )
  const suggestion = useMemo(
    () =>
      results.length === 0 && deferredQuery.trim()
        ? suggestSettingsQuery(searchIndex, deferredQuery)
        : null,
    [results.length, searchIndex, deferredQuery],
  )
  const options = useMemo<SearchOption[]>(
    () =>
      results.flatMap((result) => [
        { kind: 'page' as const, key: result.doc.id, result },
        ...result.fields.slice(0, FIELDS_PER_RESULT).map((hit) => ({
          kind: 'field' as const,
          key: `${result.doc.id}/${hit.field.key}`,
          result,
          field: hit.field,
        })),
      ]),
    [results],
  )
  const icons = useMemo(() => {
    const map = new Map<string, LucideIcon>([[GENERAL_SEARCH_ID, Settings2]])
    for (const entry of entries) map.set(entry.id, entry.icon)
    return map
  }, [entries])

  const [active, setActive] = useState(0)
  // A new result set starts on its best match.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset per result set.
  useEffect(() => setActive(0), [options])
  const activeOption = options[Math.min(active, options.length - 1)]
  const optionId = (index: number) => `${listId}-option-${index}`

  // `/` focuses the search from anywhere in settings that is not a field.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey)
        return
      const target = event.target as HTMLElement | null
      if (
        target?.closest(
          'input, textarea, select, [contenteditable=""], [contenteditable="true"]',
        )
      )
        return
      const input = searchRef.current
      if (!input?.isConnected) return
      event.preventDefault()
      input.focus()
      input.select()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  const choose = (option: SearchOption | undefined) => {
    if (!option) return
    onSelectResult(
      option.result.doc.id,
      option.kind === 'field' ? option.field : undefined,
    )
  }
  const clear = () => {
    setQuery('')
    searchRef.current?.focus()
  }

  return (
    <aside
      className={cn(
        'flex min-h-0 flex-col bg-neutral-50 dark:bg-black',
        narrow
          ? 'min-w-0 flex-1'
          : 'w-[250px] shrink-0 border-r border-neutral-200 dark:border-neutral-800',
      )}
      aria-label="Settings navigation"
    >
      <div className="shrink-0 px-3 pt-3">
        {onClose ? (
          <div className="flex justify-end">
            <button
              type="button"
              onClick={onClose}
              className="inline-flex h-8 max-w-full items-center gap-1.5 rounded-md px-2 text-[15px] text-muted-foreground transition-colors duration-150 ease-out outline-none hover:bg-neutral-200/60 hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11 dark:hover:bg-neutral-900"
            >
              <ArrowLeft className="size-4 shrink-0" aria-hidden />
              Back
            </button>
          </div>
        ) : (
          <h1 className="px-2 text-sm font-medium leading-8 text-foreground">
            Settings
          </h1>
        )}
        <div className="relative mt-3">
          <Search
            className="pointer-events-none absolute left-2.5 top-1/2 size-4 shrink-0 -translate-y-1/2 text-neutral-400 dark:text-neutral-500"
            aria-hidden
          />
          <Input
            ref={searchRef}
            type="search"
            name="settings-search"
            role="combobox"
            aria-expanded={searching && options.length > 0}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={
              searching && activeOption
                ? optionId(options.indexOf(activeOption))
                : undefined
            }
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search settings"
            aria-label="Search settings"
            aria-keyshortcuts="/"
            autoComplete="off"
            spellCheck={false}
            className="h-9 rounded-md border-neutral-200 bg-white pl-8 pr-8 text-[15px] shadow-none placeholder:text-neutral-400 pointer-coarse:h-11 dark:border-neutral-800 dark:bg-neutral-900/60 dark:placeholder:text-neutral-500 [&::-webkit-search-cancel-button]:hidden"
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query) {
                // Clear first; a second Escape closes settings.
                event.preventDefault()
                event.stopPropagation()
                setQuery('')
                return
              }
              if (!searching || options.length === 0) return
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                event.preventDefault()
                const step = event.key === 'ArrowDown' ? 1 : -1
                setActive(
                  (current) =>
                    (current + step + options.length) % options.length,
                )
              } else if (event.key === 'Enter') {
                event.preventDefault()
                choose(activeOption)
              }
            }}
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear settings search"
              onClick={clear}
              className="absolute right-1 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
            >
              <span
                className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <X className="size-4" aria-hidden />
            </button>
          ) : (
            <kbd
              className="pointer-events-none absolute right-2 top-1/2 flex h-5 min-w-5 -translate-y-1/2 items-center justify-center rounded border border-neutral-200 bg-neutral-50 px-1 font-inter text-xs text-neutral-400 pointer-coarse:hidden dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-500"
              aria-hidden
            >
              /
            </kbd>
          )}
        </div>
      </div>

      <nav
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pt-5 pb-6"
        aria-label={searching ? 'Settings search results' : 'Settings sections'}
      >
        {searching ? (
          isLoading ? (
            <NavGroup label="Results">
              <NavigationLoading />
            </NavGroup>
          ) : options.length > 0 ? (
            <NavGroup
              label="Results"
              aside={
                <span
                  className="tabular-nums text-neutral-400 dark:text-neutral-500"
                  aria-live="polite"
                  aria-atomic="true"
                >
                  <span className="sr-only">
                    {results.length === 1
                      ? '1 page'
                      : `${results.length} pages`}{' '}
                    match
                  </span>
                  <span aria-hidden>{results.length}</span>
                </span>
              }
              listbox={{ id: listId, label: 'Settings search results' }}
            >
              {options.map((option, index) => {
                const isActive = option === activeOption
                const docId = option.result.doc.id
                const current =
                  docId === GENERAL_SEARCH_ID
                    ? generalSelected
                    : selectedId === docId
                const more =
                  option.kind === 'page'
                    ? option.result.fields.length - FIELDS_PER_RESULT
                    : 0
                return option.kind === 'page' ? (
                  <ResultRow
                    key={option.key}
                    id={optionId(index)}
                    active={isActive}
                    selected={current}
                    onChoose={() => choose(option)}
                    onHover={() => setActive(index)}
                  >
                    <PageIcon
                      icon={icons.get(docId) ?? Settings2}
                      selected={current}
                    />
                    <span className="min-w-0 flex-1 truncate">
                      <Highlight
                        text={
                          entries.find((entry) => entry.id === docId)?.label ??
                          option.result.doc.name
                        }
                        query={deferredQuery}
                      />
                    </span>
                    {more > 0 ? (
                      <span className="shrink-0 text-sm tabular-nums text-neutral-400 dark:text-neutral-500">
                        +{more}
                      </span>
                    ) : isActive ? (
                      <ReturnHint />
                    ) : null}
                  </ResultRow>
                ) : (
                  <ResultRow
                    key={option.key}
                    id={optionId(index)}
                    active={isActive}
                    selected={false}
                    field
                    onChoose={() => choose(option)}
                    onHover={() => setActive(index)}
                  >
                    {/* The setting first, its section after: a narrow
                        sidebar truncates the trail, never the name. */}
                    <span
                      className="min-w-0 flex-1 truncate"
                      title={[...option.field.trail, option.field.label].join(
                        ' › ',
                      )}
                    >
                      <Highlight
                        text={option.field.label}
                        query={deferredQuery}
                      />
                      {option.field.trail.length > 0 ? (
                        <span className="text-neutral-400 dark:text-neutral-500">
                          {' · '}
                          {option.field.trail.join(' › ')}
                        </span>
                      ) : null}
                    </span>
                    {isActive ? <ReturnHint /> : null}
                  </ResultRow>
                )
              })}
            </NavGroup>
          ) : (
            <SearchEmptyState
              query={query.trim()}
              suggestion={suggestion}
              onSuggest={(next) => {
                setQuery(next)
                searchRef.current?.focus()
              }}
              onClear={clear}
            />
          )
        ) : (
          <>
            <NavGroup label="ADE">
              <NavItem
                icon={Settings2}
                label="General"
                selected={generalSelected}
                onClick={onSelectGeneral}
              />
            </NavGroup>

            <NavGroup
              label="Workers"
              aside={
                !isLoading && !isError ? (
                  <span className="tabular-nums text-neutral-400 dark:text-neutral-500">
                    {entries.length}
                  </span>
                ) : null
              }
            >
              {isLoading ? <NavigationLoading /> : null}
              {isError ? (
                <p className="px-2 py-2 text-sm leading-5 text-destructive">
                  {errorMessage ?? 'Could not load worker settings.'}
                </p>
              ) : null}
              {!isLoading && !isError && entries.length === 0 ? (
                <p className="px-2 py-2 text-sm leading-5 text-muted-foreground">
                  No workers with a settings UI are registered.
                </p>
              ) : null}
              {entries.map((entry) => (
                <NavItem
                  key={entry.id}
                  icon={entry.icon}
                  label={entry.label}
                  title={entry.description}
                  selected={selectedId === entry.id}
                  onClick={() => onSelectWorker(entry.id)}
                />
              ))}
            </NavGroup>
          </>
        )}
      </nav>
    </aside>
  )
}

function NavGroup({
  label,
  aside,
  listbox,
  children,
}: {
  label: string
  aside?: ReactNode
  /** Render the rows as the search box's listbox instead of a list. */
  listbox?: { id: string; label: string }
  children: ReactNode
}) {
  return (
    <div className="mb-6">
      <h2 className="flex items-baseline gap-2 px-2 pb-1 text-sm font-medium leading-5 text-muted-foreground">
        {label}
        {aside}
      </h2>
      {listbox ? (
        <div
          id={listbox.id}
          role="listbox"
          aria-label={listbox.label}
          className="flex flex-col gap-0.5"
        >
          {children}
        </div>
      ) : (
        // biome-ignore lint/a11y/noRedundantRoles: list-none removes native list semantics in Safari/VoiceOver.
        <ul role="list" className="flex list-none flex-col gap-0.5">
          {children}
        </ul>
      )}
    </div>
  )
}

interface NavItemProps {
  icon: LucideIcon
  label: string
  title?: string
  selected: boolean
  onClick: () => void
}

const ROW_BASE =
  'flex w-full items-center gap-2.5 rounded-md px-2 text-left leading-5 outline-none transition-colors duration-100 ease-out select-none'

function NavItem({
  icon: Icon,
  label,
  title,
  selected,
  onClick,
}: NavItemProps) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        title={title}
        aria-current={selected ? 'page' : undefined}
        className={cn(
          ROW_BASE,
          'h-9 text-[15px] focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11',
          selected
            ? 'bg-neutral-200/70 text-foreground dark:bg-neutral-800/70'
            : 'text-neutral-600 hover:bg-neutral-200/50 hover:text-foreground dark:text-neutral-400 dark:hover:bg-neutral-900',
        )}
      >
        <PageIcon icon={Icon} selected={selected} />
        <span className="min-w-0 flex-1 truncate">{label}</span>
      </button>
    </li>
  )
}

function PageIcon({
  icon: Icon,
  selected,
}: {
  icon: LucideIcon
  selected: boolean
}) {
  return (
    <Icon
      className={cn(
        'size-4 shrink-0',
        selected ? 'text-foreground' : 'text-neutral-500 dark:text-neutral-500',
      )}
      aria-hidden
    />
  )
}

/**
 * One result: a listbox option driven from the search box (arrows move the
 * active row, Enter opens it), still a plain click target for the pointer.
 * Field rows sit indented under their page, in the body size.
 */
function ResultRow({
  id,
  active,
  selected,
  field = false,
  onChoose,
  onHover,
  children,
}: {
  id: string
  active: boolean
  selected: boolean
  field?: boolean
  onChoose: () => void
  onHover: () => void
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: 'nearest' })
  }, [active])
  return (
    <div
      ref={ref}
      id={id}
      role="option"
      tabIndex={-1}
      aria-selected={active}
      aria-current={selected ? 'page' : undefined}
      // Keep focus in the search box, so typing can carry on after a click.
      onPointerDown={(event) => event.preventDefault()}
      onPointerMove={onHover}
      onClick={onChoose}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return
        event.preventDefault()
        onChoose()
      }}
      className={cn(
        ROW_BASE,
        'cursor-default',
        field
          ? 'h-8 pl-[34px] text-sm pointer-coarse:h-10'
          : 'h-9 text-[15px] pointer-coarse:h-11',
        selected
          ? 'bg-neutral-200/70 text-foreground dark:bg-neutral-800/70'
          : active
            ? 'bg-neutral-200/50 text-foreground dark:bg-neutral-900'
            : 'text-neutral-600 dark:text-neutral-400',
      )}
    >
      {children}
    </div>
  )
}

function ReturnHint() {
  return (
    <CornerDownLeft
      className="size-4 shrink-0 text-neutral-400 pointer-coarse:hidden dark:text-neutral-500"
      aria-hidden
    />
  )
}

/** `text` with the query's word-start matches set in the strong ink. */
function Highlight({ text, query }: { text: string; query: string }) {
  const ranges = highlightRanges(text, query)
  if (ranges.length === 0) return <>{text}</>
  const parts: ReactNode[] = []
  let at = 0
  for (const [start, end] of ranges) {
    if (start > at) parts.push(text.slice(at, start))
    parts.push(
      <mark key={start} className="bg-transparent font-medium text-foreground">
        {text.slice(start, end)}
      </mark>,
    )
    at = end
  }
  if (at < text.length) parts.push(text.slice(at))
  return <>{parts}</>
}

/**
 * Nothing matched: say so in the query's own words, offer the nearest
 * spelling when there is one, and give the way back.
 */
function SearchEmptyState({
  query,
  suggestion,
  onSuggest,
  onClear,
}: {
  query: string
  suggestion: string | null
  onSuggest: (next: string) => void
  onClear: () => void
}) {
  return (
    <div
      className="flex flex-col items-center px-3 pt-8 pb-4 text-center"
      role="status"
    >
      <div className="mb-4 flex size-11 items-center justify-center rounded-xl border border-neutral-200 bg-white shadow-xs dark:border-neutral-800 dark:bg-neutral-900">
        <SearchX
          className="size-5 text-neutral-500 dark:text-neutral-400"
          aria-hidden
        />
      </div>
      <p className="max-w-full text-[15px] font-medium leading-5 text-foreground [overflow-wrap:anywhere]">
        No results for “{query}”
      </p>
      {suggestion ? (
        <p className="mt-1.5 text-sm leading-5 text-muted-foreground">
          Did you mean{' '}
          <button
            type="button"
            onClick={() => onSuggest(suggestion)}
            className="rounded-sm font-medium text-foreground underline decoration-neutral-300 underline-offset-2 outline-none hover:decoration-foreground focus-visible:ring-3 focus-visible:ring-ring/50 dark:decoration-neutral-700"
          >
            {suggestion}
          </button>
          ?
        </p>
      ) : (
        <p className="mt-1.5 text-pretty text-sm leading-5 text-muted-foreground">
          Search matches page names and the settings on them, like “theme” or
          “port”.
        </p>
      )}
      <Button
        type="button"
        variant="outline"
        className="mt-4"
        onClick={onClear}
      >
        Clear search
      </Button>
    </div>
  )
}

function NavigationLoading() {
  return (
    <>
      {[0, 1, 2].map((item) => (
        <li key={item} className="flex h-9 items-center gap-2.5 px-2">
          <Skeleton className="size-4 shrink-0" />
          <Skeleton className="h-4 w-32" />
        </li>
      ))}
    </>
  )
}
