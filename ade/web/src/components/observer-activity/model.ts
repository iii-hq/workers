import { deliveryOf, type SessionObserverInfo } from '@/lib/backend/observers'
import { activityIdPrefix } from '@/lib/sessions/stored-records'
import type { ObserverFiredData, SystemMessage } from '@/types/chat'
import type { ObserverActivityMessage } from '@/types/injectable-ui'

export interface ObserverRegistration {
  /** Normalized contract passed to source-specific renderers. */
  activity: ObserverActivityMessage
  /** Original registration shape shown in the host-owned raw pane. */
  raw: unknown
  /** Compatibility hint used in pane chrome. */
  summary?: string
  /** Compatibility alias while registration views share one implementation. */
  detail: unknown
}

const recordOf = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null

const stringOf = (value: unknown): string | undefined =>
  typeof value === 'string' && value.length > 0 ? value : undefined

const numberOf = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined

/** Split the Harness wake envelope into its label and event object. */
export function parseNotification(
  content: string,
): { name?: string; payload: unknown } | null {
  const envelope = /^\[notification\]\s*([\s\S]+?)\s*$/.exec(content)
  if (!envelope) return null
  const body = envelope[1]
  const labeled = /^([^:]+):\s*([\s\S]+)$/.exec(body)
  try {
    if (labeled) {
      return {
        name: labeled[1].trim(),
        payload: JSON.parse(labeled[2]) as unknown,
      }
    }
  } catch {
    // The text may be an unlabeled object whose first colon looked like the
    // label delimiter. Retry the full body before treating it as prose.
  }
  try {
    return { payload: JSON.parse(body) as unknown }
  } catch {
    return null
  }
}

function deliveryFromRegistration(
  request: Record<string, unknown>,
): ObserverActivityMessage['delivery'] {
  const target = recordOf(request.target)
  const functionId = stringOf(target?.function_id)
  return functionId ? { kind: 'call', functionId } : { kind: 'notify' }
}

/** Normalize an agent's `harness::observe` request. */
export function registrationFromCall({
  id,
  input,
  observerId,
  effectiveOnce,
  note,
}: {
  id: string
  input: unknown
  observerId?: string
  effectiveOnce?: boolean
  note?: string
}): ObserverRegistration {
  const request = recordOf(input) ?? {}
  const lifecycle = recordOf(request.lifecycle)
  const once =
    effectiveOnce ??
    (typeof request.once === 'boolean'
      ? request.once
      : typeof lifecycle?.once === 'boolean'
        ? lifecycle.once
        : false)
  const functionId = stringOf(request.function_id) ?? 'observer'
  const args = recordOf(request.args)
  const filters = recordOf(request.filters)
  const activity: ObserverActivityMessage = {
    id,
    kind: 'registration',
    functionId,
    ...(args ? { args } : {}),
    ...(filters ? { filters } : {}),
    ...(stringOf(request.label) ? { label: stringOf(request.label) } : {}),
    ...(stringOf(request.action) ? { action: stringOf(request.action) } : {}),
    ...(Array.isArray(request.conditions)
      ? { conditions: request.conditions }
      : {}),
    delivery: deliveryFromRegistration(request),
    lifecycle: {
      state: 'active',
      once,
      fires: 0,
      ...(numberOf(lifecycle?.max_fires) !== undefined
        ? { maxFires: numberOf(lifecycle?.max_fires) }
        : {}),
    },
    ...(observerId ? { observerId } : {}),
    ...(note ? { note } : {}),
  }
  return {
    activity,
    raw: input,
    summary: 'from observe call',
    detail: input,
  }
}

/** Normalize the Harness's live/last-seen observer row. */
export function registrationFromRow(
  row: SessionObserverInfo,
): ObserverRegistration {
  const raw = {
    function_id: row.functionId,
    ...(row.args !== undefined ? { args: row.args } : {}),
    ...(row.filters !== undefined ? { filters: row.filters } : {}),
    conditions: row.conditions?.length ? row.conditions : undefined,
    once: row.once,
    label: row.label,
    action: row.action,
    target:
      row.delivery.kind === 'call'
        ? { function_id: row.delivery.functionId }
        : undefined,
  }
  const activity: ObserverActivityMessage = {
    id: `observer-registration:${row.id}`,
    kind: 'registration',
    functionId: row.functionId,
    ...(row.args !== undefined ? { args: row.args } : {}),
    ...(row.filters !== undefined ? { filters: row.filters } : {}),
    ...(row.label ? { label: row.label } : {}),
    ...(row.action ? { action: row.action } : {}),
    ...(row.conditions?.length ? { conditions: row.conditions } : {}),
    delivery: row.delivery,
    lifecycle: {
      state: row.fired ? 'retired' : 'active',
      once: row.once ?? false,
      fires: row.fires ?? 0,
      ...(row.maxFires !== undefined ? { maxFires: row.maxFires } : {}),
      ...(row.expiresAt !== undefined ? { expiresAt: row.expiresAt } : {}),
    },
    observerId: row.id,
    ...(row.firedAt !== undefined ? { firedAt: row.firedAt } : {}),
  }
  return { activity, raw, summary: row.functionId, detail: raw }
}

