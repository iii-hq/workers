/**
 * The library column: search, a "new" action and the collection's entries
 * in collapsible groups (skills by their first id segment, profiles by
 * where they came from). Rows are one compact two-line button each, so a
 * 170-skill folder scans in a screen or two instead of twenty.
 *
 * Keyboard: `/` (page-wide) focuses search, ↓ from search enters the list,
 * ↑/↓ walk the visible rows, Home/End jump, Enter opens.
 */

import {
  ShadcnButton,
  ShadcnKbd,
  ShadcnSkeleton,
  ShadcnTooltip,
  ShadcnTooltipContent,
  ShadcnTooltipTrigger,
} from '@iii-dev/console-ui'
import { ChevronRight, Plus, Search, SearchX, X } from 'lucide-react'
import type { KeyboardEvent, ReactNode, Ref } from 'react'
import { useMemo, useRef } from 'react'
import type { BrowserRow } from './browser'
import { closestMatches, firstShownRow, groupRows, isGroupCollapsed, matchesQuery } from './library-model'

/** Move focus among the rendered rows (`[data-dir-row]`) by a step, or to the first/last. */
function moveFocus(container: HTMLElement | null, from: Element | null, delta: number | 'first' | 'last') {
  if (!container) return
  const rows = [...container.querySelectorAll<HTMLButtonElement>('[data-dir-row]')]
  if (rows.length === 0) return
  let index = from ? rows.indexOf(from as HTMLButtonElement) : -1
  if (delta === 'first') index = 0
  else if (delta === 'last') index = rows.length - 1
  else index = Math.min(rows.length - 1, Math.max(0, index + delta))
  rows[index]?.focus()
  rows[index]?.scrollIntoView({ block: 'nearest' })
}

