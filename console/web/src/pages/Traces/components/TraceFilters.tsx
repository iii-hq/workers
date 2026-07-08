// Trace filter bar.
//
// Layout: a single row of always-visible controls (search, group-by,
// status, more-filters trigger, stats) with active-filter chips
// rendered below. The advanced filters (time range, sort, duration,
// worker, operation, attributes) live inside a "more filters"
// popover so they stay out of the way until needed.
//
// Behaviors preserved from motia:
//   • 300ms-debounced text inputs (workerName, operationName) — wired
//     via the parent useTraceFilters hook.
//   • Min/max duration validation warnings.
//   • Group-by switching (server-side aggregation).
//   • Status filtering (ok / error / unset / all).
//   • Sort by column + asc/desc.
//   • Time range presets.
//   • Attribute key/value editor (via AttributesFilter).
//   • Active-filter chips for one-click removal.
//   • Stats summary (totalTraces, errorCount, avgDuration).

import {
  AlertTriangle,
  ChevronDown,
  Hash,
  Search,
  SlidersHorizontal,
  Timer,
  X,
  XCircle,
} from 'lucide-react'
import { useEffect, useReducer, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Button } from '@/components/ui/Button'
import { Caret } from '@/components/ui/Caret'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { cn } from '@/lib/utils'
import type { TraceFilterState } from '../hooks/useTraceFilters'
import type { GroupByOption } from '../lib/groupTraces'
import { AttributesFilter } from './AttributesFilter'

interface TraceFiltersProps {
  filters: TraceFilterState
  onFilterChange: (key: keyof TraceFilterState, value: unknown) => void
  onClear: () => void
  validationWarnings?: {
    durationSwapped?: boolean
    timeRangeSwapped?: boolean
  }
  onClearWarnings?: () => void
  isLoading?: boolean
  searchQuery?: string
  onSearchChange?: (value: string) => void
  stats?: {
    totalTraces: number
    errorCount: number
    avgDuration: number
  }
}

const TIME_RANGE_PRESETS: Array<{ label: string; ms: number | null }> = [
  { label: 'all time', ms: null },
  { label: 'last 15m', ms: 15 * 60 * 1000 },
  { label: 'last 1h', ms: 60 * 60 * 1000 },
  { label: 'last 6h', ms: 6 * 60 * 60 * 1000 },
  { label: 'last 24h', ms: 24 * 60 * 60 * 1000 },
  { label: 'last 7d', ms: 7 * 24 * 60 * 60 * 1000 },
]

const SORT_BY_OPTIONS: Array<{
  label: string
  value: 'start_time' | 'duration' | 'service_name'
}> = [
  { label: 'start time', value: 'start_time' },
  { label: 'duration', value: 'duration' },
  { label: 'worker name', value: 'service_name' },
]

type StatusValue = 'all' | 'ok' | 'error' | 'unset'

const STATUS_OPTIONS: Array<{ value: StatusValue; label: string }> = [
  { value: 'all', label: 'all' },
  { value: 'ok', label: 'ok' },
  { value: 'error', label: 'error' },
  { value: 'unset', label: 'unset' },
]

const GROUP_BY_OPTIONS: Array<{ value: GroupByOption; label: string }> = [
  { value: 'none', label: 'no grouping' },
  { value: 'iii.message.id', label: 'message' },
  { value: 'iii.session.id', label: 'session' },
  { value: 'iii.function.id', label: 'function' },
]

const SORT_ORDER_OPTIONS: Array<{ value: 'asc' | 'desc'; label: string }> = [
  { value: 'desc', label: 'desc' },
  { value: 'asc', label: 'asc' },
]

// --- Temp Inputs Reducer ---
interface TempInputsState {
  tempWorkerName: string
  tempOperationName: string
  tempMinDuration: string
  tempMaxDuration: string
}

type TempInputsAction =
  | { type: 'SET_WORKER_NAME'; payload: string }
  | { type: 'SET_OPERATION_NAME'; payload: string }
  | { type: 'SET_MIN_DURATION'; payload: string }
  | { type: 'SET_MAX_DURATION'; payload: string }
  | { type: 'CLEAR_DURATION' }
  | { type: 'CLEAR_WORKER_NAME' }
  | { type: 'CLEAR_OPERATION_NAME' }
  | {
      type: 'SYNC_FROM_FILTERS'
      payload: {
        workerName?: string
        operationName?: string
        minDurationMs?: number | null
        maxDurationMs?: number | null
      }
    }