function kindOfRecord(
  messageId: string,
  record: ObserverFiredData,
): ObserverActivityMessage['kind'] {
  if (
    record.outcome === 'expired' ||
    record.outcome === 'unregistered' ||
    record.outcome === 'invalidated' ||
    record.retirement_reason === 'expired' ||
    record.retirement_reason === 'unregistered' ||
    record.retirement_reason === 'invalidated' ||
    record.retirement_reason === 'exhausted' ||
    activityIdPrefix(messageId, 'expired') !== null ||
    activityIdPrefix(messageId, 'stale') !== null
  ) {
    return 'retirement'
  }
  return 'fired'
}

/** Fire ids carry the observer's one-based fire counter. */
function fireCountFromEntryId(
  messageId: string,
  observerId: string,
): number | undefined {
  const prefix = activityIdPrefix(messageId, 'fired')
  if (!prefix || !messageId.startsWith(`${prefix}${observerId}_`)) {
    return undefined
  }
  const ordinal = messageId.slice(prefix.length + observerId.length + 1)
  if (!/^\d+$/.test(ordinal)) return undefined
  return Number(ordinal)
}

/** A skipped fire whose condition answered "not yet" on an observer that stays
 * armed (a barrier still waiting on keys): nothing failed. The harness puts the
 * skip's gate in the record id, `e_obsskip_<observer>_<gate>_<uuid>`; a failed
 * condition call uses `condition-error`, `condition-policy` or
 * `condition-approval` instead of `condition`. */
export function isConditionWait(activity: ObserverActivityMessage): boolean {
  return (
    activity.outcome === 'skipped' &&
    activity.lifecycle.state === 'active' &&
    activityIdPrefix(activity.id, 'skip') !== null &&
    /_condition_[0-9a-f]{32}$/.test(activity.id)
  )
}

/** The state watch a record names, as the observer's filters. */
function stateFilters(
  record: ObserverFiredData,
): Record<string, unknown> | undefined {
  const filters: Record<string, unknown> = {}
  if (typeof record.scope === 'string') {
    filters['input.scope'] = { $eq: record.scope }
  }
  if (typeof record.key === 'string') {
    filters['input.key'] = { $eq: record.key }
  }
  return Object.keys(filters).length > 0 ? filters : undefined
}

/** Combine a durable fire/retirement record with recovered registration data. */
export function activityFromObserverRecord(
  message: SystemMessage,
  registration?: ObserverRegistration,
): ObserverActivityMessage | null {
  const record = message.observer
  if (!record) return null
  const inherited = registration?.activity
  const isState = typeof record.key === 'string'
  const functionId =
    record.function_id ??
    inherited?.functionId ??
    (isState ? 'state::on-changed' : 'observer')
  const args = record.args ?? inherited?.args
  const filters =
    record.filters ??
    inherited?.filters ??
    (isState ? stateFilters(record) : undefined)
  const delivery = record.target
    ? deliveryOf(record.target)
    : (inherited?.delivery ?? { kind: 'notify' as const })
  const kind = kindOfRecord(message.id, record)
  const historicalFireCount = fireCountFromEntryId(
    message.id,
    record.observer_id,
  )
  const legacyDeliveredFire =
    kind === 'fired' &&
    (record.outcome === undefined ||
      record.outcome === 'delivered' ||
      record.outcome === 'delivery_failed')
  return {
    id: message.id,
    kind,
    functionId,
    ...(args !== undefined ? { args } : {}),
    ...(filters !== undefined ? { filters } : {}),
    ...((record.label ?? inherited?.label)
      ? { label: record.label ?? inherited?.label }
      : {}),
    ...((record.action ?? inherited?.action)
      ? { action: record.action ?? inherited?.action }
      : {}),
    ...(inherited?.conditions ? { conditions: inherited.conditions } : {}),
    delivery,
    lifecycle: {
      state: record.retired ? 'retired' : 'active',
      once: record.once,
      fires:
        record.fires ??
        historicalFireCount ??
        (legacyDeliveredFire
          ? Math.max(inherited?.lifecycle.fires ?? 0, 1)
          : (inherited?.lifecycle.fires ?? 0)),
      ...(inherited?.lifecycle.maxFires !== undefined
        ? { maxFires: inherited.lifecycle.maxFires }
        : {}),
      ...(inherited?.lifecycle.expiresAt !== undefined
        ? { expiresAt: inherited.lifecycle.expiresAt }
        : {}),
    },
    observerId: record.observer_id,
    ...(record.payload !== undefined ? { payload: record.payload } : {}),
    ...(record.fired_at !== undefined ? { firedAt: record.fired_at } : {}),
    ...(record.note ? { note: record.note } : {}),
    ...(record.outcome ? { outcome: record.outcome } : {}),
    ...(record.retirement_reason
      ? { retirementReason: record.retirement_reason }
      : {}),
  }
}
