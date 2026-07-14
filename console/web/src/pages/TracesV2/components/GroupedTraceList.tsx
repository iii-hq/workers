// Grouped list mode for the TRACES tab (`filters.groupBy !== 'none'`).
//
// Renders the server-side `engine::traces::group_by` aggregation as
// collapsible group rows. Headings prefer the engine-resolved `label`
// (session name via `iii.session.name`, message preview via
// `iii.tag.message`) over the raw grouped value, so message/session groups
// read as text instead of UUIDs. Expanding a group fetches its member
// traces in ONE `engine::traces::list` call via the `trace_ids` filter and
// reuses `TraceListRow`, so selection, labelling, and hide-function all
// behave exactly like the flat list.

import { useQuery } from '@tanstack/react-query'
import { ChevronRight, Layers, Loader2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import { fetchTraces, type TraceGroup } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import { useTraceGroups } from '../hooks/useTraceGroups'
import {
  defaultLabelAttribute,
  type GroupByOption,
  groupHeading,
  groupHeadingIsOpaque,
  summarizeGroup,
} from '../lib/groupTraces'
import {
  dedupeToTraceRoots,
  mapSpanToListItem,
  type RowLabelConfig,
} from '../lib/traceListItem'
import { TraceListRow } from './TraceListRow'

const MEMBER_FETCH_LIMIT = 500

interface GroupedTraceListProps {
  attribute: GroupByOption
  showSystem: boolean
  hiddenFunctions?: string[]
  label?: RowLabelConfig
  selectedTraceId: string | null
  onSelectTrace: (traceId: string) => void
  onHideFunction?: (functionId: string) => void
  /** accordion body rendered beneath the selected member row */
  expandedContent?: React.ReactNode
}

export function GroupedTraceList({
  attribute,
  showSystem,
  hiddenFunctions,
  label,
  selectedTraceId,
  onSelectTrace,
  onHideFunction,
  expandedContent,
}: GroupedTraceListProps) {
  const { groups, isLoading, unavailable } = useTraceGroups({
    groupBy: attribute,
    includeInternal: showSystem,
    labelAttribute: defaultLabelAttribute(attribute),
  })
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  // Selection can arrive from outside the list (follow-live-turn, timeline
  // strip, deep link). The detail accordion only renders under a visible
  // member row, so a collapsed group would swallow it: auto-expand the
  // group(s) containing the selection, then scroll the row into view once
  // the member fetch lands. One-shot per trace id — collapsing the group
  // again while that trace is still selected is respected.
  const autoExpandedForRef = useRef<string | null>(null)
  const revealFrameRef = useRef(0)
  useEffect(() => () => cancelAnimationFrame(revealFrameRef.current), [])
  useEffect(() => {
    if (!selectedTraceId) {
      autoExpandedForRef.current = null
      return
    }
    if (autoExpandedForRef.current === selectedTraceId) return
    const containing = groups.filter((g) =>
      g.trace_ids.includes(selectedTraceId),
    )
    // Groups may still be loading, or a live turn's trace may not be in the
    // aggregation yet — leave the ref unset so the next groups update retries.
    if (containing.length === 0) return
    autoExpandedForRef.current = selectedTraceId
    setExpanded((prev) => {
      if (containing.every((g) => prev.has(g.value))) return prev
      const next = new Set(prev)
      for (const g of containing) next.add(g.value)
      return next
    })
    // Member rows fetch on expansion; poll a few frames for the row to
    // mount (bounded so a failed fetch can't leave a perpetual rAF loop).
    cancelAnimationFrame(revealFrameRef.current)
    const deadline = performance.now() + 3000
    const reveal = () => {
      const row = document.querySelector(
        `[data-trace-row-id="${CSS.escape(selectedTraceId)}"]`,
      )
      if (row) {
        row.scrollIntoView({ block: 'nearest' })
        return
      }
      if (performance.now() > deadline) return
      revealFrameRef.current = requestAnimationFrame(reveal)
    }
    revealFrameRef.current = requestAnimationFrame(reveal)
  }, [selectedTraceId, groups])

  if (unavailable) {
    return (
      <div className="flex flex-col items-center justify-center py-12 px-6 text-center font-mono text-[12px] text-ink-faint lowercase">
        <Layers className="w-6 h-6 mb-2 opacity-50" />
        <div className="font-medium text-ink mb-1">group-by not available</div>
        <div className="max-w-md leading-[1.7]">
          the engine doesn't expose{' '}
          <code className="text-warn">engine::traces::group_by</code>. either
          the engine is older than the version that introduced it, or the{' '}
          <code className="text-warn">iii-observability</code> worker is not
          configured. switch &quot;group by&quot; back to &quot;no
          grouping&quot; to use the flat trace list.
        </div>
      </div>
    )
  }

  if (isLoading && groups.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 font-mono text-[12px] text-ink-faint gap-2 lowercase">
        <Loader2 className="w-4 h-4 animate-spin" />
        loading groups…
      </div>
    )
  }

  if (groups.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 px-6 text-center font-mono text-[12px] text-ink-faint lowercase">
        <Layers className="w-6 h-6 mb-2 opacity-50" />
        <div>no traces carry this attribute yet.</div>
      </div>
    )
  }

  const toggle = (value: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(value)) next.delete(value)
      else next.add(value)
      return next
    })
  }

  return (
    <div className="flex flex-col">
      {groups.map((group) => {
        const isExpanded = expanded.has(group.value)
        return (
          <div key={`${attribute}:${group.value}`}>
            <GroupHeaderRow
              attribute={attribute}
              group={group}
              isExpanded={isExpanded}
              containsSelection={
                selectedTraceId !== null &&
                group.trace_ids.includes(selectedTraceId)
              }
              onToggle={() => toggle(group.value)}
            />
            {isExpanded && (
              <GroupMembers
                attribute={attribute}
                group={group}
                showSystem={showSystem}
                hiddenFunctions={hiddenFunctions}
                label={label}
                selectedTraceId={selectedTraceId}
                onSelectTrace={onSelectTrace}
                onHideFunction={onHideFunction}
                expandedContent={expandedContent}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

interface GroupHeaderRowProps {
  attribute: GroupByOption
  group: TraceGroup
  isExpanded: boolean
  containsSelection: boolean
  onToggle: () => void
}

function GroupHeaderRow({
  attribute,
  group,
  isExpanded,
  containsSelection,
  onToggle,
}: GroupHeaderRowProps) {
  const heading = groupHeading(group, attribute)
  const opaque = groupHeadingIsOpaque(group, attribute)
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={isExpanded}
      className={cn(
        'w-full flex items-center gap-2 px-3 py-2 text-left border-b border-rule-2 transition-colors hover:bg-panel',
        containsSelection && 'border-l-2 border-l-accent',
      )}
      title={`${heading} (${group.trace_ids.length} trace${group.trace_ids.length === 1 ? '' : 's'})`}
    >
      <ChevronRight
        className={cn(
          'w-3 h-3 flex-shrink-0 text-ink-faint transition-transform',
          isExpanded && 'rotate-90',
        )}
      />
      <Layers
        className={cn(
          'w-3.5 h-3.5 flex-shrink-0',
          group.error_count > 0 ? 'text-alert' : 'text-ink-faint',
        )}
      />
      <span
        className={cn(
          'font-mono text-[12px] truncate lowercase',
          opaque ? 'text-ink-faint' : 'text-ink',
        )}
      >
        {heading}
      </span>
      <span className="ml-auto font-mono text-[11px] text-ink-faint flex-shrink-0 tabular-nums lowercase">
        {summarizeGroup(group)}
      </span>
    </button>
  )
}

interface GroupMembersProps {
  attribute: GroupByOption
  group: TraceGroup
  showSystem: boolean
  hiddenFunctions?: string[]
  label?: RowLabelConfig
  selectedTraceId: string | null
  onSelectTrace: (traceId: string) => void
  onHideFunction?: (functionId: string) => void
  expandedContent?: React.ReactNode
}

function GroupMembers({
  attribute,
  group,
  showSystem,
  hiddenFunctions,
  label,
  selectedTraceId,
  onSelectTrace,
  onHideFunction,
  expandedContent,
}: GroupMembersProps) {
  // Keyed by group identity + member-count so live growth refetches, while
  // the id list itself rides in via closure (it can be hundreds of entries).
  const { data, isLoading } = useQuery<TraceListItem[]>({
    queryKey: [
      'traceGroupMembers',
      attribute,
      group.value,
      group.trace_ids.length,
      showSystem,
    ],
    queryFn: async () => {
      const { spans } = await fetchTraces({
        trace_ids: group.trace_ids.slice(0, MEMBER_FETCH_LIMIT),
        include_internal: showSystem,
        limit: MEMBER_FETCH_LIMIT,
      })
      const rows = dedupeToTraceRoots(spans).map(mapSpanToListItem)
      rows.sort((a, b) => b.startTime - a.startTime)
      return rows
    },
    staleTime: 1000,
  })

  if (isLoading && !data) {
    return (
      <div className="flex items-center gap-2 pl-10 py-2 border-b border-rule-2 font-mono text-[11px] text-ink-faint lowercase">
        <Loader2 className="w-3 h-3 animate-spin" />
        loading traces…
      </div>
    )
  }

  const hidden = hiddenFunctions ?? []
  const rows = (data ?? []).filter(
    (t) => !(t.functionId && hidden.includes(t.functionId)),
  )

  if (rows.length === 0) {
    return (
      <div className="pl-10 py-2 border-b border-rule-2 font-mono text-[11px] text-ink-faint lowercase">
        no visible traces in this group.
      </div>
    )
  }

  return (
    <div className="pl-6 border-l border-rule-2 ml-4">
      {rows.map((trace) => (
        <div key={trace.traceId} data-trace-row-id={trace.traceId}>
          <TraceListRow
            trace={trace}
            isSelected={selectedTraceId === trace.traceId}
            isNew={false}
            onSelect={() => onSelectTrace(trace.traceId)}
            onAnimationEnd={() => {}}
            label={label}
            onHideFunction={onHideFunction}
          />
          {selectedTraceId === trace.traceId && expandedContent}
        </div>
      ))}
    </div>
  )
}