function tempInputsReducer(
  state: TempInputsState,
  action: TempInputsAction,
): TempInputsState {
  switch (action.type) {
    case 'SET_WORKER_NAME':
      return { ...state, tempWorkerName: action.payload }
    case 'SET_OPERATION_NAME':
      return { ...state, tempOperationName: action.payload }
    case 'SET_MIN_DURATION':
      return { ...state, tempMinDuration: action.payload }
    case 'SET_MAX_DURATION':
      return { ...state, tempMaxDuration: action.payload }
    case 'CLEAR_DURATION':
      return { ...state, tempMinDuration: '', tempMaxDuration: '' }
    case 'CLEAR_WORKER_NAME':
      return { ...state, tempWorkerName: '' }
    case 'CLEAR_OPERATION_NAME':
      return { ...state, tempOperationName: '' }
    case 'SYNC_FROM_FILTERS':
      return {
        tempWorkerName: action.payload.workerName || '',
        tempOperationName: action.payload.operationName || '',
        tempMinDuration: action.payload.minDurationMs?.toString() || '',
        tempMaxDuration: action.payload.maxDurationMs?.toString() || '',
      }
    default:
      return state
  }
}

function formatDurationMs(ms: number | undefined): string {
  if (!ms) return '—'
  if (ms < 1) return '<1ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function currentTimeRangeIndex(filters: TraceFilterState): number {
  if (!filters.startTime || !filters.endTime) return 0
  const diff = filters.endTime - filters.startTime
  const idx = TIME_RANGE_PRESETS.findIndex((p) => p.ms === diff)
  return idx === -1 ? 0 : idx
}

const inputClass =
  'h-8 px-2 font-mono text-[12px] bg-bg border border-rule text-ink placeholder:text-ink-ghost focus:outline-none focus:border-accent transition-colors lowercase'

const selectClass =
  'h-8 px-2 font-mono text-[12px] bg-bg border border-rule text-ink focus:outline-none focus:border-accent transition-colors lowercase appearance-none cursor-pointer'

// Lightweight popover anchored to a trigger via getBoundingClientRect.
// Portals into document.body so it isn't clipped by overflow-hidden
// ancestors (the Traces section is overflow-hidden).
interface MoreFiltersPopoverProps {
  open: boolean
  onClose: () => void
  triggerRef: React.RefObject<HTMLElement | null>
  width?: number
  children: React.ReactNode
}

function MoreFiltersPopover({
  open,
  onClose,
  triggerRef,
  width = 520,
  children,
}: MoreFiltersPopoverProps) {
  const popoverRef = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{
    top: number
    right: number
    width: number
    maxHeight: number
  } | null>(null)

  useEffect(() => {
    if (!open || !triggerRef.current) return
    const MARGIN = 8
    const update = () => {
      const trigger = triggerRef.current
      if (!trigger) return
      const rect = trigger.getBoundingClientRect()
      const actualWidth = Math.min(width, window.innerWidth - MARGIN * 2)
      // Right-align to the trigger, clamped so neither edge leaves the
      // viewport.
      const right = Math.min(
        Math.max(window.innerWidth - rect.right, MARGIN),
        window.innerWidth - actualWidth - MARGIN,
      )
      let top = rect.bottom + 4
      let maxHeight = window.innerHeight - top - MARGIN
      const MIN_HEIGHT = 240
      if (maxHeight < MIN_HEIGHT) {
        // Not enough room below the trigger — raise the popover (possibly
        // over the trigger) so the content stays usable and scrollable.
        maxHeight = Math.min(MIN_HEIGHT, window.innerHeight - MARGIN * 2)
        top = window.innerHeight - MARGIN - maxHeight
      }
      setPos({ top, right, width: actualWidth, maxHeight })
    }
    update()
    window.addEventListener('resize', update)
    window.addEventListener('scroll', update, true)
    return () => {
      window.removeEventListener('resize', update)
      window.removeEventListener('scroll', update, true)
    }
  }, [open, triggerRef, width])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (popoverRef.current?.contains(target)) return
      if (triggerRef.current?.contains(target)) return
      onClose()
    }
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleEscape)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleEscape)
    }
  }, [open, onClose, triggerRef])

  if (!open || !pos) return null

  return createPortal(
    <div
      ref={popoverRef}
      style={{
        position: 'fixed',
        top: pos.top,
        right: pos.right,
        width: pos.width,
        maxHeight: pos.maxHeight,
        zIndex: 50,
      }}
      className="bg-bg border border-rule p-3 shadow-[0_8px_24px_rgba(0,0,0,0.18)] overflow-y-auto"
    >
      {children}
    </div>,
    document.body,
  )
}

