/**
 * Session-owned trigger subscriptions, read from the harness's durable
 * binding store via `harness::triggers::list`. The engine's own trigger
 * registry cannot serve this view anymore: every agent binding registers as
 * the internal delivery hop (`harness::trigger::deliver`) with metadata that
 * is only a `__binding` pointer, so owner and shape live in the harness
 * records alone. Rows are source-generic — the raw `trigger_type` + `config`
 * plus a delivery that is either "notify the owner" or "call a plain
 * function" — so trigger sources that do not exist yet render unchanged.
 */

import type { IiiClient } from '@/lib/iii-client'
import type { TriggerFiredData } from '@/types/chat'

export type TriggerDelivery =
  | { kind: 'notify' }
  | { kind: 'call'; functionId: string }

export interface SessionTriggerInfo {
  /** Harness subscription id — the unregister handle. */
  id: string
  /** Engine-side trigger id (absent only mid-registration). */
  triggerId?: string
  /** e.g. `cron`, `state`, `timer`, `database::row-changed`, or any future source. */
  triggerType: string
  delivery: TriggerDelivery
  config?: unknown
  conditions?: unknown[]
  label?: string
  /** Human-readable event text declared as registration metadata.action. */
  action?: string
  once?: boolean
  fires?: number
  maxFires?: number
  expiresAt?: number
  createdAt?: number
  /** Latest structured trigger activity, when the transcript carries it. */
  outcome?: TriggerFiredData['outcome']
  /** Why this row became inactive. Absent on active and historical records. */
  retirementReason?: TriggerFiredData['retirement_reason']
  /**
   * Historical property name: this subscription is inactive per a retired
   * `trigger_fired` activity. Its structured reason may be consumption,
   * expiry, unregistration, invalidation, or a legacy unknown — never infer
   * one from this boolean alone. Nothing remains to unregister; ✕ dismisses.
   */
  fired?: boolean
  firedAt?: number
}

const NOTIFY_TARGET = 'harness::send'

interface TriggerRow {
  subscription_id: string
  trigger_id?: string
  trigger_type?: string
  config?: unknown
  target?: string
  conditions?: unknown[]
  label?: string
  action?: string
  once: boolean
  max_fires?: number
  expires_at?: number
  fires: number
  created_at: number
}

/**
 * Map a raw delivery target to its kind. Live rows carry the target function
 * id (absent = notify); `trigger_fired` records written before the delivery
 * hop carry the legacy words `'notify'` / `'spawn'`.
 */
export function deliveryOf(target: string | undefined | null): TriggerDelivery {
  if (!target || target === 'notify' || target === NOTIFY_TARGET)
    return { kind: 'notify' }
  if (target === 'spawn') return { kind: 'call', functionId: 'harness::spawn' }
  return { kind: 'call', functionId: target }
}

/**
 * List the subscriptions `sessionId` owns — one call, straight from the
 * store. Transport failures reject; an empty array means the store was read
 * successfully and the session owns no subscriptions.
 */
export async function listSessionTriggers(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
): Promise<SessionTriggerInfo[]> {
  const response = await client.trigger<{ subscriptions: TriggerRow[] }>(
    'harness::triggers::list',
    { session_id: sessionId },
  )
  return (response?.subscriptions ?? []).map((row) => ({
    id: row.subscription_id,
    triggerId: row.trigger_id ?? undefined,
    triggerType: row.trigger_type ?? 'trigger',
    delivery: deliveryOf(row.target),
    config: row.config,
    conditions: row.conditions,
    label: row.label ?? undefined,
    action: row.action ?? undefined,
    once: row.once,
    fires: row.fires,
    maxFires: row.max_fires ?? undefined,
    expiresAt: row.expires_at ?? undefined,
    createdAt: row.created_at,
  }))
}

/**
 * Tear a subscription down through the harness — engine trigger AND durable
 * record. A raw engine-side unregister would orphan the record and leave the
 * owner session believing in an armed wake that can never fire.
 */
