import {
  defaultRangeExtractor,
  observeElementRect,
  type Range,
  type Rect,
  type Virtualizer,
} from '@tanstack/react-virtual'
import { type ReactNode, type RefObject, useEffect, useState } from 'react'
import {
  type TimelineActivityGroupRow,
  type TimelineMessageListRow,
  timelineActivityPresentationKey,
} from '@/components/observer-activity/grouping'
import type { AssistantMessage, Message } from '@/types/chat'

/** A chat this short keeps every row mounted: windowing it saves nothing. */
export const WINDOW_MIN_ROWS = 20
/** Rows kept mounted past each edge of the viewport. */
export const WINDOW_OVERSCAN_ROWS = 6
/** Rows mounted at the tail while there is no scroll offset to compute from. */
export const WINDOW_OPENING_ROWS = 12
/**
 * Slot of a row that has never been measured. It sizes the spacers, so it only
 * moves the scrollbar: a row's real height replaces it as soon as it mounts.
 */
export const estimateRowSlot = () => 320

/** One direct child of the transcript stack. Windowing counts these. */
export type FlatTimelineRow =
  | {
      kind: 'activity-group'
      key: string
      row: TimelineActivityGroupRow
      staggerIndex: number
    }
  | {
      kind: 'summary'
      key: string
      message: AssistantMessage
      staggerIndex: number
    }
  | { kind: 'message'; key: string; message: Message; staggerIndex: number }

/**
 * `knownKeys` are the keys already in use. A group is keyed by its first item,
 * so an older page merging into the head batch would rename it; it keeps the
 * key it is mounted and measured under instead.
 */
export function flattenTimelineRows(
  rows: readonly TimelineMessageListRow[],
  settlingThoughtIds: ReadonlySet<string>,
  knownKeys: ReadonlySet<string> = new Set(),
): FlatTimelineRow[] {
  const flat: FlatTimelineRow[] = []
  rows.forEach((row, rowIndex) => {
    if (row.kind === 'activity-group') {
      flat.push({
        kind: 'activity-group',
        key:
          row.items
            .map(timelineActivityPresentationKey)
            .find((key) => knownKeys.has(key)) ?? row.id,
        row,
        staggerIndex: rowIndex,
      })
      // Summaries remain direct children of the transcript stack. If late
      // protocol data reclassifies existing prose as a phase summary, the
      // stable key keeps its Message instance alive.
      if (row.summary) {
        flat.push({
          kind: 'summary',
          key: row.summary.id,
          message: row.summary,
          staggerIndex: rowIndex + 1,
        })
      }
      return
    }
    const message = row.message
    if (
      message.role === 'thought' &&
      !message.streaming &&
      !settlingThoughtIds.has(message.id)
    ) {
      return
    }
    flat.push({
      kind: 'message',
      key: message.id,
      message,
      staggerIndex: rowIndex,
    })
  })
  return flat
}

/** Assistant prose: what the sibling spacing around a batch keys off. */
export function isProseRow(flat: FlatTimelineRow | undefined): boolean {
  return (
    flat?.kind === 'summary' ||
    (flat?.kind === 'message' && flat.message.role === 'assistant')
  )
}

/**
 * A row still changing on its own. It stays mounted wherever the reader is:
 * unmounting one would drop an in-flight approval (the keyboard commands read
 * those from the DOM) or a reply being written out. An ask card counts for as
 * long as it exists: it only knows it was answered while mounted, and remounted
 * it would offer its form, and a stale answer, again.
 */
export function isLiveRow(flat: FlatTimelineRow): boolean {
  if (flat.kind === 'activity-group') {
    return flat.row.items.some(
      (item) =>
        item.kind === 'function-call' &&
        (item.message.running === true ||
          item.message.pendingApproval === true ||
          item.message.functionId === 'harness::ask'),
    )
  }
  const { message } = flat
  return (
    message.role === 'thought' ||
    (message.role === 'assistant' && message.streaming === true)
  )
}

/**
 * Every transcript entry id to the row showing it. A batch shows its calls, and
 * a call that absorbed its wake notification shows that entry too.
 */
export function rowIndexByEntryId(
  flatRows: readonly FlatTimelineRow[],
): Map<string, number> {
  const indexes = new Map<string, number>()
  flatRows.forEach((flat, index) => {
    indexes.set(flat.key, index)
    if (flat.kind !== 'activity-group') return
    for (const item of flat.row.items) {
      indexes.set(item.message.id, index)
      if (item.kind === 'observer-activity' && item.notification) {
        indexes.set(item.notification.id, index)
      }
    }
  })
  return indexes
}

/**
 * Whether the agent has moved past the batch at `index`. Its calls alone cannot
 * say: between one call finishing and the next starting, none is running, and a
 * thought is the agent working out that next call. The turn ending, or anything
 * else coming after the batch, can.
 */
export function agentMovedPast(
  flatRows: readonly FlatTimelineRow[],
  index: number,
  turnActive: boolean,
): boolean {
  if (!turnActive) return true
  for (let next = index + 1; next < flatRows.length; next++) {
    const flat = flatRows[next]
    if (flat.kind !== 'message' || flat.message.role !== 'thought') return true
  }
  return false
}

/** Older history landed above `previousFirstId`, which is still in the list. */
export function isHistoryPrepend(
  previousFirstId: string | undefined,
  messages: readonly Message[],
): boolean {
  return (
    previousFirstId !== undefined &&
    messages[0]?.id !== previousFirstId &&
    messages.some((message) => message.id === previousFirstId)
  )
}

