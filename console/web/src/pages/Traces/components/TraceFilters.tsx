// Trace filter bar — rebuilt on schematic primitives.
//
// The motia version of this file relied on `cmdk` and `react-select`
// for popovers and combobox dropdowns. The iii Schematic system favors
// a flatter affordance set: ModeToggle for short option sets, native
// `<select>` for longer ones, bordered text inputs for free-form text,
// and `<details className="iii-details">` for collapsible chrome.
//
// Behaviors preserved from motia:
//   • 300ms-debounced text inputs (serviceName, operationName) — wired
//     via the parent useTraceFilters hook.
//   • Min/max duration validation warnings.
//   • Group-by switching (server-side aggregation).
//   • Status filtering (ok / error / unset / all).
//   • Sort by column + asc/desc.
//   • Time range presets.
//   • Attribute key/value editor (via AttributesFilter).
//   • Active-filter chips for one-click removal.
//   • Stats summary (totalTraces, errorCount, avgDuration).
//
// Behaviors dropped (no equivalent without cmdk/select):
//   • Service/Operation autocomplete dropdowns — these now collapse
//     to plain text inputs. The engine has no list endpoint exposed
//     yet, so the dropdowns in motia were free-form anyway.

import {
  AlertTriangle,
  ArrowUpDown,
  CircleDot,
  Hash,
  Layers,
  Search,
  Timer,
  X,
  XCircle,
} from 'lucide-react'
import { useEffect, useReducer, useState } from 'react'
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
  { label: 'service name', value: 'service_name' },
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
  tempServiceName: string
  tempOperationName: string
  tempMinDuration: string
  tempMaxDuration: string
}

