/* One selection engine for the Git window's lists and trees, the way
   WebStorm's work. The list itself holds focus and names its active row
   through aria-activedescendant: the log is windowed, so its rows cannot
   each take focus.

   A click selects and a double click or Enter acts. The arrows, Home/End
   and the page keys move. Delete (or Mod+Backspace) deletes. Shift+F10 or
   the menu key opens the row's menu. Typing a few letters jumps to a row
   holding them (speed search): the arrows then step through the matches,
   Backspace edits and Esc clears. */

import { type KeyboardEvent, type MouseEvent, useCallback, useEffect, useRef, useState } from 'react'
import type { ContextMenuAnchor } from './ContextMenu'

export interface RowNavOptions<T> {
  items: readonly T[]
  idOf(item: T): string
  /** What speed search matches. */
  labelOf(item: T): string
  /** DOM id prefix: row `i` is `${domId}-${i}`. */
  domId: string
  selected: string | null
  onSelect(id: string | null): void
  onAct?(item: T): void
  onDelete?(item: T): void
  onMenu?(item: T, anchor: ContextMenuAnchor): void
  /** A click on a row, after it is selected. */
  onClickRow?(item: T): void
  /** Keys a list adds (a tree's ← and →): true when it handled the key. */
  onKey?(event: KeyboardEvent<HTMLElement>, item: T, index: number): boolean
  /** Brings row `index` into view: a windowed list scrolls itself there. */
  reveal?(index: number): void
  /** Rows a page key moves. */
  pageSize?: number
  /** A selected row that leaves `items` was deleted: select its neighbour.
      False where rows only hide (a closed group, a filter) or where the
      owner re-picks. Default true. */
  handOff?: boolean
}

export interface RowNav {
  /** The active row's index, or -1. */
  activeIndex: number
  /** The speed-search text, '' when none. */
  query: string
  listProps: {
    tabIndex: 0
    'aria-activedescendant': string | undefined
    onKeyDown(event: KeyboardEvent<HTMLElement>): void
    onBlur(): void
  }
  rowProps(index: number): {
    id: string
    'aria-selected': boolean
    onClick(event: MouseEvent<HTMLElement>): void
    onDoubleClick(event: MouseEvent<HTMLElement>): void
    onContextMenu(event: MouseEvent<HTMLElement>): void
  }
  /** rowProps' mouse handlers, held once by an element around the rows:
      each finds its row by the row's id. Memoized rows then take no
      functions, and one selection re-renders two rows, not all of them. */
  rowEvents: {
    onClick(event: MouseEvent<HTMLElement>): void
    onDoubleClick(event: MouseEvent<HTMLElement>): void
    onContextMenu(event: MouseEvent<HTMLElement>): void
  }
}

const MENU_KEY = 'ContextMenu'

