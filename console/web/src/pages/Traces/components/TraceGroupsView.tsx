// Renders the server-aggregated group-by view for the TRACES tab.
//
// Used when `filters.groupBy !== 'none'`. The route swaps this in place of
// the flat WaterfallChart list. Each row collapses one attribute-value's
// worth of spans across multiple traces and shows summary metadata
// (span_count, duration, error_count). Clicking a group row drills down
// to a single trace via the existing trace-detail panel; for v1, we
// surface the first trace_id in the group.
//
// If the engine doesn't expose `engine::traces::group_by` (older deploy
// or `iii-observability` not running), the `unavailable` flag from the
// hook surfaces a soft fallback message — the user can switch back to
// "No grouping" without seeing an error toast.

import { Layers, Loader2 } from 'lucide-react'

import { cn } from '@/lib/utils'
import type { TraceGroup } from '../api/traces'
import { useTraceGroups } from '../hooks/useTraceGroups'
import type { GroupByAttribute } from '../lib/groupTraces'
import { groupHeading, summarizeGroup } from '../lib/groupTraces'

interface TraceGroupsViewProps {
  attribute: GroupByAttribute
  showSystem: boolean
  /**
   * Called with the full TraceGroup when a row is clicked. Preferred path —
   * opens a session-style multi-trace detail panel that owns its own fetches.
   */
  onSelectGroup?: (group: TraceGroup) => void
  /**
   * Legacy fallback for call-sites without a session-detail container: drills
   * into the group's first trace via single-trace selection. Only used when
   * `onSelectGroup` is not provided.
   */
  onSelectTrace?: (traceId: string) => void
  /** `group.value` of the currently-open group, for row highlight. */
  selectedGroupValue: string | null
}

export function TraceGroupsView({
  attribute,
  showSystem,
  onSelectGroup,
  onSelectTrace,
  selectedGroupValue,
}: TraceGroupsViewProps) {
  const { groups, isLoading, unavailable } = useTraceGroups({
    groupBy: attribute,
    includeInternal: showSystem,
  })

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
        <div>no spans match this attribute yet.</div>
      </div>
    )
  }

  return (
    <div className="flex flex-col">
      {groups.map((group) => (
        <GroupRow
          key={`${attribute}:${group.value}`}
          attribute={attribute}
          group={group}
          isSelected={
            selectedGroupValue !== null && group.value === selectedGroupValue
          }
          onClick={() => {
            // Prefer the session-aware container; fall back to single-trace
            // selection only for call-sites that don't handle groups.
            if (onSelectGroup) {
              onSelectGroup(group)
            } else {
              const firstTrace = group.trace_ids[0]
              if (firstTrace) onSelectTrace?.(firstTrace)
            }
          }}
        />
      ))}
    </div>
  )
}

interface GroupRowProps {
  attribute: GroupByAttribute
  group: TraceGroup
  isSelected: boolean
  onClick: () => void
}

function GroupRow({ attribute, group, isSelected, onClick }: GroupRowProps) {
  const heading = groupHeading(group, attribute)
  const summary = summarizeGroup(group)
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-2 px-3 py-2 text-left border-b border-rule-2 transition-colors',
        isSelected ? 'bg-panel border-l-2 border-l-accent' : 'hover:bg-panel',
      )}
      title={`${heading} (${group.trace_ids.length} trace${group.trace_ids.length === 1 ? '' : 's'})`}
    >
      <Layers
        className={cn(
          'w-3.5 h-3.5 flex-shrink-0',
          group.error_count > 0 ? 'text-alert' : 'text-ink-faint',
        )}
      />
      <span
        className={cn(
          'font-mono text-[12px] truncate lowercase',
          isSelected ? 'text-accent' : 'text-ink',
        )}
      >
        {heading}
      </span>
      <span className="ml-auto font-mono text-[11px] text-ink-faint flex-shrink-0 tabular-nums lowercase">
        {summary}
      </span>
    </button>
  )
}
