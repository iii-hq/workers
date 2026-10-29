import { Search, X } from 'lucide-react'
import {
  type KeyboardEvent,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/shadcn/button'
import { Hint } from '@/components/ui/Hint'
import { Kbd } from '@/components/ui/Kbd'
import { Wordmark } from '@/components/ui/Wordmark'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { cn } from '@/lib/utils'
import { EXT_SCREEN_PREFIX, type TabScreen } from '@/lib/workspace-tabs'
import { filterScreenOptions } from './empty-pane-search'
import type { ScreenOption } from './use-screen-options'
import '@/components/chat/conversation-design.css'
import './empty-pane.css'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

const DESKTOP_PANE_SEARCH_QUERY = `(min-width: 640px) and ${DESKTOP_POINTER_QUERY}`

interface EmptyPaneProps {
  screenOptions: ScreenOption[]
  onAttach: (screen: TabScreen) => void
  /** Present when the column can be dropped (multi-column tabs only). */
  onRemove?: () => void
}

interface OptionGroup {
  key: string
  label: string
  /** Index of the group's first option in the flat, filtered order. */
  start: number
  options: ScreenOption[]
}

/**
 * Browsing splits the console's own pages from the ones workers inject, so
 * a long tail of worker pages never buries chat and traces. A query drops
 * the grouping: results are one ranked run, best match first.
 */
function groupOptions(options: ScreenOption[], query: string): OptionGroup[] {
  if (query.trim()) {
    return options.length
      ? [{ key: 'results', label: 'Results', start: 0, options }]
      : []
  }
  const consolePages = options.filter(
    (option) => !option.value.startsWith(EXT_SCREEN_PREFIX),
  )
  const workerPages = options.filter((option) =>
    option.value.startsWith(EXT_SCREEN_PREFIX),
  )
  return [
    { key: 'console', label: 'Console', start: 0, options: consolePages },
    {
      key: 'workers',
      label: 'From workers',
      start: consolePages.length,
      options: workerPages,
    },
  ].filter((group) => group.options.length > 0)
}

/**
 * The option one visual row above or below `from` in a wrapping grid: the
 * nearest row in that direction, then the column closest to the current
 * one. Measured from layout, so it follows the container-query column count
 * and steps across group boundaries without knowing either.
 */
function verticalNeighbor(
  elements: Array<HTMLElement | null>,
  from: number,
  direction: 1 | -1,
): number {
  const origin = elements[from]?.getBoundingClientRect()
  if (!origin) return from
  const originCenter = origin.left + origin.width / 2
  let best = from
  let bestRow = Number.POSITIVE_INFINITY
  let bestColumn = Number.POSITIVE_INFINITY
  elements.forEach((element, index) => {
    if (!element || index === from) return
    const rect = element.getBoundingClientRect()
    const row = direction === 1 ? rect.top - origin.top : origin.top - rect.top
    if (row < 1) return
    const column = Math.abs(rect.left + rect.width / 2 - originCenter)
    if (
      row < bestRow - 1 ||
      (Math.abs(row - bestRow) <= 1 && column < bestColumn)
    ) {
      best = index
      bestRow = row
      bestColumn = column
    }
  })
  return best
}

/**
 * An empty workspace panel is a page-selection context, so it opens as a
 * launcher in the chat start screen's language: the inset mark, one
 * question, a composer-shaped search, then every page as a plain row —
 * console pages first, worker pages after. Search and the arrow keys (in
 * both grid directions) keep it quick with dozens of worker pages; when
 * they outgrow the pane, the pane itself scrolls.
 */
export function EmptyPane({
  screenOptions,
  onAttach,
  onRemove,
}: EmptyPaneProps) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  // The first page is the Enter target from the start, but it is only drawn
  // highlighted once a query, an arrow key or the pointer gives it a reason;
  // an untouched launcher shows every row at rest.
  const [navigated, setNavigated] = useState(false)
  const listId = useId()
  const searchRef = useRef<HTMLInputElement>(null)
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([])
  const desktopPaneSearch = useMediaQuery(DESKTOP_PANE_SEARCH_QUERY)
  const filteredOptions = useMemo(
    () => filterScreenOptions(screenOptions, query),
    [screenOptions, query],
  )
  const groups = useMemo(
    () => groupOptions(filteredOptions, query),
    [filteredOptions, query],
  )
  const activeOption = filteredOptions[activeIndex] ?? null
  const showActive = navigated || query.trim() !== ''

  useEffect(() => {
    if (desktopPaneSearch) searchRef.current?.focus()
  }, [desktopPaneSearch])

  useEffect(() => {
    if (activeIndex >= filteredOptions.length) {
      setActiveIndex(Math.max(0, filteredOptions.length - 1))
    }
  }, [activeIndex, filteredOptions.length])

  useEffect(() => {
    optionRefs.current[activeIndex]?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  const updateQuery = (next: string) => {
    setQuery(next)
    setActiveIndex(0)
  }

  const moveActive = (nextIndex: number) => {
    if (filteredOptions.length === 0) return
    setActiveIndex(
      (nextIndex + filteredOptions.length) % filteredOptions.length,
    )
  }

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        event.preventDefault()
        if (filteredOptions.length === 0) return
        setNavigated(true)
        // The first arrow press reveals the highlight where it already is.
        if (!showActive) return
        setActiveIndex(
          verticalNeighbor(
            optionRefs.current.slice(0, filteredOptions.length),
            activeIndex,
            event.key === 'ArrowDown' ? 1 : -1,
          ),
        )
        break
      case 'ArrowRight':
      case 'ArrowLeft':
        // The caret owns left and right while there is text to move through.
        if (query) return
        event.preventDefault()
        setNavigated(true)
        if (!showActive) return
        moveActive(activeIndex + (event.key === 'ArrowRight' ? 1 : -1))
        break
      case 'Home':
      case 'End':
        if (query) return
        event.preventDefault()
        setNavigated(true)
        moveActive(event.key === 'Home' ? 0 : filteredOptions.length - 1)
        break
      case 'Enter':
        if (!activeOption) return
        event.preventDefault()
        onAttach(activeOption.value)
        break
      case 'Escape':
        if (!query) return
        event.preventDefault()
        updateQuery('')
        break
    }
  }

  return (
    <div className="conversation-surface @container relative flex min-h-0 flex-1 overflow-hidden">
      {onRemove ? (
        <Hint label="Remove this panel" side="left">
          <button
            type="button"
            onClick={onRemove}
            aria-label="remove this panel"
            className="absolute right-2 top-2 z-10 flex size-7 items-center justify-center rounded-sm stroke-ink-ghost hover:bg-surface-hover hover:stroke-ink focus-visible:outline-2 focus-visible:outline-accent"
          >
            <span
              aria-hidden
              className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            />
            <X aria-hidden className="size-4 shrink-0" />
          </button>
        </Hint>
      ) : null}

      <div className="flex min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 @2xl:px-6">
        {/* Anchored to the top, not centred: a centred column would slide
            the search box down under the caret as a query shrinks the list. */}
        <div className="mx-auto flex h-fit w-full max-w-[46rem] flex-col gap-7 pt-10 pb-12 @2xl:pt-16">
          <div className="flex flex-col items-center gap-2.5 text-center">
            <Wordmark
              appearance="inset"
              className="mb-1.5 size-9 fill-ink-faint/60"
            />
            <h2 className="text-balance text-2xl font-medium tracking-tight text-ink @2xl:text-[28px]">
              What should this panel show?
            </h2>
            <p className="max-w-md text-pretty text-sm leading-relaxed text-ink-faint">
              Open a console page, or one your workers add.
            </p>
          </div>

          <label className="empty-pane-search flex h-12 w-full items-center gap-3.5 px-3">
            <Search aria-hidden className="size-4 shrink-0 text-ink-faint" />
            <input
              ref={searchRef}
              type="search"
              name="page-search"
              value={query}
              onChange={(event) => updateQuery(event.currentTarget.value)}
              onKeyDown={onSearchKeyDown}
              // This box opens focused, so without these the workspace keys
              // spell themselves into the query instead of moving anywhere.
              data-keybindings-allow="workspace.selectByIndex panel.split panel.splitLeft"
              placeholder="Search pages"
              aria-label="search pages"
              autoComplete="off"
              role="combobox"
              aria-autocomplete="list"
              aria-controls={listId}
              aria-expanded="true"
              aria-activedescendant={
                activeOption && showActive
                  ? `${listId}-option-${activeIndex}`
                  : undefined
              }
              className="min-w-0 flex-1 bg-transparent text-base text-ink outline-none placeholder:text-ink-ghost sm:text-sm"
            />
            {desktopPaneSearch && activeOption && showActive ? (
              <span
                aria-hidden
                className="flex shrink-0 items-center gap-1.5 text-xs text-ink-ghost"
              >
                <Kbd>↵</Kbd>
                open
              </span>
            ) : null}
          </label>

          <div
            id={listId}
            role="listbox"
            aria-label="available pages"
            className="flex flex-col gap-5"
          >
            {groups.map((group) => (
              // biome-ignore lint/a11y/useSemanticElements: ARIA listbox groups are not form fieldsets
              <div
                key={group.key}
                role="group"
                aria-labelledby={`${listId}-${group.key}`}
                className="flex flex-col gap-1"
              >
                <div
                  id={`${listId}-${group.key}`}
                  className="flex items-baseline gap-2 px-3 text-[13px] font-medium text-ink-faint"
                >
                  {group.label}
                  <span aria-hidden className="tabular-nums text-ink-ghost">
                    {group.options.length}
                  </span>
                </div>
                <div
                  role="presentation"
                  className="grid grid-cols-1 gap-x-4 gap-y-px @2xl:grid-cols-2"
                >
                  {group.options.map((option, offset) => {
                    const index = group.start + offset
                    const Icon = option.icon
                    const active = showActive && index === activeIndex
                    return (
                      <button
                        key={option.value}
                        ref={(element) => {
                          optionRefs.current[index] = element
                        }}
                        id={`${listId}-option-${index}`}
                        type="button"
                        role="option"
                        tabIndex={-1}
                        aria-selected={active}
                        // Pointer motion, not enter: rows the arrow keys
                        // scroll under a resting cursor keep the keyboard's
                        // highlight.
                        onPointerMove={() => {
                          if (active) return
                          setNavigated(true)
                          setActiveIndex(index)
                        }}
                        onClick={() => onAttach(option.value)}
                        className="flex min-h-12 w-full min-w-0 items-center gap-3.5 rounded-lg px-3 py-1.5 text-left transition-[background-color] duration-(--motion-duration-fast) ease-(--motion-ease-standard) active:bg-surface-active aria-selected:bg-surface-hover focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-rule-focus"
                      >
                        <Icon
                          aria-hidden
                          className={cn(
                            'size-4 shrink-0 transition-[color] duration-(--motion-duration-fast) ease-(--motion-ease-standard)',
                            active ? 'text-ink' : 'text-ink-faint',
                          )}
                        />
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                          <span className="break-words text-sm/5 font-medium text-ink">
                            {option.label}
                          </span>
                          {option.description ? (
                            <span className="line-clamp-2 text-[13px]/[18px] text-ink-faint">
                              {option.description}
                            </span>
                          ) : null}
                        </span>
                      </button>
                    )
                  })}
                </div>
              </div>
            ))}

            {filteredOptions.length === 0 ? (
              <div className="flex flex-col items-center gap-3 py-4 text-center">
                <p className="text-pretty text-sm text-ink-faint">
                  No pages match “{query}”.
                </p>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    updateQuery('')
                    searchRef.current?.focus()
                  }}
                >
                  Clear search
                </Button>
              </div>
            ) : null}
          </div>

          <p aria-live="polite" className="sr-only">
            {query
              ? `${filteredOptions.length} ${filteredOptions.length === 1 ? 'page' : 'pages'}`
              : ''}
          </p>
        </div>
      </div>
    </div>
  )
}