/**
 * Which rows are mounted. `opening` holds the tail until the first landing has
 * given the list a real offset. `frozen` holds the rows already on screen for
 * the one commit where a prepend has shifted every index but the scroll offset
 * has not caught up, which would otherwise mount the wrong page and drop the
 * rows under the reader. Pinned rows join on top of either, and each gap they
 * leave gets its own spacer.
 */
export function windowedRowIndexes(
  range: Range,
  {
    opening,
    frozen,
    pinned,
  }: {
    opening: boolean
    frozen: readonly number[] | null
    pinned: Iterable<number>
  },
): number[] {
  const { count } = range
  if (count <= WINDOW_MIN_ROWS && !frozen) {
    return Array.from({ length: count }, (_, index) => index)
  }
  const base = opening
    ? Array.from(
        { length: Math.min(count, WINDOW_OPENING_ROWS) },
        (_, offset) => count - 1 - offset,
      )
    : (frozen ?? defaultRangeExtractor(range))
  // The tail is always there: a row appended live decides its entry animation
  // in the one commit it first appears.
  const indexes = new Set<number>([...base, count - 1, ...pinned])
  return [...indexes]
    .filter((index) => index >= 0 && index < count)
    .sort((a, b) => a - b)
}

/**
 * The distance a row adds to the stack: its box, its margins, and one gap. All
 * of it counts because a sibling-spacing margin is invisible to ResizeObserver
 * yet still moves everything below when the row unmounts. A spacer standing in
 * for rows is therefore their slots minus the one gap it brings itself.
 */
export function measureRowSlot(
  element: HTMLDivElement,
  entry: ResizeObserverEntry | undefined,
  instance: Virtualizer<HTMLElement, HTMLDivElement>,
): number {
  const key = instance.options.getItemKey(instance.indexFromElement(element))
  const cached = instance.itemSizeCache.get(key)
  // The synchronous path runs on every mount: a known row skips the layout read
  // and ResizeObserver reports it if it changed.
  if (!entry && cached !== undefined) return cached
  const height = entry?.borderBoxSize?.[0]?.blockSize ?? element.offsetHeight
  // A hidden pane reports zero for every mounted row; that is not its height.
  if (height === 0 && cached !== undefined) return cached
  return rowSlot(element, height)
}

export function rowSlot(element: HTMLDivElement, height: number): number {
  const style = getComputedStyle(element)
  const gap = element.parentElement
    ? Number.parseFloat(getComputedStyle(element.parentElement).rowGap)
    : 0
  return (
    height +
    (Number.parseFloat(style.marginTop) || 0) +
    (Number.parseFloat(style.marginBottom) || 0) +
    (gap || 0)
  )
}

/** A pane that loses its height keeps its window instead of mounting nothing. */
export function observeNonEmptyRect(
  instance: Virtualizer<HTMLElement, HTMLDivElement>,
  callback: (rect: Rect) => void,
) {
  return observeElementRect(instance, (rect) => {
    if (rect.height > 0) callback(rect)
  })
}

/**
 * The mounted rows in order, with a spacer for each run of unmounted ones.
 * `slot` is what the run adds to the list, gaps included; a spacer is a flex
 * child too, so it subtracts the one gap it brings itself.
 */
export function rowsWithSpacers(
  items: readonly { index: number; key: unknown; start: number; end: number }[],
  totalSize: number,
  renderRow: (index: number) => ReactNode,
  renderSpacer: (key: string, slot: number) => ReactNode,
): ReactNode[] {
  const nodes: ReactNode[] = []
  let previousEnd = 0
  for (const item of items) {
    if (item.start > previousEnd) {
      nodes.push(renderSpacer(`spacer:${item.key}`, item.start - previousEnd))
    }
    nodes.push(renderRow(item.index))
    previousEnd = item.end
  }
  if (totalSize > previousEnd) {
    nodes.push(renderSpacer('spacer:end', totalSize - previousEnd))
  }
  return nodes
}

/** The stack child a node belongs to. Group items carry the attribute too. */
export const TOP_LEVEL_ROW_SELECTOR = '.chat-message-stack > [data-message-row]'

export function topLevelRowKey(target: Element | null): string | undefined {
  return (
    target?.closest<HTMLElement>(TOP_LEVEL_ROW_SELECTOR)?.dataset.messageRow ??
    undefined
  )
}

export interface SelectedRowKeys {
  anchor: string
  focus: string
}

/**
 * The rows at both ends of the reader's text selection. Unmounting either end
 * collapses the selection, and any row between them would drop out of a copy.
 */
export function useSelectedRowKeys(
  contentRef: RefObject<HTMLElement | null>,
): SelectedRowKeys | null {
  const [selected, setSelected] = useState<SelectedRowKeys | null>(null)
  useEffect(() => {
    const rowKeyOf = (node: Node | null) => {
      const element =
        node instanceof Element ? node : (node?.parentElement ?? null)
      return element && contentRef.current?.contains(element)
        ? topLevelRowKey(element)
        : undefined
    }
    const onSelectionChange = () => {
      const selection = document.getSelection()
      const collapsed = !selection || selection.isCollapsed
      const anchor = collapsed ? undefined : rowKeyOf(selection.anchorNode)
      const focus = collapsed ? undefined : rowKeyOf(selection.focusNode)
      setSelected((current) => {
        if (!anchor && !focus) return null
        // A drag over a spacer has no row at that end yet: keep the last one.
        const nextAnchor = anchor ?? current?.anchor ?? (focus as string)
        const nextFocus = focus ?? current?.focus ?? (anchor as string)
        return current?.anchor === nextAnchor && current.focus === nextFocus
          ? current
          : { anchor: nextAnchor, focus: nextFocus }
      })
    }
    document.addEventListener('selectionchange', onSelectionChange)
    return () =>
      document.removeEventListener('selectionchange', onSelectionChange)
  }, [contentRef])
  return selected
}
