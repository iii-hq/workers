import type { ReactNode } from 'react'
import { FilterChip } from '@/components/chat/engine/shared'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { ObserverActivityMessage } from '@/types/injectable-ui'
import {
  firstRenderedObserverActivity,
  useObserverActivityRenderers,
} from './renderer-registry'

export function ObserverSource({
  activity,
  presentation = 'default',
}: {
  activity: ObserverActivityMessage
  presentation?: 'default' | 'compact'
}) {
  const renderers = useObserverActivityRenderers()
  const rendered = firstRenderedObserverActivity(renderers, activity)
  return (
    <div
      className={cn('min-w-0', presentation === 'compact' && 'contents')}
      data-observer-source={activity.functionId}
    >
      {rendered?.node ?? (
        <GenericObserverSource
          activity={activity}
          presentation={presentation}
        />
      )}
    </div>
  )
}

/**
 * Any observed function: its args (the worker's setup — a schedule, a
 * route) as chips, then its filters (which calls reach the observer) as
 * `field op value` chips. Nested args fall back to JSON so nothing is
 * silently dropped.
 */
function GenericObserverSource({
  activity,
  presentation,
}: {
  activity: ObserverActivityMessage
  presentation: 'default' | 'compact'
}) {
  const args = objectOf(activity.args)
  const entries = args ? Object.entries(args) : []
  const scalarEntries = entries.filter(([, value]) => isDisplayScalar(value))
  const nestedEntries = entries.filter(([, value]) => !isDisplayScalar(value))
  const filters = filterChips(activity.filters)
  const empty =
    scalarEntries.length === 0 &&
    nestedEntries.length === 0 &&
    filters.length === 0
  return (
    <div
      className={cn(
        'min-w-0 bg-bg',
        presentation === 'compact' && 'bg-transparent',
      )}
    >
      <div
        className={cn(
          'flex flex-col gap-1.5 border-b border-rule-2 px-3 py-2',
          presentation === 'compact' && 'border-b-0 p-0',
        )}
      >
        {presentation === 'default' ? (
          <span className="font-mono text-[13px] break-all text-ink">
            {activity.functionId}
          </span>
        ) : null}
        {scalarEntries.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {scalarEntries.map(([label, value]) => (
              <FilterChip
                key={label}
                label={label}
                value={displayScalar(value)}
              />
            ))}
          </div>
        ) : null}
        {filters.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {filters.map(({ field, value }) => (
              <FilterChip key={field} label={field} value={value} />
            ))}
          </div>
        ) : null}
        {empty ? (
          <span className="font-mono text-[11px] text-ink-ghost">
            · every call
          </span>
        ) : null}
      </div>
      {nestedEntries.length > 0 ? (
        <SourceJson
          value={Object.fromEntries(nestedEntries)}
          compact={presentation === 'compact'}
        />
      ) : null}
    </div>
  )
}

const FILTER_OPERATORS: Readonly<Record<string, string>> = {
  $eq: '=',
  $ne: '≠',
  $gt: '>',
  $gte: '≥',
  $lt: '<',
  $lte: '≤',
  $in: 'in',
  $nin: 'not in',
  $exists: 'exists',
  $prefix: 'starts with',
  $contains: 'contains',
}

/**
 * Filters as `{ field, value }` chips: `{ 'input.scope': { $eq: 'run' } }`
 * reads `input.scope = run`. A field with an operator this view does not
 * know shows its operand as JSON.
 */
export function filterChips(
  filters: unknown,
): { field: string; value: string }[] {
  const record = objectOf(filters)
  if (!record) return []
  return Object.entries(record).map(([field, condition]) => {
    const ops = objectOf(condition)
    if (!ops) return { field, value: displayValue(condition) }
    const value = Object.entries(ops)
      .map(([op, operand]) =>
        `${FILTER_OPERATORS[op] ?? op} ${displayValue(operand)}`.trim(),
      )
      .join(' · ')
    return { field, value }
  })
}

const displayValue = (value: unknown): string => {
  if (isDisplayScalar(value)) return displayScalar(value)
  try {
    return JSON.stringify(value) ?? String(value)
  } catch {
    return String(value)
  }
}

function SourceJson({
  value,
  compact = false,
}: {
  value: unknown
  compact?: boolean
}) {
  const json = safeJson(value)
  return (
    <div
      className={cn(
        'max-h-64 overflow-auto',
        compact
          ? 'rounded-sm border border-edge bg-bg'
          : 'border-b border-rule-2',
      )}
    >
      <JsonHighlight code={json} wrap />
    </div>
  )
}

const objectOf = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null

const isScalar = (value: unknown) =>
  value === null ||
  typeof value === 'string' ||
  typeof value === 'number' ||
  typeof value === 'boolean'

const isDisplayScalar = (value: unknown) =>
  isScalar(value) || (Array.isArray(value) && value.every(isScalar))

const displayScalar = (value: unknown) =>
  Array.isArray(value) ? value.map(String).join(', ') : String(value)

const safeJson = (value: unknown): string => {
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return '[unserializable args]'
  }
}

export function ObserverSourceSection({
  label,
  activity,
}: {
  label: ReactNode
  activity: ObserverActivityMessage
}) {
  return (
    <>
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 font-sans text-[11px] font-semibold text-ink-faint">
        {label}
      </div>
      <ObserverSource activity={activity} />
    </>
  )
}