export function useRowNav<T>(options: RowNavOptions<T>): RowNav {
  const { items, idOf, labelOf, domId, selected, onSelect, onAct, onDelete, onMenu, onClickRow, onKey, reveal } =
    options
  const pageSize = options.pageSize ?? 10
  const handOff = options.handOff ?? true
  const [query, setQuery] = useState('')
  const activeIndex = selected === null ? -1 : items.findIndex((item) => idOf(item) === selected)
  // Where the selection sat, so a row that goes away (removed, merged)
  // hands it to its neighbour rather than to nothing.
  const lastIndex = useRef(-1)
  if (activeIndex >= 0) lastIndex.current = activeIndex
  const revealNext = useRef(false)

  useEffect(() => {
    if (!handOff || selected === null || activeIndex >= 0 || items.length === 0 || lastIndex.current < 0) return
    onSelect(idOf(items[Math.min(lastIndex.current, items.length - 1)]))
  }, [handOff, selected, activeIndex, items, idOf, onSelect])

  useEffect(() => {
    if (!revealNext.current || activeIndex < 0) return
    revealNext.current = false
    if (reveal) reveal(activeIndex)
    else document.getElementById(`${domId}-${activeIndex}`)?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex, domId, reveal])

  const moveTo = useCallback(
    (index: number) => {
      if (items.length === 0) return
      const next = Math.max(0, Math.min(items.length - 1, index))
      revealNext.current = true
      onSelect(idOf(items[next]))
    },
    [items, idOf, onSelect],
  )

  const matchFrom = (text: string, start: number, step: 1 | -1): number => {
    const needle = text.toLowerCase()
    for (let n = 0; n < items.length; n += 1) {
      const index = (((start + n * step) % items.length) + items.length) % items.length
      if (labelOf(items[index]).toLowerCase().includes(needle)) return index
    }
    return -1
  }

  // A row scrolled out of view (or out of a windowed list's DOM) scrolls
  // back while its menu opens at the list's top.
  const menuAt = (index: number, list: HTMLElement) => {
    const item = items[index]
    if (item === undefined || !onMenu) return
    const bounds = list.getBoundingClientRect()
    const rect = document.getElementById(`${domId}-${index}`)?.getBoundingClientRect()
    if (rect !== undefined && rect.bottom > bounds.top && rect.top < bounds.bottom) {
      onMenu(item, { x: rect.left + 16, y: rect.bottom })
      return
    }
    reveal?.(index)
    onMenu(item, { x: bounds.left + 16, y: bounds.top + 16 })
  }

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    // Keys typed into a field inside the list (an inline form) are its own.
    const target = event.target as HTMLElement
    if (target !== event.currentTarget && target.closest('input, textarea, select, [contenteditable="true"]')) return
    const item = activeIndex >= 0 ? items[activeIndex] : undefined
    if (item !== undefined && onKey?.(event, item, activeIndex)) return
    const mod = event.metaKey || event.ctrlKey
    const handled = () => {
      event.preventDefault()
      event.stopPropagation()
    }
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        handled()
        const step = event.key === 'ArrowDown' ? 1 : -1
        if (query !== '') {
          const hit = matchFrom(query, activeIndex + step, step)
          if (hit >= 0) moveTo(hit)
        } else {
          moveTo(activeIndex < 0 ? 0 : activeIndex + step)
        }
        return
      }
      case 'Home':
        handled()
        moveTo(0)
        return
      case 'End':
        handled()
        moveTo(items.length - 1)
        return
      case 'PageDown':
        handled()
        moveTo(activeIndex + pageSize)
        return
      case 'PageUp':
        handled()
        moveTo(activeIndex - pageSize)
        return
      case 'Enter':
        if (item === undefined || !onAct || mod) return
        handled()
        onAct(item)
        return
      case 'Delete':
        if (item === undefined || !onDelete) return
        handled()
        onDelete(item)
        return
      case 'Escape':
        if (query === '') return
        handled()
        setQuery('')
        return
      case 'Backspace':
        if (mod && item !== undefined && onDelete) {
          handled()
          onDelete(item)
        } else if (query !== '') {
          handled()
          setQuery(query.slice(0, -1))
        }
        return
      default:
    }
    if (event.key === MENU_KEY || (event.key === 'F10' && event.shiftKey)) {
      handled()
      if (activeIndex >= 0) menuAt(activeIndex, event.currentTarget)
      return
    }
    // A printable key starts or extends the speed search.
    if (event.key.length === 1 && !mod && !event.altKey && !(event.key === ' ' && query === '')) {
      handled()
      const next = query + event.key
      setQuery(next)
      const hit = matchFrom(next, Math.max(activeIndex, 0), 1)
      if (hit >= 0) moveTo(hit)
    }
  }

  const click = (index: number) => {
    const item = items[index]
    if (item === undefined) return
    onSelect(idOf(item))
    onClickRow?.(item)
  }
  const doubleClick = (index: number) => {
    const item = items[index]
    if (item !== undefined) onAct?.(item)
  }
  const contextMenu = (index: number, event: MouseEvent<HTMLElement>) => {
    const item = items[index]
    if (item === undefined || !onMenu) return
    event.preventDefault()
    onSelect(idOf(item))
    onMenu(item, { x: event.clientX, y: event.clientY })
  }
  // The row an event came from, by the id rowProps gives it; -1 off the rows.
  const rowOf = (event: MouseEvent<HTMLElement>): number => {
    for (let node = event.target as HTMLElement | null; node !== null; node = node.parentElement) {
      if (node.id.startsWith(`${domId}-`)) return Number(node.id.slice(domId.length + 1))
      if (node === event.currentTarget) break
    }
    return -1
  }

  return {
    activeIndex,
    query,
    listProps: {
      tabIndex: 0,
      'aria-activedescendant': activeIndex >= 0 ? `${domId}-${activeIndex}` : undefined,
      onKeyDown,
      onBlur: () => setQuery(''),
    },
    rowProps: (index) => ({
      id: `${domId}-${index}`,
      'aria-selected': index === activeIndex,
      onClick: () => click(index),
      onDoubleClick: () => doubleClick(index),
      onContextMenu: (event) => contextMenu(index, event),
    }),
    rowEvents: {
      onClick: (event) => click(rowOf(event)),
      onDoubleClick: (event) => doubleClick(rowOf(event)),
      onContextMenu: (event) => contextMenu(rowOf(event), event),
    },
  }
}

/** `label` with the speed-search `query` marked, for a row's text. */
export function speedMarks(label: string, query: string): Array<{ text: string; hit: boolean }> {
  if (query === '') return [{ text: label, hit: false }]
  const at = label.toLowerCase().indexOf(query.toLowerCase())
  if (at < 0) return [{ text: label, hit: false }]
  return [
    { text: label.slice(0, at), hit: false },
    { text: label.slice(at, at + query.length), hit: true },
    { text: label.slice(at + query.length), hit: false },
  ].filter((part) => part.text !== '')
}
