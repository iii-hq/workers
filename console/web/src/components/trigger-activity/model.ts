import { deliveryOf, type SessionTriggerInfo } from '@/lib/backend/triggers'
import type { SystemMessage, TriggerFiredData } from '@/types/chat'
import type { TriggerActivityMessage } from '@/types/injectable-ui'

export interface TriggerRegistration {
  /** Normalized contract passed to source-specific renderers. */
  activity: TriggerActivityMessage
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
): TriggerActivityMessage['delivery'] {
  const target = recordOf(request.target)
  const functionId =
    stringOf(target?.function_id) ?? stringOf(request.function_id)
  return functionId ? { kind: 'call', functionId } : { kind: 'notify' }
}

/** Normalize a historical `engine::register_trigger` request. */
export function registrationFromCall({
  id,
  input,
  subscriptionId,
  effectiveOnce,
  note,
}: {
  id: string
  input: unknown
  subscriptionId?: string
  effectiveOnce?: boolean
  note?: string
}): TriggerRegistration {
  const request = recordOf(input) ?? {}
  const lifecycle = recordOf(request.lifecycle)
  const once =
    effectiveOnce ??
    (typeof request.once === 'boolean'
      ? request.once
      : typeof lifecycle?.once === 'boolean'
        ? lifecycle.once
        : false)
  const triggerType = stringOf(request.trigger_type) ?? 'trigger'
  const activity: TriggerActivityMessage = {
    id,
    kind: 'registration',
    triggerType,
    ...(request.config !== undefined ? { config: request.config } : {}),
    ...(stringOf(request.label) ? { label: stringOf(request.label) } : {}),
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
      ...(numberOf(lifecycle?.expires_at) !== undefined
        ? { expiresAt: numberOf(lifecycle?.expires_at) }
        : {}),
    },
    ...(subscriptionId ? { subscriptionId } : {}),
    ...(note ? { note } : {}),
  }
  return {
    activity,
    raw: input,
    summary: 'from register call',
    detail: input,
  }
}

/** Normalize the Harness's live/last-seen binding row. */
export function registrationFromRow(
  row: SessionTriggerInfo,
): TriggerRegistration {
  const raw = {
    config: row.config,
    conditions: row.conditions?.length ? row.conditions : undefined,
    once: row.once,
    label: row.label,
    function_id:
      row.delivery.kind === 'call' ? row.delivery.functionId : undefined,
  }
  const activity: TriggerActivityMessage = {
    id: `trigger-registration:${row.id}`,
    kind: 'registration',
    triggerType: row.triggerType,
    ...(row.config !== undefined ? { config: row.config } : {}),
    ...(row.label ? { label: row.label } : {}),
    ...(row.conditions?.length ? { conditions: row.conditions } : {}),
    delivery: row.delivery,
    lifecycle: {
      state: row.fired ? 'retired' : 'active',
      once: row.once ?? false,
      fires: row.fires ?? 0,
      ...(row.maxFires !== undefined ? { maxFires: row.maxFires } : {}),
      ...(row.expiresAt !== undefined ? { expiresAt: row.expiresAt } : {}),
    },
    subscriptionId: row.id,
    ...(row.triggerId ? { triggerId: row.triggerId } : {}),
    ...(row.firedAt !== undefined ? { firedAt: row.firedAt } : {}),
  }
  return { activity, raw, summary: row.triggerType, detail: raw }
}

function kindOfRecord(
  messageId: string,
  trigger: TriggerFiredData,
): TriggerActivityMessage['kind'] {
  if (
    trigger.outcome === 'expired' ||
    trigger.outcome === 'unregistered' ||
    trigger.outcome === 'invalidated' ||
    trigger.retirement_reason === 'expired' ||
    trigger.retirement_reason === 'unregistered' ||
    trigger.retirement_reason === 'invalidated' ||
    trigger.retirement_reason === 'exhausted' ||
    messageId.startsWith('e_trigexpired_') ||
    messageId.startsWith('e_trigstale_')
  ) {
    return 'retirement'
  }
  return 'fired'
}

/** Current fire ids carry the binding's one-based fire counter. */
function fireCountFromEntryId(
  messageId: string,
  subscriptionId: string,
): number | undefined {
  const prefix = `e_trigfired_${subscriptionId}_`
  if (!messageId.startsWith(prefix)) return undefined
  const ordinal = messageId.slice(prefix.length)
  if (!/^\d+$/.test(ordinal)) return undefined
  return Number(ordinal)
}

/** Combine a durable fire/retirement record with recovered registration data. */
export function activityFromTriggerRecord(
  message: SystemMessage,
  registration?: TriggerRegistration,
): TriggerActivityMessage | null {
  const trigger = message.trigger
  if (!trigger) return null
  const inherited = registration?.activity
  const isState = typeof trigger.key === 'string'
  const triggerType =
    trigger.trigger_type ??
    inherited?.triggerType ??
    (isState ? 'state' : 'trigger')
  const config =
    trigger.config !== undefined
      ? trigger.config
      : (inherited?.config ??
        (isState ? { scope: trigger.scope, key: trigger.key } : undefined))
  const delivery = trigger.target
    ? deliveryOf(trigger.target)
    : (inherited?.delivery ?? { kind: 'notify' as const })
  const kind = kindOfRecord(message.id, trigger)
  const historicalFireCount = fireCountFromEntryId(
    message.id,
    trigger.subscription_id,
  )
  const legacyDeliveredFire =
    kind === 'fired' &&
    (trigger.outcome === undefined ||
      trigger.outcome === 'delivered' ||
      trigger.outcome === 'delivery_failed')
  return {
    id: message.id,
    kind,
    triggerType,
    ...(config !== undefined ? { config } : {}),
    ...((trigger.label ?? inherited?.label)
      ? { label: trigger.label ?? inherited?.label }
      : {}),
    ...(inherited?.conditions ? { conditions: inherited.conditions } : {}),
    delivery,
    lifecycle: {
      state: trigger.retired ? 'retired' : 'active',
      once: trigger.once,
      fires:
        trigger.fires ??
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
    subscriptionId: trigger.subscription_id,
    ...(trigger.trigger_id ? { triggerId: trigger.trigger_id } : {}),
    ...(trigger.payload !== undefined ? { payload: trigger.payload } : {}),
    ...(trigger.fired_at !== undefined ? { firedAt: trigger.fired_at } : {}),
    ...(trigger.note ? { note: trigger.note } : {}),
    ...(trigger.outcome ? { outcome: trigger.outcome } : {}),
    ...(trigger.retirement_reason
      ? { retirementReason: trigger.retirement_reason }
      : {}),
  }
}