/** The library column for one collection: search, the "new" action, and the grouped rows (or a loading, empty, error or no-match state). */
export function Library({
  noun,
  nounPlural,
  rows,
  error,
  onRetry,
  selected,
  onOpen,
  query,
  onQuery,
  searchRef,
  onCreate,
  groupOrder,
  fallbackGroupLabel,
  collapsed,
  onToggleGroup,
  emptyTitle,
  emptyBody,
  createLabel,
  menu,
  errorTitle,
  errorBody,
}: {
  noun: string
  nounPlural: string
  rows: BrowserRow[] | null
  error: string | null
  onRetry: () => void
  selected: string | null
  onOpen: (key: string) => void
  query: string
  onQuery: (next: string) => void
  searchRef: Ref<HTMLInputElement>
  onCreate?: () => void
  groupOrder?: readonly string[]
  fallbackGroupLabel?: string
  collapsed: readonly string[]
  onToggleGroup: (id: string) => void
  emptyTitle: string
  emptyBody: string
  /** The "new" action's label (defaults to `New <noun>`; kits: `Install kit`). */
  createLabel?: string
  /** Section-level ⋯ menu beside the new action (kits: Check for updates). */
  menu?: ReactNode
  /** Load-failure copy (defaults to the worker-running hint). */
  errorTitle?: string
  errorBody?: string
}) {
  const newLabel = createLabel ?? `New ${noun}`
  const plural = nounPlural.toLowerCase()
  const listRef = useRef<HTMLDivElement | null>(null)
  const needle = query.trim().toLowerCase()
  const visible = useMemo(() => (rows ?? []).filter((r) => matchesQuery(r, needle)), [rows, needle])
  const groups = useMemo(
    () => groupRows(visible, groupOrder, fallbackGroupLabel),
    [visible, groupOrder, fallbackGroupLabel],
  )
  // Nothing matched: offer what the query most likely meant.
  const suggestions = useMemo(
    () => (needle && rows && visible.length === 0 ? closestMatches(rows, needle) : []),
    [rows, needle, visible.length],
  )
  // One group needs no header: it would only repeat the section name.
  const showHeaders = groups.length > 1

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      moveFocus(listRef.current, null, 'first')
    } else if (e.key === 'Escape' && query) {
      e.preventDefault()
      onQuery('')
    } else if (e.key === 'Enter') {
      // The row at the top of the list as drawn (not the adapter's order),
      // or the best "did you mean" when nothing matched.
      const first = firstShownRow(groups, { filtering: !!needle, collapsed }) ?? suggestions[0]
      if (!first) return
      e.preventDefault()
      onOpen(first.key)
    }
  }

  const onListKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const from = document.activeElement
    if (e.key === 'ArrowDown') moveFocus(listRef.current, from, 1)
    else if (e.key === 'ArrowUp') {
      const rowsEls = listRef.current?.querySelectorAll('[data-dir-row]')
      if (rowsEls && rowsEls[0] === from) {
        e.preventDefault()
        ;(listRef.current?.closest('.dir-lib')?.querySelector('.dir-lib-search input') as HTMLElement | null)?.focus()
        return
      }
      moveFocus(listRef.current, from, -1)
    } else if (e.key === 'Home') moveFocus(listRef.current, from, 'first')
    else if (e.key === 'End') moveFocus(listRef.current, from, 'last')
    else return
    e.preventDefault()
  }

  let body: ReactNode
  if (error) {
    body = (
      <div className="dir-lib-state" role="alert">
        <p className="dir-lib-state-title">{errorTitle ?? `Unable to load ${plural}.`}</p>
        <p className="dir-lib-state-body">
          {errorBody ?? 'Check that the iii-directory worker is running, then try again.'}
        </p>
        <p className="dir-lib-state-detail">{error}</p>
        <ShadcnButton variant="secondary" size="sm" onClick={onRetry}>
          Try again
        </ShadcnButton>
      </div>
    )
  } else if (rows === null) {
    body = (
      <div className="dir-lib-skeleton" aria-hidden>
        {[62, 48, 70, 55, 40, 66, 52].map((w, i) => (
          <div key={i} className="dir-lib-skeleton-row">
            <ShadcnSkeleton style={{ width: `${w}%`, height: 12 }} />
            <ShadcnSkeleton style={{ width: `${w + 22}%`, height: 10 }} />
          </div>
        ))}
      </div>
    )
  } else if (rows.length === 0) {
    body = (
      <div className="dir-lib-state">
        <p className="dir-lib-state-title">{emptyTitle}</p>
        <p className="dir-lib-state-body">{emptyBody}</p>
        {onCreate ? (
          // Secondary: the document's empty state owns the one primary.
          <ShadcnButton variant="secondary" size="sm" onClick={onCreate}>
            <Plus aria-hidden />
            {newLabel}
          </ShadcnButton>
        ) : null}
      </div>
    )
  } else if (visible.length === 0) {
    const shown = query.trim()
    body = (
      <div className="dir-lib-nomatch">
        <span className="dir-lib-nomatch-mark" aria-hidden>
          <SearchX />
        </span>
        <div role="status">
          <p className="dir-lib-nomatch-title">No {plural} found</p>
          <p className="dir-lib-nomatch-body">
            Nothing matches{' '}
            <span className="dir-lib-nomatch-query">
              <q>{shown}</q>.
            </span>{' '}
            Check the spelling, or search by what it does.
          </p>
        </div>
        <ShadcnButton variant="secondary" size="sm" onClick={() => onQuery('')}>
          Clear search
          <ShadcnKbd aria-hidden>Esc</ShadcnKbd>
        </ShadcnButton>
        {suggestions.length > 0 ? (
          // biome-ignore lint/a11y/noStaticElementInteractions: arrow-key relay between the row buttons inside
          <div className="dir-lib-suggest" ref={listRef} onKeyDown={onListKeyDown}>
            <p className="dir-lib-suggest-label">Did you mean</p>
            <ul className="dir-lib-rows">
              {suggestions.map((row) => (
                <li key={row.key}>
                  {/* Out of their groups, same-named rows need their group to tell apart. */}
                  <LibraryRow
                    row={{ ...row, badge: row.groupLabel ?? (row.group || fallbackGroupLabel) ?? row.badge }}
                    current={row.key === selected}
                    onOpen={onOpen}
                  />
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    )
  } else {
    body = (
      // biome-ignore lint/a11y/noStaticElementInteractions: arrow-key relay between the row buttons inside
      <div ref={listRef} onKeyDown={onListKeyDown}>
        {groups.map((group) => {
          const isCollapsed = isGroupCollapsed(group.id, {
            groupCount: groups.length,
            filtering: !!needle,
            collapsed,
          })
          return (
            <section key={group.id} className="dir-lib-group" aria-label={group.label}>
              {showHeaders ? (
                <button
                  type="button"
                  className="dir-lib-group-head"
                  aria-expanded={!isCollapsed}
                  onClick={() => onToggleGroup(group.id)}
                  disabled={!!needle}
                >
                  <ChevronRight className="dir-lib-group-chevron" aria-hidden />
                  <span className="dir-lib-group-label">{group.label}</span>
                  <span className="dir-lib-group-count">{group.rows.length}</span>
                </button>
              ) : null}
              {isCollapsed ? null : (
                <ul className="dir-lib-rows">
                  {group.rows.map((row) => (
                    <li key={row.key}>
                      <LibraryRow row={row} current={row.key === selected} onOpen={onOpen} />
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )
        })}
      </div>
    )
  }

  return (
    <div className="dir-lib">
      <div className="dir-lib-top">
        <label className="dir-lib-search">
          <Search aria-hidden className="dir-lib-search-icon" />
          <input
            ref={searchRef}
            type="search"
            value={query}
            onChange={(e) => onQuery(e.currentTarget.value)}
            onKeyDown={onSearchKeyDown}
            placeholder={`Search ${plural}`}
            aria-label={`Search ${plural}`}
            spellCheck={false}
            autoComplete="off"
          />
          {query ? (
            <button
              type="button"
              className="dir-lib-search-clear"
              aria-label="Clear search"
              onClick={() => onQuery('')}
            >
              <X aria-hidden />
            </button>
          ) : (
            <ShadcnKbd className="dir-lib-search-kbd">/</ShadcnKbd>
          )}
        </label>
        {onCreate ? (
          <ShadcnTooltip>
            <ShadcnTooltipTrigger
              render={
                <ShadcnButton variant="ghost" size="icon-sm" aria-label={newLabel} onClick={onCreate}>
                  <Plus aria-hidden />
                </ShadcnButton>
              }
            />
            <ShadcnTooltipContent>{newLabel}</ShadcnTooltipContent>
          </ShadcnTooltip>
        ) : null}
        {menu}
      </div>
      {/* Rendered empty first so the count is announced when a query lands;
          the tab already says how many there are in total. */}
      <div className="dir-lib-meta" role="status">
        {needle && rows !== null && !error && visible.length > 0 ? `${visible.length} of ${rows.length}` : null}
      </div>
      <div className="dir-lib-scroll">{body}</div>
    </div>
  )
}

/** One two-line library row: optional glyph, title with a trailing tag, one-line description. */
function LibraryRow({ row, current, onOpen }: { row: BrowserRow; current: boolean; onOpen: (key: string) => void }) {
  // Ids read as data (mono) only when there is no title to show instead.
  const titled = row.title !== ''
  return (
    <button
      type="button"
      data-dir-row
      className="dir-lib-row"
      aria-current={current ? 'true' : undefined}
      onClick={() => onOpen(row.key)}
    >
      {row.icon ? (
        <span className="dir-lib-row-glyph" data-color={row.iconTone} aria-hidden>
          {row.icon}
        </span>
      ) : null}
      <span className="dir-lib-row-text">
        <span className="dir-lib-row-title">
          <span className={titled ? 'dir-lib-row-name' : 'dir-lib-row-name dir-mono'}>
            {titled ? row.title : row.key}
          </span>
          {row.badge ? <span className="dir-lib-row-badge">{row.badge}</span> : null}
        </span>
        {row.description ? <span className="dir-lib-row-desc">{row.description}</span> : null}
      </span>
    </button>
  )
}
