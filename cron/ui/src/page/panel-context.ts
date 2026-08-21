import type { JsonValue } from '@iii-dev/console-ui'

/** Opens the page on a specific schedule (the palette source) or straight
    into the new-schedule form (the setup-time command). */
export type CronPanelContext =
  | { action: 'new' }
  | { action: 'schedule'; subscriptionId: string }

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value
    : null
}

export function parseCronPanelContext(
  value: JsonValue,
): CronPanelContext | null {
  const record = asRecord(value)
  if (!record) return null
  if (record.action === 'new') return { action: 'new' }
  if (
    record.action === 'schedule' &&
    typeof record.subscriptionId === 'string' &&
    record.subscriptionId !== ''
  )
    return { action: 'schedule', subscriptionId: record.subscriptionId }
  return null
}
