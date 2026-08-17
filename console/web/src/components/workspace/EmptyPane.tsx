import { Search, X } from 'lucide-react'
import {
  type KeyboardEvent,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'
import type { TabScreen } from '@/lib/workspace-tabs'
import { filterScreenOptions } from './empty-pane-search'
import type { ScreenOption } from './use-screen-options'

interface EmptyPaneProps {
  screenOptions: ScreenOption[]
  onAttach: (screen: TabScreen) => void
  /** Present when the column can be dropped (multi-column tabs only). */
  onRemove?: () => void
}

/**
 * An empty workspace panel is already a page-selection context, so the
 * available pages stay visible instead of hiding behind a dropdown. Search
 * and keyboard selection keep the same surface useful with dozens of worker-
 * injected pages; the list alone scrolls so the surrounding pane stays put.
 */
export function EmptyPane({
  screenOptions,
  onAttach,
  onRemove,
}: EmptyPaneProps) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const listId = useId()
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([])
  const filteredOptions = useMemo(
    () => filterScreenOptions(screenOptions, query),
    [screenOptions, query],
  )
  const activeOption = filteredOptions[activeIndex] ?? null

  useEffect(() => {
    if (activeIndex >= filteredOptions.length) {
      setActiveIndex(Math.max(0, filteredOptions.length - 1))
    }
  }, [activeIndex, filteredOptions.length])

  useEffect(() => {
    optionRefs.current[activeIndex]?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  const moveActive = (nextIndex: number) => {
    if (filteredOptions.length === 0) return
    setActiveIndex(
      (nextIndex + filteredOptions.length) % filteredOptions.length,
    )
  }

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        moveActive(activeIndex + 1)
        break
      case 'ArrowUp':
        event.preventDefault()
        moveActive(activeIndex - 1)
        break
      case 'Home':
        event.preventDefault()
        moveActive(0)
        break
      case 'End':
        event.preventDefault()
        moveActive(filteredOptions.length - 1)
        break
      case 'Enter':
        if (!activeOption) return
        event.preventDefault()
        onAttach(activeOption.value)
        break
      case 'Escape':
        if (!query) return
        event.preventDefault()
        setQuery('')
        setActiveIndex(0)
        break
    }
  }

  return (
    <div className="@container relative flex min-h-0 flex-1 overflow-hidden">
      {onRemove ? (
        <button
          type="button"
          onClick={onRemove}
          aria-label="remove this panel"
          title="remove this panel"
          className="absolute right-2 top-2 z-10 flex size-7 items-center justify-center rounded-sm stroke-ink-ghost hover:bg-surface-hover hover:stroke-ink focus-visible:outline-2 focus-visible:outline-accent"
        >
          <span
            aria-hidden
            className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          />
          <X aria-hidden className="size-4 shrink-0" />
        </button>
      ) : null}

      <div className="flex min-h-0 w-full flex-1 flex-col items-center justify-center p-4 @md:p-6">
        <div className="flex max-h-full min-h-0 w-full max-w-md flex-col gap-3 text-left">
          <div className="flex shrink-0 flex-col gap-1">
            <h2 className="text-balance font-mono text-base font-medium lowercase text-ink sm:text-sm">
              attach a page
            </h2>
            <p className="hidden text-pretty font-mono text-sm text-ink-faint @md:block">
              Choose what this panel should show.
            </p>
          </div>

          <div className="flex h-10 shrink-0 items-center gap-2 rounded-sm bg-surface px-3 focus-within:outline-2 focus-within:-outline-offset-1 focus-within:outline-rule-focus">
            <Search aria-hidden className="size-4 shrink-0 stroke-ink-ghost" />
            <input
              // biome-ignore lint/a11y/noAutofocus: a newly created empty panel is already a page-selection flow
              autoFocus
              type="search"
              name="page-search"
              value={query}
              onChange={(event) => {
                setQuery(event.currentTarget.value)
                setActiveIndex(0)
              }}
              onKeyDown={onSearchKeyDown}
              placeholder="search pages…"
              aria-label="search pages"
              role="combobox"
              aria-autocomplete="list"
              aria-controls={listId}
              aria-expanded="true"
              aria-activedescendant={
                activeOption ? `${listId}-option-${activeIndex}` : undefined
              }
              className="min-w-0 flex-1 bg-transparent font-mono text-base lowercase text-ink outline-none placeholder:text-ink-ghost sm:text-[0.8125rem]"
            />
          </div>

          <div className="flex min-h-0 flex-col overflow-hidden rounded-sm bg-surface">
            <div className="flex shrink-0 items-center justify-between gap-3 px-3 py-2 font-mono text-[0.6875rem] uppercase tracking-wide text-ink-faint">
              <div>{query ? 'results' : 'all pages'}</div>
              <div
                aria-live="polite"
                className="shrink-0 tabular-nums text-ink-ghost"
              >
                {filteredOptions.length}
              </div>
            </div>

            <div
              id={listId}
              role="listbox"
              aria-label="available pages"
              className="min-h-0 max-h-72 overflow-y-auto p-1"
            >
              {filteredOptions.length > 0 ? (
                filteredOptions.map((option, index) => {
                  const Icon = option.icon
                  const active = index === activeIndex
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
                      onMouseEnter={() => setActiveIndex(index)}
                      onClick={() => onAttach(option.value)}
                      className={cn(
                        'flex w-full items-start gap-2 rounded-sm px-2 py-2 text-left',
                        active
                          ? 'bg-surface-selected'
                          : 'hover:bg-surface-hover',
                      )}
                    >
                      <Icon
                        aria-hidden
                        className={cn(
                          'size-4 shrink-0',
                          active ? 'stroke-accent' : 'stroke-ink-faint',
                        )}
                      />
                      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <span className="truncate font-mono text-base lowercase text-ink sm:text-[0.8125rem]">
                          {option.label}
                        </span>
                        {option.description ? (
                          <span className="hidden truncate font-mono text-sm text-ink-faint @md:inline">
                            {option.description}
                          </span>
                        ) : null}
                      </span>
                    </button>
                  )
                })
              ) : (
                <p className="px-3 py-6 text-center font-mono text-sm text-ink-faint">
                  No pages match “{query}”.
                </p>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