function SearchInput({
  searchQuery,
  onSearchChange,
  className,
}: {
  searchQuery: string
  onSearchChange?: (value: string) => void
  className?: string
}) {
  const [searchFocused, setSearchFocused] = useState(false)
  return (
    <label
      className={cn(
        'flex items-center gap-2 h-8 px-2 border bg-bg transition-colors',
        searchFocused ? 'border-accent' : 'border-rule',
        className,
      )}
    >
      <Search className="w-3 h-3 text-ink-faint flex-shrink-0" />
      <span className="text-accent font-mono text-[12px] select-none">$</span>
      <input
        type="text"
        value={searchQuery}
        onChange={(e) => onSearchChange?.(e.target.value)}
        onFocus={() => setSearchFocused(true)}
        onBlur={() => setSearchFocused(false)}
        placeholder="search traces..."
        className="flex-1 bg-transparent font-mono text-[12px] text-ink placeholder:text-ink-ghost focus:outline-none lowercase min-w-0"
      />
      {searchFocused && !searchQuery && <Caret className="h-[12px] w-[5px]" />}
      {searchQuery && (
        <button
          type="button"
          onClick={() => onSearchChange?.('')}
          className="text-ink-faint hover:text-ink transition-colors"
          aria-label="clear search"
        >
          <X className="w-3 h-3" />
        </button>
      )}
    </label>
  )
}

function StatsBlock({ stats }: { stats: TraceFiltersProps['stats'] }) {
  if (!stats) return null
  return (
    <div className="flex items-stretch border border-rule divide-x divide-rule-2">
      <div className="flex items-center gap-1.5 px-2.5 py-1 font-mono text-[11px] text-ink-faint lowercase">
        <Hash className="w-3 h-3 text-ink-faint" />
        <span className="text-ink tabular-nums">{stats.totalTraces}</span>
        <span className="text-ink-ghost">traces</span>
      </div>
      <div
        className={cn(
          'flex items-center gap-1.5 px-2.5 py-1 font-mono text-[11px] lowercase',
          stats.errorCount > 0 ? 'text-alert' : 'text-ink-faint',
        )}
      >
        <XCircle className="w-3 h-3" />
        <span className="tabular-nums">{stats.errorCount}</span>
        <span className="text-ink-ghost">errors</span>
      </div>
      <div className="flex items-center gap-1.5 px-2.5 py-1 font-mono text-[11px] text-ink-faint lowercase">
        <Timer className="w-3 h-3" />
        <span className="text-ink tabular-nums">
          {formatDurationMs(stats.avgDuration)}
        </span>
        <span className="text-ink-ghost">avg</span>
      </div>
    </div>
  )
}

