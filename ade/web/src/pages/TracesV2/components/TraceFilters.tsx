// Trace filter bar, on shadcn.
//
// Layout: one row of always-visible controls — the saved-views switcher,
// search, group-by, a "Filters" popover trigger (with a count badge) and
// the page's stats — with the active filters as removable chips below.
// The advanced filters (status, time range, sort, duration, worker,
// operation, display, attributes) live inside the popover.
//
// Behaviors preserved:
//   • 300ms-debounced text inputs (workerName, operationName) — wired
//     via the parent useTraceFilters hook; here they apply on blur/Enter.
//   • Min/max duration validation warnings.
//   • Group-by switching (server-side aggregation).
//   • Status filtering (ok / error / unset / all).
//   • Sort by column + asc/desc.
//   • Time range presets.
//   • Attribute key/value editor (via AttributesFilter).
//   • Active-filter chips for one-click removal.
//   • Stats summary (global total, current-page errors and average).

import {
  AlertTriangle,
  ArrowUp,
  Hash,
  Search,
  SlidersHorizontal,
  Timer,
  X,
  XCircle,
} from 'lucide-react'
import { useEffect, useReducer, useState } from 'react'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import { Input } from '@/components/shadcn/input'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import { Tabs, TabsList, TabsTrigger } from '@/components/shadcn/tabs'
import { cn } from '@/lib/utils'
import type { TraceFilterState } from '../hooks/useTraceFilters'
import type { GroupByOption } from '../lib/groupTraces'
import { AttributesFilter } from './AttributesFilter'
import { GroupByPicker } from './GroupByPicker'

export interface TraceFiltersProps {
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
    pageTraceCount: number
    errorCount: number
    avgDuration: number
  }
  /**
   * Rows the live list is holding back while the user reads it (see
   * index.tsx). Rendered as one more segment of the stats block — a quiet
   * "N new" next to the counts rather than anything over the rows — that
   * lands them on click.
   */
  heldNew?: HeldNewTraces
  /** Rendered first in the control row (the saved-views dropdown). */
  leading?: React.ReactNode
  /**
   * Attribute keys observed on loaded traces (row attributes + trace tags),
   * offered as group-by suggestions. Grouping accepts ANY key — these just
   * make the known ones one click.
   */
  attributeKeySuggestions?: string[]
}

const TIME_RANGE_PRESETS: Array<{ label: string; ms: number | null }> = [
  { label: 'All time', ms: null },
  { label: 'Last 15 minutes', ms: 15 * 60 * 1000 },
  { label: 'Last hour', ms: 60 * 60 * 1000 },
  { label: 'Last 6 hours', ms: 6 * 60 * 60 * 1000 },
  { label: 'Last 24 hours', ms: 24 * 60 * 60 * 1000 },
  { label: 'Last 7 days', ms: 7 * 24 * 60 * 60 * 1000 },
]

const SORT_BY_OPTIONS: Array<{
  label: string
  value: 'start_time' | 'duration' | 'service_name'
}> = [
  { label: 'Start time', value: 'start_time' },
  { label: 'Duration', value: 'duration' },
  { label: 'Worker name', value: 'service_name' },
]

type StatusValue = 'all' | 'ok' | 'error' | 'unset'

const STATUS_OPTIONS: Array<{ value: StatusValue; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'ok', label: 'OK' },
  { value: 'error', label: 'Error' },
  { value: 'unset', label: 'Unset' },
]

const LABEL_MODE_OPTIONS: Array<{
  value: NonNullable<TraceFilterState['labelMode']>
  label: string
}> = [
  { value: 'function', label: 'Function name' },
  { value: 'span-name', label: 'Span name' },
  { value: 'attribute', label: 'Attribute value' },
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

/** A field in the filters popover: a quiet label over its control. */
function Field({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </div>
  )
}

export function SearchInput({
  searchQuery,
  onSearchChange,
  className,
}: {
  searchQuery: string
  onSearchChange?: (value: string) => void
  className?: string
}) {
  return (
    <div className={cn('relative', className)}>
      <Search
        className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
      <Input
        type="search"
        value={searchQuery}
        onChange={(e) => onSearchChange?.(e.target.value)}
        placeholder="Search traces"
        aria-label="search traces"
        className="rounded-xl bg-muted/60 pr-8 pl-8 dark:bg-muted/40"
      />
      {searchQuery ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          onClick={() => onSearchChange?.('')}
          aria-label="clear search"
          className="absolute top-1/2 right-1 -translate-y-1/2"
        >
          <X />
        </Button>
      ) : null}
    </div>
  )
}

