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
  once?: boolean
  fires?: number
  maxFires?: number
  expiresAt?: number
  createdAt?: number
  /**
   * This subscription already fired and was retired (per a durable
   * `trigger_fired` transcript entry — see `mergeFiredTriggers`). Either a
   * still-polled row annotated ahead of the next poll, or a synthesized
   * "ghost" row reconstructed after the poll dropped it. Nothing left to
   * unregister — the ✕ dismisses locally instead.
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

/** List the subscriptions `sessionId` owns — one call, straight from the store. */
export async function listSessionTriggers(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
): Promise<SessionTriggerInfo[]> {
  const response = await client
    .trigger<{ subscriptions: TriggerRow[] }>('harness::triggers::list', {
      session_id: sessionId,
    })
    .catch(() => null)
  return (response?.subscriptions ?? []).map((row) => ({
    id: row.subscription_id,
    triggerId: row.trigger_id ?? undefined,
    triggerType: row.trigger_type ?? 'trigger',
    delivery: deliveryOf(row.target),
    config: row.config,
    conditions: row.conditions,
    label: row.label ?? undefined,
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

/** Reconstruct a fired-and-retired subscription's panel row from its record. */
function firedGhostRow(t: TriggerFiredData): SessionTriggerInfo {
  const isState = typeof t.key === 'string'
  return {
    id: t.subscription_id,
    triggerId: t.trigger_id ?? undefined,
    // The record carries no trigger_type; infer state from the watch and fall
    // back to a generic name so a label-less ghost never renders an empty row.
    triggerType: isState ? 'state' : 'trigger',
    delivery: deliveryOf(t.target),
    config: isState ? { scope: t.scope, key: t.key } : undefined,
    label: t.label,
    once: t.once,
    fired: true,
    firedAt: t.fired_at,
  }
}

/**
 * Merge the live poll with fired-subscription history, correlated on the
 * subscription id (present on both sides). A *retired* fire means the binding
 * is gone: if the (≤5s stale) poll still lists it, annotate that row as fired
 * in place; once the poll drops it, append a greyed "ghost" row. Non-retired
 * fires leave their live row untouched; repeat fires collapse to one record
 * (newest wins).
 *
 * Ghost fidelity is two-tier: prefer the FULL last-seen polled row (from
 * `seenRows`) — the fired record alone carries no config or conditions. The
 * thin record-only ghost remains the post-reload fallback.
 */
export function mergeFiredTriggers(
  polled: SessionTriggerInfo[],
  fired: TriggerFiredData[],
  seenRows?: ReadonlyMap<string, SessionTriggerInfo>,
): SessionTriggerInfo[] {
  const liveIds = new Set(polled.map((t) => t.id))
  const retiredLive = new Map<string, TriggerFiredData>()
  const ghosts: SessionTriggerInfo[] = []
  const seen = new Set<string>()
  for (let i = fired.length - 1; i >= 0; i--) {
    const t = fired[i]
    if (!t.retired) continue
    const key = t.subscription_id
    if (seen.has(key)) continue
    seen.add(key)
    if (liveIds.has(key)) {
      retiredLive.set(key, t) // stale poll row — mark, don't ghost
    } else {
      const remembered = seenRows?.get(key)
      ghosts.push(
        remembered
          ? { ...remembered, fired: true, firedAt: t.fired_at }
          : firedGhostRow(t),
      )
    }
  }
  if (retiredLive.size === 0 && ghosts.length === 0) return polled
  const rows = polled.map((row) => {
    const t = retiredLive.get(row.id)
    return t ? { ...row, fired: true, firedAt: t.fired_at } : row
  })
  return [...rows, ...ghosts]
}