export function TraceFilters({
  filters,
  onFilterChange,
  onClear,
  validationWarnings,
  onClearWarnings,
  isLoading,
  searchQuery = '',
  onSearchChange,
  stats,
}: TraceFiltersProps) {
  const [tempInputs, dispatchTempInputs] = useReducer(tempInputsReducer, {
    tempWorkerName: filters.workerName || '',
    tempOperationName: filters.operationName || '',
    tempMinDuration: filters.minDurationMs?.toString() || '',
    tempMaxDuration: filters.maxDurationMs?.toString() || '',
  })
  const {
    tempWorkerName,
    tempOperationName,
    tempMinDuration,
    tempMaxDuration,
  } = tempInputs

  const [prevFilters, setPrevFilters] = useState({
    workerName: filters.workerName,
    operationName: filters.operationName,
    minDurationMs: filters.minDurationMs,
    maxDurationMs: filters.maxDurationMs,
  })

  if (
    prevFilters.workerName !== filters.workerName ||
    prevFilters.operationName !== filters.operationName ||
    prevFilters.minDurationMs !== filters.minDurationMs ||
    prevFilters.maxDurationMs !== filters.maxDurationMs
  ) {
    setPrevFilters({
      workerName: filters.workerName,
      operationName: filters.operationName,
      minDurationMs: filters.minDurationMs,
      maxDurationMs: filters.maxDurationMs,
    })
    dispatchTempInputs({
      type: 'SYNC_FROM_FILTERS',
      payload: {
        workerName: filters.workerName,
        operationName: filters.operationName,
        minDurationMs: filters.minDurationMs,
        maxDurationMs: filters.maxDurationMs,
      },
    })
  }

  const applyWorker = () => {
    onFilterChange('workerName', tempWorkerName || undefined)
  }
  const applyOperation = () => {
    onFilterChange('operationName', tempOperationName || undefined)
  }
  const applyDuration = () => {
    const parsedMin = tempMinDuration
      ? Number.parseInt(tempMinDuration, 10)
      : null
    const parsedMax = tempMaxDuration
      ? Number.parseInt(tempMaxDuration, 10)
      : null
    onFilterChange(
      'minDurationMs',
      parsedMin !== null && Number.isFinite(parsedMin) ? parsedMin : null,
    )
    onFilterChange(
      'maxDurationMs',
      parsedMax !== null && Number.isFinite(parsedMax) ? parsedMax : null,
    )
  }

  const handleTimeRangeChange = (presetIndex: number) => {
    const preset = TIME_RANGE_PRESETS[presetIndex]
    if (!preset) return
    if (preset.ms === null) {
      onFilterChange('startTime', null)
      onFilterChange('endTime', null)
      return
    }
    const now = Date.now()
    onFilterChange('startTime', now - preset.ms)
    onFilterChange('endTime', now)
  }

  const handleEnterApply = (
    e: React.KeyboardEvent<HTMLInputElement>,
    handler: () => void,
  ) => {
    if (e.key === 'Enter') handler()
  }

  const removeFilter = (key: string) => {
    switch (key) {
      case 'status':
        onFilterChange('status', null)
        break
      case 'workerName':
        onFilterChange('workerName', undefined)
        dispatchTempInputs({ type: 'CLEAR_WORKER_NAME' })
        break
      case 'operationName':
        onFilterChange('operationName', undefined)
        dispatchTempInputs({ type: 'CLEAR_OPERATION_NAME' })
        break
      case 'duration':
        onFilterChange('minDurationMs', null)
        onFilterChange('maxDurationMs', null)
        dispatchTempInputs({ type: 'CLEAR_DURATION' })
        break
      case 'timeRange':
        onFilterChange('startTime', null)
        onFilterChange('endTime', null)
        break
      case 'attributes':
        onFilterChange('attributes', undefined)
        break
      case 'sortBy':
        onFilterChange('sortBy', 'start_time')
        break
      case 'sortOrder':
        onFilterChange('sortOrder', 'desc')
        break
    }
  }

  const activeFilters: Array<{ key: string; label: string }> = []
  if (filters.status) {
    activeFilters.push({ key: 'status', label: `status: ${filters.status}` })
  }
  if (filters.workerName) {
    activeFilters.push({
      key: 'workerName',
      label: `worker: ${filters.workerName}`,
    })
  }
  if (filters.operationName) {
    activeFilters.push({
      key: 'operationName',
      label: `operation: ${filters.operationName}`,
    })
  }
  if (filters.minDurationMs || filters.maxDurationMs) {
    const parts: string[] = []
    if (filters.minDurationMs) parts.push(`${filters.minDurationMs}ms`)
    if (filters.maxDurationMs) parts.push(`${filters.maxDurationMs}ms`)
    activeFilters.push({
      key: 'duration',
      label: `duration: ${parts.join('-')}`,
    })
  }
  if (filters.startTime && filters.endTime) {
    const diff = filters.endTime - filters.startTime
    const preset = TIME_RANGE_PRESETS.find((p) => p.ms === diff)
    activeFilters.push({
      key: 'timeRange',
      label: preset ? preset.label : 'custom range',
    })
  }
  if (filters.attributes && filters.attributes.length > 0) {
    activeFilters.push({
      key: 'attributes',
      label: `attributes (${filters.attributes.length})`,
    })
  }
  if (filters.sortBy && filters.sortBy !== 'start_time') {
    const opt = SORT_BY_OPTIONS.find((o) => o.value === filters.sortBy)
    activeFilters.push({
      key: 'sortBy',
      label: `sort: ${opt?.label ?? filters.sortBy}`,
    })
  }
  if (filters.sortOrder && filters.sortOrder !== 'desc') {
    activeFilters.push({
      key: 'sortOrder',
      label: `order: ${filters.sortOrder}`,
    })
  }

  const statusValue: StatusValue = (filters.status ?? 'all') as StatusValue
  const groupByValue = (filters.groupBy ?? 'none') as GroupByOption
  const sortOrderValue = filters.sortOrder ?? 'desc'

  // Chips that aren't already represented by inline controls (status/group)
  const advancedFilterCount = activeFilters.filter(
    (f) => f.key !== 'status',
  ).length

  const [popoverOpen, setPopoverOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)

  // Hide validation warnings automatically once the underlying values
  // stop being suspect. The parent surfaces a manual dismiss button too.
  useEffect(() => {
    if (!validationWarnings) return
    if (
      !validationWarnings.durationSwapped &&
      !validationWarnings.timeRangeSwapped
    ) {
      onClearWarnings?.()
    }
  }, [validationWarnings, onClearWarnings])

  return (
    <div className="space-y-2" data-testid="trace-filters">
      {(validationWarnings?.durationSwapped ||
        validationWarnings?.timeRangeSwapped) && (
        <div className="relative">
          <StatusPanel
            variant="warn"
            icon={<AlertTriangle className="w-full h-full" />}
            headline="values corrected"
            detail={
              <>
                {validationWarnings.durationSwapped &&
                  'min/max duration swapped. '}
                {validationWarnings.timeRangeSwapped &&
                  'start/end time swapped. '}
              </>
            }
          />
          {onClearWarnings && (
            <button
              type="button"
              onClick={onClearWarnings}
              className="absolute top-2 right-2 text-warn hover:text-ink transition-colors"
              title="dismiss warning"
              aria-label="dismiss warning"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      )}

      {/* Always-visible row: search, group-by, status, more filters, stats */}
      <div className="flex items-center gap-2 flex-wrap">
        <SearchInput
          searchQuery={searchQuery}
          onSearchChange={onSearchChange}
          className="flex-1 min-w-[240px] max-w-[460px]"
        />

        <ModeToggle<GroupByOption>
          value={groupByValue}
          options={GROUP_BY_OPTIONS}
          onChange={(next) => onFilterChange('groupBy', next)}
        />

        <ModeToggle<StatusValue>
          value={statusValue}
          options={STATUS_OPTIONS}
          onChange={(next) =>
            onFilterChange('status', next === 'all' ? null : next)
          }
        />

        <button
          ref={triggerRef}
          type="button"
          onClick={() => setPopoverOpen((v) => !v)}
          className={cn(
            'inline-flex items-center gap-2 h-8 px-2.5 border font-mono text-[12px] lowercase transition-colors',
            popoverOpen || advancedFilterCount > 0
              ? 'border-accent text-ink'
              : 'border-rule text-ink-faint hover:text-ink hover:border-rule',
          )}
          aria-haspopup="dialog"
          aria-expanded={popoverOpen}
        >
          <SlidersHorizontal className="w-3 h-3" />
          <span>more filters</span>
          {advancedFilterCount > 0 && (
            <span className="px-1 py-0 bg-accent text-bg font-mono text-[10px] tabular-nums leading-none flex items-center min-w-[14px] justify-center">
              {advancedFilterCount}
            </span>
          )}
          <ChevronDown
            className={cn(
              'w-3 h-3 transition-transform',
              popoverOpen && 'rotate-180',
            )}
          />
        </button>

        <div className="ml-auto">
          <StatsBlock stats={stats} />
        </div>
      </div>

      {/* Active filter chips */}
      {activeFilters.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap pt-0.5">
          {isLoading && (
            <span
              aria-hidden
              className="w-1.5 h-1.5 bg-accent rounded-full pulse-dot mr-0.5"
            />
          )}
          {activeFilters.map((filter) => (
            <button
              key={filter.key}
              type="button"
              onClick={() => removeFilter(filter.key)}
              className="flex items-center gap-1 px-1.5 py-0.5 bg-panel border border-rule hover:border-accent hover:text-ink font-mono text-[10px] text-ink-faint transition-colors group lowercase"
            >
              <span>{filter.label}</span>
              <X className="w-2.5 h-2.5 opacity-50 group-hover:opacity-100 group-hover:text-accent transition-all" />
            </button>
          ))}
          <Button variant="ghost" size="sm" onClick={onClear} className="ml-1">
            clear all
          </Button>
        </div>
      )}

      <MoreFiltersPopover
        open={popoverOpen}
        onClose={() => setPopoverOpen(false)}
        triggerRef={triggerRef}
      >
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-mono text-[11px] text-ink-faint uppercase tracking-[0.06em]">
              more filters
            </h3>
            <button
              type="button"
              onClick={() => setPopoverOpen(false)}
              className="text-ink-faint hover:text-ink"
              aria-label="close"
            >
              <X className="w-3 h-3" />
            </button>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                time range
              </span>
              <select
                className={selectClass}
                value={currentTimeRangeIndex(filters)}
                onChange={(e) =>
                  handleTimeRangeChange(Number.parseInt(e.target.value, 10))
                }
              >
                {TIME_RANGE_PRESETS.map((preset, i) => (
                  <option key={preset.label} value={i}>
                    {preset.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                sort by
              </span>
              <div className="flex gap-2">
                <select
                  className={cn(selectClass, 'flex-1')}
                  value={filters.sortBy ?? 'start_time'}
                  onChange={(e) =>
                    onFilterChange(
                      'sortBy',
                      e.target.value as
                        | 'start_time'
                        | 'duration'
                        | 'service_name',
                    )
                  }
                >
                  {SORT_BY_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
                <ModeToggle<'asc' | 'desc'>
                  value={sortOrderValue}
                  options={SORT_ORDER_OPTIONS}
                  onChange={(next) => onFilterChange('sortOrder', next)}
                />
              </div>
            </label>

            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                worker
              </span>
              <input
                type="text"
                placeholder="e.g. api-*, backend"
                value={tempWorkerName}
                onChange={(e) =>
                  dispatchTempInputs({
                    type: 'SET_WORKER_NAME',
                    payload: e.target.value,
                  })
                }
                onBlur={applyWorker}
                onKeyDown={(e) => handleEnterApply(e, applyWorker)}
                className={inputClass}
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                operation
              </span>
              <input
                type="text"
                placeholder="e.g. get *, post /api/*"
                value={tempOperationName}
                onChange={(e) =>
                  dispatchTempInputs({
                    type: 'SET_OPERATION_NAME',
                    payload: e.target.value,
                  })
                }
                onBlur={applyOperation}
                onKeyDown={(e) => handleEnterApply(e, applyOperation)}
                className={inputClass}
              />
            </label>

            <label className="flex flex-col gap-1 col-span-2">
              <span className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                duration (ms)
              </span>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  inputMode="numeric"
                  placeholder="min"
                  value={tempMinDuration}
                  onChange={(e) =>
                    dispatchTempInputs({
                      type: 'SET_MIN_DURATION',
                      payload: e.target.value,
                    })
                  }
                  onBlur={applyDuration}
                  onKeyDown={(e) => handleEnterApply(e, applyDuration)}
                  className={cn(inputClass, 'flex-1 tabular-nums')}
                />
                <span className="text-ink-ghost font-mono text-[11px]">—</span>
                <input
                  type="number"
                  inputMode="numeric"
                  placeholder="max"
                  value={tempMaxDuration}
                  onChange={(e) =>
                    dispatchTempInputs({
                      type: 'SET_MAX_DURATION',
                      payload: e.target.value,
                    })
                  }
                  onBlur={applyDuration}
                  onKeyDown={(e) => handleEnterApply(e, applyDuration)}
                  className={cn(inputClass, 'flex-1 tabular-nums')}
                />
              </div>
            </label>
          </div>

          <div className="pt-2 border-t border-rule-2">
            <div className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em] mb-2">
              attributes
              {filters.attributes && filters.attributes.length > 0 ? (
                <span className="ml-1 text-accent tabular-nums normal-case">
                  ({filters.attributes.length})
                </span>
              ) : null}
            </div>
            <AttributesFilter
              value={filters.attributes || []}
              onChange={(attrs) =>
                onFilterChange(
                  'attributes',
                  attrs.length > 0 ? attrs : undefined,
                )
              }
            />
          </div>

          <div className="flex items-center justify-end pt-2 border-t border-rule-2">
            <Button variant="ghost" size="sm" onClick={onClear}>
              clear all filters
            </Button>
          </div>
        </div>
      </MoreFiltersPopover>
    </div>
  )
}
