/**
 * Session-owned observers, read from the harness's durable store via
 * `harness::observers::list`. `engine::observers::list` cannot serve this
 * view: every agent observer targets the internal delivery hop
 * (`harness::observer::deliver`) with metadata that is only a `__binding`
 * pointer, so owner and shape live in the harness records alone. Rows are
 * source-generic — the observed function with its raw args and filters, plus
 * a delivery that is either "notify the owner" or "call a plain function" —
 * so functions that do not exist yet render unchanged.
 */

import type { IiiClient } from '@/lib/iii-client'
import type { ObserverFiredData } from '@/types/chat'

export type ObserverDelivery =
  | { kind: 'notify' }
  | { kind: 'call'; functionId: string }

export interface SessionObserverInfo {
  /** Harness observer id — the dispose handle, and the engine observer's id. */
  id: string
  /** The observed function: `cron::on-tick`, `state::on-changed`, or any other. */
  functionId: string
  /** Where the observed function is hosted, when the observer names it. */
  namespace?: string
  /** Read by the observed function's worker (a schedule, a route). */
  args?: Record<string, unknown>
  /** Which calls reach the observer, by explicit operators on their fields. */
  filters?: Record<string, unknown>
  delivery: ObserverDelivery
  conditions?: unknown[]
  label?: string
  /** Human-readable event text declared as the observe call's `action`. */
  action?: string
  once?: boolean
  fires?: number
  maxFires?: number
  expiresAt?: number
  createdAt?: number
  /** Latest structured activity, when the transcript carries it. */
  outcome?: ObserverFiredData['outcome']
  /** Why this row became inactive. Absent on active and historical records. */
  retirementReason?: ObserverFiredData['retirement_reason']
  /**
   * This observer is inactive per a retired activity record. Its structured
   * reason may be consumption, expiry, removal, invalidation, or a legacy
   * unknown — never infer one from this boolean alone. Nothing remains to
   * dispose; ✕ dismisses.
   */
  fired?: boolean
  firedAt?: number
}

const NOTIFY_TARGET = 'harness::send'

/** One row of `harness::observers::list` (`harness/src/functions/observers_list.rs`). */
interface ObserverRow {
  observer_id: string
  function_id?: string
  namespace?: string
  args?: unknown
  filters?: unknown
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

const recordOf = (value: unknown): Record<string, unknown> | undefined =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined

/**
 * Map a raw delivery target to its kind. Live rows carry the target function
 * id (absent = notify); activity records written before the delivery hop
 * carry the legacy words `'notify'` / `'spawn'`.
 */
export function deliveryOf(
  target: string | undefined | null,
): ObserverDelivery {
  if (!target || target === 'notify' || target === NOTIFY_TARGET)
    return { kind: 'notify' }
  if (target === 'spawn') return { kind: 'call', functionId: 'harness::spawn' }
  return { kind: 'call', functionId: target }
}

/**
 * List the observers `sessionId` owns — one call, straight from the store.
 * Transport failures reject; an empty array means the store was read
 * successfully and the session owns no observers.
 */
export async function listSessionObservers(
  client: Pick<IiiClient, 'call'>,
  sessionId: string,
): Promise<SessionObserverInfo[]> {
  const response = await client.call<{ observers: ObserverRow[] }>(
    'harness::observers::list',
    { session_id: sessionId },
  )
  return (response?.observers ?? []).map((row) => {
    const args = recordOf(row.args)
    const filters = recordOf(row.filters)
    return {
      id: row.observer_id,
      functionId: row.function_id ?? 'observer',
      ...(row.namespace ? { namespace: row.namespace } : {}),
      ...(args ? { args } : {}),
      ...(filters ? { filters } : {}),
      delivery: deliveryOf(row.target),
      conditions: row.conditions,
      label: row.label ?? undefined,
      action: row.action ?? undefined,
      once: row.once,
      fires: row.fires,
      maxFires: row.max_fires ?? undefined,
      expiresAt: row.expires_at ?? undefined,
      createdAt: row.created_at,
    }
  })
}

/**
 * Remove an observer through the harness — the engine observer AND its
 * durable record. Disposing only the engine observer would orphan the record
 * and leave the owner session believing in an armed wake that can never fire.
 */
export async function disposeSessionObserver(
  client: Pick<IiiClient, 'call'>,
  observerId: string,
  sessionId: string,
): Promise<void> {
  await client.call('harness::observers::dispose', {
    observer_id: observerId,
    session_id: sessionId,
  })
}

/** Reconstruct an inactive observer's panel row from its activity record. */
function firedGhostRow(t: ObserverFiredData): SessionObserverInfo {
  const isState = typeof t.key === 'string'
  const stateFilters = isState
    ? {
        ...(t.scope !== undefined ? { 'input.scope': { $eq: t.scope } } : {}),
        'input.key': { $eq: t.key },
      }
    : undefined
  return {
    id: t.observer_id,
    // Current records carry the observed function. Older state records can
    // still be reconstructed from their scope/key watch.
    functionId: t.function_id ?? (isState ? 'state::on-changed' : 'observer'),
    ...(t.args !== undefined ? { args: t.args } : {}),
    ...(t.filters !== undefined
      ? { filters: t.filters }
      : stateFilters
        ? { filters: stateFilters }
        : {}),
    delivery: deliveryOf(t.target),
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
  row: SessionObserverInfo,
  t: ObserverFiredData,
): SessionObserverInfo {
  return {
    ...row,
    functionId: t.function_id ?? row.functionId,
    ...(t.args !== undefined ? { args: t.args } : {}),
    ...(t.filters !== undefined ? { filters: t.filters } : {}),
    action: t.action ?? row.action,
    fires: t.fires ?? row.fires,
    outcome: t.outcome,
    retirementReason: t.retirement_reason,
    ...(t.retired ? { fired: true, firedAt: t.fired_at } : {}),
  }
}

/**
 * Merge the live list with fired-observer history, correlated on the
 * observer id (present on both sides). A *retired* fire means the observer
 * is gone: if a not-yet-refetched list still carries it, annotate that row as
 * inactive in place; once the refetch drops it, append a greyed "ghost" row.
 * The newest non-retired activity enriches its live row without making it
 * inactive. Live rows use the newest activity; absent rows use the newest
 * retirement and ignore non-retired activity that arrived after it.
 *
 * Ghost fidelity is two-tier: prefer the full last-seen listed row (from
 * `seenRows`) for conditions and registration details, then overlay the
 * record-owned source/lifecycle fields. The record-only ghost remains the
 * post-reload fallback; historical records may still be thin.
 */
export function mergeFiredObservers(
  listed: SessionObserverInfo[],
  fired: ObserverFiredData[],
  seenRows?: ReadonlyMap<string, SessionObserverInfo>,
): SessionObserverInfo[] {
  const liveIds = new Set(listed.map((t) => t.id))
  const liveActivity = new Map<string, ObserverFiredData>()
  const ghosts: SessionObserverInfo[] = []
  const seen = new Set<string>()
  for (let i = fired.length - 1; i >= 0; i--) {
    const t = fired[i]
    const key = t.observer_id
    if (liveIds.has(key)) {
      if (seen.has(key)) continue
      seen.add(key)
      if (
        t.retired ||
        t.function_id !== undefined ||
        t.args !== undefined ||
        t.filters !== undefined ||
        t.fires !== undefined ||
        t.outcome !== undefined ||
        t.retirement_reason !== undefined
      ) {
        liveActivity.set(key, t)
      }
    } else if (t.retired && !seen.has(key)) {
      // Once the durable list has dropped an observer, only a retirement
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
