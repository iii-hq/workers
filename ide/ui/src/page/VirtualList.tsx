/* A fixed-row-height windowed list: only the rows inside the scroll
   viewport (plus a margin) mount. Search results and change lists reach
   thousands of rows; this keeps them at a few dozen DOM nodes. */

import { memo, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from 'react'

export interface VirtualListProps<T> {
  rows: readonly T[]
  rowHeight: number
  /** Rows rendered beyond the viewport on each side. */
  overscan?: number
  renderRow: (row: T, index: number) => ReactNode
  rowKey: (row: T, index: number) => string
  className?: string
  /** Bring this index into view whenever it changes. */
  scrollToIndex?: number | null
  /** Set on the scrolling element; keyboard handlers live on it. */
  role?: string
  'aria-label'?: string
  tabIndex?: number
  onKeyDown?: (event: React.KeyboardEvent<HTMLDivElement>) => void
  listRef?: React.Ref<HTMLDivElement>
  /** The option a listbox's focus sits on while the scroller keeps it. */
  'aria-activedescendant'?: string
  /** The rows mounted now, overscan included: a paged list loads more
      as `last` nears the end. */
  onRangeChange?: (first: number, last: number) => void
  /** A row kept mounted outside the window: the one aria-activedescendant
      names, so assistive tech can still read it. */
  keepIndex?: number | null
}

function VirtualListView<T>({
  rows,
  rowHeight,
  overscan = 8,
  renderRow,
  rowKey,
  className,
  scrollToIndex = null,
  role,
  'aria-label': ariaLabel,
  tabIndex,
  onKeyDown,
  listRef,
  'aria-activedescendant': activeDescendant,
  onRangeChange,
  keepIndex = null,
}: VirtualListProps<T>) {
  const viewportRef = useRef<HTMLDivElement>(null)
  // The first row in view, not the pixel offset: a scroll within a row
  // renders nothing. Likewise the rows the viewport fits, not its height: a
  // dock dragged taller renders only when another row fits.
  const [topRow, setTopRow] = useState(0)
  const [viewRows, setViewRows] = useState(0)
  const rowHeightRef = useRef(rowHeight)
  rowHeightRef.current = rowHeight

  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const measure = () => setViewRows(Math.ceil(el.clientHeight / rowHeightRef.current))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // A new row height (a narrow pane's two-line rows) moves the first row,
  // and changes how many fit.
  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    setTopRow(Math.floor(el.scrollTop / rowHeight))
    setViewRows(Math.ceil(el.clientHeight / rowHeight))
  }, [rowHeight])

  useEffect(() => {
    const el = viewportRef.current
    if (!el || scrollToIndex === null || scrollToIndex < 0) return
    const top = scrollToIndex * rowHeight
    const bottom = top + rowHeight
    if (top < el.scrollTop) el.scrollTop = top
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight
  }, [scrollToIndex, rowHeight])

  const total = rows.length * rowHeight
  const first = Math.max(0, topRow - overscan)
  const last = Math.min(rows.length, topRow + viewRows + 1 + overscan)
  const rangeRef = useRef(onRangeChange)
  rangeRef.current = onRangeChange
  useEffect(() => {
    rangeRef.current?.(first, last)
  }, [first, last])
  const row = (index: number) => (
    <div
      key={rowKey(rows[index], index)}
      className="shui-vrow"
      style={{ transform: `translateY(${index * rowHeight}px)`, height: rowHeight }}
    >
      {renderRow(rows[index], index)}
    </div>
  )
  const visible: ReactNode[] = []
  const kept = keepIndex !== null && keepIndex >= 0 && keepIndex < rows.length ? keepIndex : null
  // A kept row outside the window comes with its neighbours, so Tab and
  // Shift+Tab from it land on the next row rather than leave the list.
  const near =
    kept === null
      ? []
      : [kept - 1, kept, kept + 1].filter(
          (index) => index >= 0 && index < rows.length && (index < first || index >= last),
        )
  for (const index of near) if (index < first) visible.push(row(index))
  for (let index = first; index < last; index++) visible.push(row(index))
  for (const index of near) if (index >= last) visible.push(row(index))

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the scroller carries the caller's role and keyboard handling
    // biome-ignore lint/a11y/useAriaPropsSupportedByRole: the role is the caller's (tree, listbox)
    <div
      ref={(node) => {
        viewportRef.current = node
        if (typeof listRef === 'function') listRef(node)
        else if (listRef) (listRef as React.MutableRefObject<HTMLDivElement | null>).current = node
      }}
      className={className ? `shui-vlist ${className}` : 'shui-vlist'}
      onScroll={(event) => setTopRow(Math.floor(event.currentTarget.scrollTop / rowHeight))}
      role={role}
      aria-label={ariaLabel}
      aria-activedescendant={activeDescendant}
      tabIndex={tabIndex}
      onKeyDown={onKeyDown}
    >
      <div className="shui-vlist-space" style={{ height: total }}>
        {visible}
      </div>
    </div>
  )
}

/** Memoized: a caller whose props keep their identity (`renderRow`, `rowKey`)
    skips re-rendering every mounted row when it re-renders itself. */
export const VirtualList = memo(VirtualListView) as typeof VirtualListView