export interface HeldNewTraces {
  /** held rows that will show once landed */
  count: number
  /** on a later page the segment offers the latest traces on page 1 */
  onPage1: boolean
  onShow: () => void
}

/** A stat in the toolbar: a glyph, the number in mono, a quiet label. */
function Stat({
  icon,
  value,
  label,
  title,
  tone,
}: {
  icon: React.ReactNode
  value: string
  label: string
  title?: string
  tone?: 'alert'
}) {
  return (
    <div
      className={cn(
        'flex items-center gap-1.5 px-2.5 text-xs',
        tone === 'alert' ? 'text-destructive' : 'text-muted-foreground',
      )}
      title={title}
    >
      {icon}
      <span
        className={cn(
          'font-mono tabular-nums',
          tone === 'alert' ? 'text-destructive' : 'text-foreground',
        )}
      >
        {value}
      </span>
      <span>{label}</span>
    </div>
  )
}

export function StatsBlock({
  stats,
  heldNew,
}: {
  stats: TraceFiltersProps['stats']
  heldNew?: HeldNewTraces
}) {
  if (!stats) return null
  const held = heldNew && heldNew.count > 0 ? heldNew : null
  return (
    <div className="flex h-8 items-stretch divide-x divide-border rounded-xl bg-muted/60 dark:bg-muted/40">
      {held ? (
        <button
          type="button"
          onClick={held.onShow}
          data-trace-list-held={held.count}
          className="flex items-center gap-1.5 rounded-l-xl px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-muted"
          title={
            held.onPage1
              ? `${held.count} new ${held.count === 1 ? 'trace' : 'traces'} waiting — show them at the top`
              : 'new traces arrived — show the latest on page 1'
          }
          aria-label={
            held.onPage1
              ? `show ${held.count} new ${held.count === 1 ? 'trace' : 'traces'}`
              : 'show the latest traces on page 1'
          }
        >
          <ArrowUp className="size-4" aria-hidden />
          {held.onPage1 ? (
            <>
              <span className="font-mono tabular-nums">{held.count}</span>
              <span>new</span>
            </>
          ) : (
            <span>latest</span>
          )}
        </button>
      ) : null}
      <Stat
        icon={<Hash className="size-4" aria-hidden />}
        value={String(stats.totalTraces)}
        label="Traces"
        title={`${stats.pageTraceCount} traces on this page, ${stats.totalTraces} total matching traces`}
      />
      <Stat
        icon={<XCircle className="size-4" aria-hidden />}
        value={String(stats.errorCount)}
        label="Page errors"
        tone={stats.errorCount > 0 ? 'alert' : undefined}
      />
      <Stat
        icon={<Timer className="size-4" aria-hidden />}
        value={formatDurationMs(stats.avgDuration)}
        label="Page avg"
      />
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
  leading,
  heldNew,
  attributeKeySuggestions = [],
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

  // Display: label-attribute key is typed then applied on blur/Enter, like
  // the other text filters (but simple enough not to join the reducer).
  const [tempLabelAttribute, setTempLabelAttribute] = useState(
    filters.labelAttribute ?? '',
  )
  const [prevLabelAttribute, setPrevLabelAttribute] = useState(
    filters.labelAttribute,
  )
  if (prevLabelAttribute !== filters.labelAttribute) {
    setPrevLabelAttribute(filters.labelAttribute)
    setTempLabelAttribute(filters.labelAttribute ?? '')
  }
  const applyLabelAttribute = () => {
    onFilterChange('labelAttribute', tempLabelAttribute.trim() || undefined)
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
      case 'display':
        onFilterChange('labelMode', 'function')
        onFilterChange('labelAttribute', undefined)
        setTempLabelAttribute('')
        break
    }
  }

  const unhideFunction = (fn: string) => {
    onFilterChange(
      'hiddenFunctions',
      (filters.hiddenFunctions ?? []).filter((f) => f !== fn),
    )
  }

  const activeFilters: Array<{
    key: string
    label: string
    onRemove?: () => void
  }> = []
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
      label: preset ? preset.label.toLowerCase() : 'custom range',
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
      label: `sort: ${(opt?.label ?? filters.sortBy).toLowerCase()}`,
    })
  }
  if (filters.sortOrder && filters.sortOrder !== 'desc') {
    activeFilters.push({
      key: 'sortOrder',
      label: `order: ${filters.sortOrder}`,
    })
  }
  if (filters.labelMode && filters.labelMode !== 'function') {
    activeFilters.push({
      key: 'display',
      label:
        filters.labelMode === 'attribute'
          ? `display: ${filters.labelAttribute || 'attribute'}`
          : 'display: span name',
    })
  }
  for (const fn of filters.hiddenFunctions ?? []) {
    activeFilters.push({
      key: `hidden:${fn}`,
      label: `hidden: ${fn}`,
      onRemove: () => unhideFunction(fn),
    })
  }

  const statusValue: StatusValue = (filters.status ?? 'all') as StatusValue
  const groupByValue = (filters.groupBy ?? 'none') as GroupByOption
  const sortOrderValue = filters.sortOrder ?? 'desc'
  const advancedFilterCount = activeFilters.length

  const [popoverOpen, setPopoverOpen] = useState(false)

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
    <div className="flex flex-col gap-2" data-testid="trace-filters">
      {(validationWarnings?.durationSwapped ||
        validationWarnings?.timeRangeSwapped) && (
        <div className="flex items-center gap-2 rounded-xl bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
          <AlertTriangle className="size-4 shrink-0" aria-hidden />
          <span className="flex-1">
            Values corrected:{' '}
            {validationWarnings.durationSwapped &&
              'min and max duration were swapped. '}
            {validationWarnings.timeRangeSwapped &&
              'start and end time were swapped.'}
          </span>
          {onClearWarnings && (
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              onClick={onClearWarnings}
              aria-label="dismiss warning"
            >
              <X />
            </Button>
          )}
        </div>
      )}

      {/* The toolbar: views, search, group-by, filters, stats. */}
      <div className="flex flex-wrap items-center gap-2">
        {leading}

        <SearchInput
          searchQuery={searchQuery}
          onSearchChange={onSearchChange}
          className="min-w-[240px] max-w-[460px] flex-1"
        />

        <GroupByPicker
          value={groupByValue}
          onChange={(next) => onFilterChange('groupBy', next)}
          suggestions={attributeKeySuggestions}
        />

        <Popover open={popoverOpen} onOpenChange={setPopoverOpen}>
          <PopoverTrigger
            render={
              <Button
                type="button"
                variant={advancedFilterCount > 0 ? 'secondary' : 'outline'}
                className="rounded-xl"
                aria-label="more filters"
              />
            }
          >
            <SlidersHorizontal data-icon="inline-start" />
            Filters
            {advancedFilterCount > 0 ? (
              <Badge className="ml-0.5 h-4 min-w-4 px-1 font-mono tabular-nums">
                {advancedFilterCount}
              </Badge>
            ) : null}
          </PopoverTrigger>
          <PopoverContent
            align="start"
            collisionPadding={8}
            className="w-[min(34rem,calc(100vw-1rem))] gap-0 p-0"
          >
            <div className="flex items-center justify-between px-4 pt-3 pb-2">
              <span className="text-sm font-semibold">Filters</span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={onClear}
                className="-mr-2"
              >
                Clear all
              </Button>
            </div>

            <div className="grid grid-cols-2 gap-3 px-4 pb-4">
              <Field label="Status" className="col-span-2">
                <Tabs
                  value={statusValue}
                  onValueChange={(next) =>
                    onFilterChange(
                      'status',
                      next === 'all' ? null : (next as StatusValue),
                    )
                  }
                >
                  <TabsList className="w-full">
                    {STATUS_OPTIONS.map((option) => (
                      <TabsTrigger key={option.value} value={option.value}>
                        {option.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              </Field>

              <Field label="Time range">
                <Select
                  value={String(currentTimeRangeIndex(filters))}
                  onValueChange={(value) =>
                    handleTimeRangeChange(Number.parseInt(String(value), 10))
                  }
                  items={TIME_RANGE_PRESETS.map((preset, i) => ({
                    value: String(i),
                    label: preset.label,
                  }))}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TIME_RANGE_PRESETS.map((preset, i) => (
                      <SelectItem key={preset.label} value={String(i)}>
                        {preset.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>

              <Field label="Sort">
                <div className="flex gap-2">
                  <Select
                    value={filters.sortBy ?? 'start_time'}
                    onValueChange={(value) =>
                      onFilterChange(
                        'sortBy',
                        value as 'start_time' | 'duration' | 'service_name',
                      )
                    }
                    items={SORT_BY_OPTIONS}
                  >
                    <SelectTrigger className="min-w-0 flex-1">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SORT_BY_OPTIONS.map((opt) => (
                        <SelectItem key={opt.value} value={opt.value}>
                          {opt.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Tabs
                    value={sortOrderValue}
                    onValueChange={(next) =>
                      onFilterChange('sortOrder', next as 'asc' | 'desc')
                    }
                  >
                    <TabsList>
                      <TabsTrigger value="desc">Desc</TabsTrigger>
                      <TabsTrigger value="asc">Asc</TabsTrigger>
                    </TabsList>
                  </Tabs>
                </div>
              </Field>

              <Field label="Worker">
                <Input
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
                />
              </Field>

              <Field label="Operation">
                <Input
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
                />
              </Field>

              <Field label="Duration (ms)" className="col-span-2">
                <div className="flex items-center gap-2">
                  <Input
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
                    className="font-mono tabular-nums"
                  />
                  <span className="text-xs text-muted-foreground">to</span>
                  <Input
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
                    className="font-mono tabular-nums"
                  />
                </div>
              </Field>

              <Field label="Label rows by">
                <Select
                  value={filters.labelMode ?? 'function'}
                  onValueChange={(value) =>
                    onFilterChange(
                      'labelMode',
                      value as TraceFilterState['labelMode'],
                    )
                  }
                  items={LABEL_MODE_OPTIONS}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {LABEL_MODE_OPTIONS.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              {filters.labelMode === 'attribute' ? (
                <Field label="Attribute key">
                  <Input
                    type="text"
                    placeholder="e.g. iii.tag.message"
                    value={tempLabelAttribute}
                    onChange={(e) => setTempLabelAttribute(e.target.value)}
                    onBlur={applyLabelAttribute}
                    onKeyDown={(e) => handleEnterApply(e, applyLabelAttribute)}
                    className="font-mono"
                  />
                </Field>
              ) : null}

              <Field label="Attributes" className="col-span-2">
                <AttributesFilter
                  value={filters.attributes || []}
                  onChange={(attrs) =>
                    onFilterChange(
                      'attributes',
                      attrs.length > 0 ? attrs : undefined,
                    )
                  }
                />
              </Field>
            </div>
          </PopoverContent>
        </Popover>

        <div className="ml-auto">
          <StatsBlock stats={stats} heldNew={heldNew} />
        </div>
      </div>

      {/* Active filter chips */}
      {activeFilters.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          {isLoading && (
            <span
              aria-hidden
              className="mr-0.5 size-1.5 rounded-full bg-sky-500 pulse-dot"
            />
          )}
          {activeFilters.map((filter) => (
            <Badge
              key={filter.key}
              variant="secondary"
              className="h-6 gap-1 pr-1 pl-2.5 font-normal"
            >
              <span>{filter.label}</span>
              <button
                type="button"
                onClick={() =>
                  filter.onRemove ? filter.onRemove() : removeFilter(filter.key)
                }
                aria-label={`remove ${filter.label}`}
                className="flex size-5 items-center justify-center rounded-full text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
              >
                <X className="size-4!" aria-hidden />
              </button>
            </Badge>
          ))}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onClear}
            className="ml-0.5"
          >
            Clear all
          </Button>
        </div>
      )}
    </div>
  )
}