type TempInputsAction =
  | { type: 'SET_SERVICE_NAME'; payload: string }
  | { type: 'SET_OPERATION_NAME'; payload: string }
  | { type: 'SET_MIN_DURATION'; payload: string }
  | { type: 'SET_MAX_DURATION'; payload: string }
  | { type: 'CLEAR_DURATION' }
  | { type: 'CLEAR_SERVICE_NAME' }
  | { type: 'CLEAR_OPERATION_NAME' }
  | {
      type: 'SYNC_FROM_FILTERS'
      payload: {
        serviceName?: string
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
    case 'SET_SERVICE_NAME':
      return { ...state, tempServiceName: action.payload }
    case 'SET_OPERATION_NAME':
      return { ...state, tempOperationName: action.payload }
    case 'SET_MIN_DURATION':
      return { ...state, tempMinDuration: action.payload }
    case 'SET_MAX_DURATION':
      return { ...state, tempMaxDuration: action.payload }
    case 'CLEAR_DURATION':
      return { ...state, tempMinDuration: '', tempMaxDuration: '' }
    case 'CLEAR_SERVICE_NAME':
      return { ...state, tempServiceName: '' }
    case 'CLEAR_OPERATION_NAME':
      return { ...state, tempOperationName: '' }
    case 'SYNC_FROM_FILTERS':
      return {
        tempServiceName: action.payload.serviceName || '',
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
  // Temporary state for text inputs — kept identical to motia so the
  // useTraceFilters debounce semantics still hold.
  const [tempInputs, dispatchTempInputs] = useReducer(tempInputsReducer, {
    tempServiceName: filters.serviceName || '',
    tempOperationName: filters.operationName || '',
    tempMinDuration: filters.minDurationMs?.toString() || '',
    tempMaxDuration: filters.maxDurationMs?.toString() || '',
  })
  const {
    tempServiceName,
    tempOperationName,
    tempMinDuration,
    tempMaxDuration,
  } = tempInputs

  const [prevFilters, setPrevFilters] = useState({
    serviceName: filters.serviceName,
    operationName: filters.operationName,
    minDurationMs: filters.minDurationMs,
    maxDurationMs: filters.maxDurationMs,
  })

  if (
    prevFilters.serviceName !== filters.serviceName ||
    prevFilters.operationName !== filters.operationName ||
    prevFilters.minDurationMs !== filters.minDurationMs ||
    prevFilters.maxDurationMs !== filters.maxDurationMs
  ) {
    setPrevFilters({
      serviceName: filters.serviceName,
      operationName: filters.operationName,
      minDurationMs: filters.minDurationMs,
      maxDurationMs: filters.maxDurationMs,
    })
    dispatchTempInputs({
      type: 'SYNC_FROM_FILTERS',
      payload: {
        serviceName: filters.serviceName,
        operationName: filters.operationName,
        minDurationMs: filters.minDurationMs,
        maxDurationMs: filters.maxDurationMs,
      },
    })
  }

  // Search focus tracking — drives the blinking caret only while the
  // input is focused, mirroring the schematic Terminal pattern.
  const [searchFocused, setSearchFocused] = useState(false)

  // Apply debounced service/operation values when they are changed
  // locally. Motia debounces in the hook; here we forward the temp
  // value on blur (and on Enter) so apply is explicit.
  const applyService = () => {
    onFilterChange('serviceName', tempServiceName || undefined)
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
      case 'serviceName':
        onFilterChange('serviceName', undefined)
        dispatchTempInputs({ type: 'CLEAR_SERVICE_NAME' })
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

  // Active filter chips data
  const activeFilters: Array<{ key: string; label: string }> = []
  if (filters.status) {
    activeFilters.push({ key: 'status', label: `status: ${filters.status}` })
  }
  if (filters.serviceName) {
    activeFilters.push({
      key: 'serviceName',
      label: `service: ${filters.serviceName}`,
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
    <div className="space-y-3" data-testid="trace-filters">
      {/* Validation Warnings */}
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

      {/* Row 1: search + groupBy + status */}
      <div className="flex items-center gap-3 flex-wrap">
        {/* Search field with terminal-style blinking caret */}
        <label
          className={cn(
            'flex items-center gap-2 h-8 px-2 border bg-bg transition-colors min-w-[220px] flex-1 max-w-[360px]',
            searchFocused ? 'border-accent' : 'border-rule',
          )}
        >
          <Search className="w-3 h-3 text-ink-faint flex-shrink-0" />
          <span className="text-accent font-mono text-[12px] select-none">
            $
          </span>
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => onSearchChange?.(e.target.value)}
            onFocus={() => setSearchFocused(true)}
            onBlur={() => setSearchFocused(false)}
            placeholder="search traces..."
            className="flex-1 bg-transparent font-mono text-[12px] text-ink placeholder:text-ink-ghost focus:outline-none lowercase"
          />
          {searchFocused && !searchQuery && (
            <Caret className="h-[12px] w-[5px]" />
          )}
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

        {/* Group By segmented control */}
        <div className="flex items-center gap-2">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            <Layers className="w-3 h-3 inline -mt-px mr-1" />
            group
          </span>
          <ModeToggle<GroupByOption>
            value={groupByValue}
            options={GROUP_BY_OPTIONS}
            onChange={(next) => onFilterChange('groupBy', next)}
          />
        </div>

        {/* Status segmented control */}
        <div className="flex items-center gap-2">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            <CircleDot className="w-3 h-3 inline -mt-px mr-1" />
            status
          </span>
          <ModeToggle<StatusValue>
            value={statusValue}
            options={STATUS_OPTIONS}
            onChange={(next) =>
              onFilterChange('status', next === 'all' ? null : next)
            }
          />
        </div>

        {/* Stats summary (pushed to right) */}
        {stats && (
          <div className="ml-auto flex items-stretch border border-rule divide-x divide-rule-2">
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
        )}
      </div>

      {/* Row 2: time range + sort + sort order */}
      <div className="flex items-center gap-3 flex-wrap">
        <label className="flex items-center gap-2">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            time
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

        <label className="flex items-center gap-2">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            <ArrowUpDown className="w-3 h-3 inline -mt-px mr-1" />
            sort
          </span>
          <select
            className={selectClass}
            value={filters.sortBy ?? 'start_time'}
            onChange={(e) =>
              onFilterChange(
                'sortBy',
                e.target.value as 'start_time' | 'duration' | 'service_name',
              )
            }
          >
            {SORT_BY_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </label>

        <ModeToggle<'asc' | 'desc'>
          value={sortOrderValue}
          options={SORT_ORDER_OPTIONS}
          onChange={(next) => onFilterChange('sortOrder', next)}
        />

        {/* Duration min/max inline inputs */}
        <label className="flex items-center gap-1.5">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            <Timer className="w-3 h-3 inline -mt-px mr-1" />
            dur
          </span>
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
            className={cn(inputClass, 'w-20 tabular-nums')}
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
            className={cn(inputClass, 'w-20 tabular-nums')}
          />
          <span className="text-ink-ghost font-mono text-[10px]">ms</span>
        </label>
      </div>

      {/* Row 3: service + operation free-form text */}
      <div className="flex items-center gap-3 flex-wrap">
        <label className="flex items-center gap-2 flex-1 min-w-[200px]">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
            service
          </span>
          <input
            type="text"
            placeholder="e.g. api-*, backend"
            value={tempServiceName}
            onChange={(e) =>
              dispatchTempInputs({
                type: 'SET_SERVICE_NAME',
                payload: e.target.value,
              })
            }
            onBlur={applyService}
            onKeyDown={(e) => handleEnterApply(e, applyService)}
            className={cn(inputClass, 'flex-1')}
          />
        </label>
        <label className="flex items-center gap-2 flex-1 min-w-[200px]">
          <span
            className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]"
            aria-hidden
          >
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
            className={cn(inputClass, 'flex-1')}
          />
        </label>
      </div>

      {/* Attributes editor — wrapped in `<details>` so the editor stays
          out of the way until the user opts in. */}
      <details className="iii-details border border-rule-2 bg-bg">
        <summary className="flex items-center gap-2 px-3 py-2 font-mono text-[12px] text-ink-faint hover:text-ink transition-colors lowercase select-none">
          <span
            aria-hidden
            className="iii-chev text-ink-ghost w-[8px] inline-block"
          >
            ▸
          </span>
          <span>
            attributes
            {filters.attributes && filters.attributes.length > 0 ? (
              <span className="ml-1 text-accent tabular-nums">
                ({filters.attributes.length})
              </span>
            ) : null}
          </span>
        </summary>
        <div className="px-3 pb-3 border-t border-rule-2 pt-3">
          <AttributesFilter
            value={filters.attributes || []}
            onChange={(attrs) =>
              onFilterChange('attributes', attrs.length > 0 ? attrs : undefined)
            }
          />
        </div>
      </details>

      {/* Applied Filter Chips */}
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
    </div>
  )
}