export async function unregisterTrigger(
  client: Pick<IiiClient, 'trigger'>,
  subscriptionId: string,
  sessionId: string,
): Promise<void> {
  await client.trigger('harness::triggers::unregister', {
    subscription_id: subscriptionId,
    session_id: sessionId,
  })
}

/** Reconstruct an inactive subscription's panel row from its activity record. */
function firedGhostRow(t: TriggerFiredData): SessionTriggerInfo {
  const isState = typeof t.key === 'string'
  return {
    id: t.subscription_id,
    triggerId: t.trigger_id ?? undefined,
    // Enriched records carry the exact source. Historical state records can
    // still be reconstructed from their scope/key watch.
    triggerType: t.trigger_type ?? (isState ? 'state' : 'trigger'),
    delivery: deliveryOf(t.target),
    config:
      t.config !== undefined
        ? t.config
        : isState
          ? { scope: t.scope, key: t.key }
          : undefined,
    label: t.label,
    action: t.action,
    once: t.once,
    fires: t.fires,
    outcome: t.outcome,
    retirementReason: t.retirement_reason,
    fired: true,
    firedAt: t.fired_at,
  }
}

/** Overlay record-owned source/lifecycle facts while retaining richer listed
 * details such as conditions and creation time. */
function withFiredActivity(
  row: SessionTriggerInfo,
  t: TriggerFiredData,
): SessionTriggerInfo {
  return {
    ...row,
    triggerType: t.trigger_type ?? row.triggerType,
    config: t.config !== undefined ? t.config : row.config,
    action: t.action ?? row.action,
    fires: t.fires ?? row.fires,
    outcome: t.outcome,
    retirementReason: t.retirement_reason,
    ...(t.retired ? { fired: true, firedAt: t.fired_at } : {}),
  }
}

/**
 * Merge the live list with fired-subscription history, correlated on the
 * subscription id (present on both sides). A *retired* fire means the binding
 * is gone: if a not-yet-refetched list still carries it, annotate that row as
 * inactive in place; once the refetch drops it, append a greyed "ghost" row.
 * The newest non-retired activity enriches its live row without making it
 * inactive. Live rows use the newest activity; absent rows use the newest
 * retirement and ignore non-retired activity that arrived after it.
 *
 * Ghost fidelity is two-tier: prefer the full last-seen listed row (from
 * `seenRows`) for conditions and registration metadata, then overlay the
 * record-owned source/config/lifecycle fields. The enriched record-only ghost
 * remains the post-reload fallback; historical records may still be thin.
 */
export function mergeFiredTriggers(
  listed: SessionTriggerInfo[],
  fired: TriggerFiredData[],
  seenRows?: ReadonlyMap<string, SessionTriggerInfo>,
): SessionTriggerInfo[] {
  const liveIds = new Set(listed.map((t) => t.id))
  const liveActivity = new Map<string, TriggerFiredData>()
  const ghosts: SessionTriggerInfo[] = []
  const seen = new Set<string>()
  for (let i = fired.length - 1; i >= 0; i--) {
    const t = fired[i]
    const key = t.subscription_id
    if (liveIds.has(key)) {
      if (seen.has(key)) continue
      seen.add(key)
      if (
        t.retired ||
        t.trigger_type !== undefined ||
        t.config !== undefined ||
        t.fires !== undefined ||
        t.outcome !== undefined ||
        t.retirement_reason !== undefined
      ) {
        liveActivity.set(key, t)
      }
    } else if (t.retired && !seen.has(key)) {
      // Once the durable list has dropped a subscription, only a retirement
      // can explain its absence. A later non-retired delivery record may have
      // arrived out of order, so it must not hide the newest retirement.
      seen.add(key)
      const remembered = seenRows?.get(key)
      ghosts.push(
        remembered ? withFiredActivity(remembered, t) : firedGhostRow(t),
      )
    }
  }
  if (liveActivity.size === 0 && ghosts.length === 0) return listed
  const rows = listed.map((row) => {
    const t = liveActivity.get(row.id)
    return t ? withFiredActivity(row, t) : row
  })
  return [...rows, ...ghosts]
}
