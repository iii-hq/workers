import type { ReactNode } from 'react'
import { FilterChip } from '@/components/chat/engine/shared'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { TriggerActivityMessage } from '@/types/injectable-ui'
import {
  firstRenderedTriggerActivity,
  useTriggerActivityRenderers,
} from './renderer-registry'

export function TriggerSource({
  activity,
  presentation = 'default',
}: {
  activity: TriggerActivityMessage
  presentation?: 'default' | 'compact'
}) {
  const renderers = useTriggerActivityRenderers()
  const rendered = firstRenderedTriggerActivity(renderers, activity)
  return (
    <div
      className={cn('min-w-0', presentation === 'compact' && 'contents')}
      data-trigger-source={activity.triggerType}
    >
      {rendered?.node ?? (
        <GenericTriggerSource activity={activity} presentation={presentation} />
      )}
    </div>
  )
}

function GenericTriggerSource({
  activity,
  presentation,
}: {
  activity: TriggerActivityMessage
  presentation: 'default' | 'compact'
}) {
  const config = objectOf(activity.config)
  const entries = config ? Object.entries(config) : []
  const scalarEntries = entries.filter(([, value]) => isDisplayScalar(value))
  const nestedEntries = entries.filter(([, value]) => !isDisplayScalar(value))
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
            {activity.triggerType}
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
        ) : activity.config === undefined || isEmptyObject(config) ? (
          <span className="font-mono text-[11px] text-ink-ghost">
            · no configuration
          </span>
        ) : null}
      </div>
      {nestedEntries.length > 0 ||
      (activity.config !== undefined && !config) ? (
        <SourceJson
          value={
            nestedEntries.length > 0
              ? Object.fromEntries(nestedEntries)
              : activity.config
          }
          compact={presentation === 'compact'}
        />
      ) : null}
    </div>
  )
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

const isEmptyObject = (value: Record<string, unknown> | null) =>
  value !== null && Object.keys(value).length === 0

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
    return '[unserializable configuration]'
  }
}

export function TriggerSourceSection({
  label,
  activity,
}: {
  label: ReactNode
  activity: TriggerActivityMessage
}) {
  return (
    <>
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 font-sans text-[11px] font-semibold text-ink-faint">
        {label}
      </div>
      <TriggerSource activity={activity} />
    </>
  )
}
