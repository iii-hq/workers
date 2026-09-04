/* A fixed-row-height windowed list: only the rows inside the scroll
   viewport (plus a margin) mount. Search results and change lists reach
   thousands of rows; this keeps them at a few dozen DOM nodes. */

import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from 'react'

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
}

export function VirtualList<T>({
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
}: VirtualListProps<T>) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [height, setHeight] = useState(0)

  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const measure = () => setHeight(el.clientHeight)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const el = viewportRef.current
    if (!el || scrollToIndex === null || scrollToIndex < 0) return
    const top = scrollToIndex * rowHeight
    const bottom = top + rowHeight
    if (top < el.scrollTop) el.scrollTop = top
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight
  }, [scrollToIndex, rowHeight])

  const total = rows.length * rowHeight
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan)
  const last = Math.min(rows.length, Math.ceil((scrollTop + height) / rowHeight) + overscan)
  const visible: ReactNode[] = []
  for (let index = first; index < last; index++) {
    visible.push(
      <div
        key={rowKey(rows[index], index)}
        className="shui-vrow"
        style={{ transform: `translateY(${index * rowHeight}px)`, height: rowHeight }}
      >
        {renderRow(rows[index], index)}
      </div>,
    )
  }

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
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      role={role}
      aria-label={ariaLabel}
      tabIndex={tabIndex}
      onKeyDown={onKeyDown}
    >
      <div className="shui-vlist-space" style={{ height: total }}>
        {visible}
      </div>
    </div>
  )
}
