/**
 * Session-owned trigger subscriptions. The agent registers them through the
 * harness's `engine::register_trigger` intercept, which binds each one to
 * `harness::notify_agent` (notification into the owning session) or
 * `harness::react` (spawn a sub-agent) and stamps the owning session onto the
 * engine trigger's metadata — `session_id` for notify bindings,
 * `__owner_session_id` for react bindings (see harness
 * `subscriptions/reconcile.rs::owner_key`). The console lists both targets and
 * filters by that owner to show a conversation's subscriptions.
 */

import type { IiiClient } from '@/lib/iii-client'
import type { TriggerFiredData } from '@/types/chat'

export interface SessionTriggerInfo {
  /** Engine trigger id — the unregister handle. */
  id: string
  /** e.g. `cron`, `state`, `harness::turn-completed`. */
  triggerType: string
  /** `harness::notify_agent` or `harness::react`. */
  functionId: string
  config: unknown
  configSummary: string
  label?: string
  once?: boolean
  metadata?: Record<string, unknown>
  /**
   * This trigger already fired and was unregistered (per a durable
   * `trigger_fired` transcript entry — see `mergeFiredTriggers`). Either a
   * still-polled row annotated ahead of the next poll, or a synthesized
   * "ghost" row reconstructed after the poll dropped it. No engine handle to
   * unregister — the ✕ dismisses locally instead.
   */
  fired?: boolean
  firedAt?: number
}

const NOTIFY_TARGET = 'harness::notify_agent'
const REACT_TARGET = 'harness::react'

interface RegisteredTriggerSummary {
  id: string
  trigger_type: string
  function_id: string
  worker_name: string
  config: unknown
  config_summary: string
}

interface RegisteredTriggerDetail extends RegisteredTriggerSummary {
  metadata?: Record<string, unknown>
}

/**
 * List the triggers owned by `sessionId`: both harness targets, detail-read
 * for the owner stamp (the list summary carries no metadata).
 */
// ponytail: 2 lists + one info per binding each poll; add an owner filter to
// engine::registered-triggers::list if binding counts ever matter.
export async function listSessionTriggers(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
): Promise<SessionTriggerInfo[]> {
  const out: SessionTriggerInfo[] = []
  for (const functionId of [NOTIFY_TARGET, REACT_TARGET]) {
    const list = await client
      .trigger<{ registered_triggers: RegisteredTriggerSummary[] }>(
        'engine::registered-triggers::list',
        { function_id: functionId },
      )
      .catch(() => null)
    for (const summary of list?.registered_triggers ?? []) {
      const detail = await client
        .trigger<RegisteredTriggerDetail>('engine::registered-triggers::info', {
          id: summary.id,
        })
        .catch(() => null)
      if (!detail) continue
      const meta = detail.metadata ?? {}
      const owner =
        functionId === NOTIFY_TARGET ? meta.session_id : meta.__owner_session_id
      if (owner !== sessionId) continue
      out.push({
        id: detail.id,
        triggerType: detail.trigger_type,
        functionId,
        config: detail.config,
        configSummary: summary.config_summary,
        label: typeof meta.label === 'string' ? meta.label : undefined,
        // Notify bindings stamp `once`; react bindings stamp `__once`.
        once:
          typeof meta.once === 'boolean'
            ? meta.once
            : typeof meta.__once === 'boolean'
              ? meta.__once
              : undefined,
        metadata: meta,
      })
    }
  }
  return out
}

/**
 * Unregister an engine trigger by id. Goes straight to the engine (the
 * console is a trusted consumer, not an in-run agent). A notify binding's
 * in-memory harness registry entry may linger, but with the engine trigger
 * gone it can never fire and is swept on session delete / harness restart.
 */
export async function unregisterTrigger(
  client: Pick<IiiClient, 'trigger'>,
  triggerId: string,
): Promise<void> {
  await client.trigger('engine::unregister_trigger', { id: triggerId })
}

/** Reconstruct a fired-and-unregistered trigger's panel row from its record. */
function firedGhostRow(t: TriggerFiredData): SessionTriggerInfo {
  const isState = typeof t.key === 'string'
  return {
    id: t.trigger_id ?? `fired:${t.subscription_id}`,
    // The record carries no trigger_type; infer state from the watch and fall
    // back to a generic name so a label-less ghost never renders an empty row.
    triggerType: isState ? 'state' : t.join ? 'join' : 'trigger',
    functionId: t.target === 'spawn' ? REACT_TARGET : NOTIFY_TARGET,
    config: isState ? { scope: t.scope, key: t.key } : undefined,
    configSummary: '',
    label: t.label ?? (t.join ? `join ${t.join.id}` : undefined),
    once: t.once,
    metadata: t.model ? { model: t.model } : undefined,
    fired: true,
    firedAt: t.fired_at,
  }
}

/**
 * Merge the live poll with fired-trigger history. A *retired* fire means the
 * binding was unregistered engine-side: if the (≤5s stale) poll still lists
 * it, annotate that row as fired in place; once the poll drops it, append a
 * greyed "ghost" row. Non-retired fires leave their live row untouched;
 * repeat fires collapse to one record (newest wins).
 *
 * Ghost fidelity is two-tier: prefer the FULL last-seen polled row (from
 * `seenRows`) so join grouping and the workflow/DAG structure survive the
 * binding's retirement — the fired record alone carries no `metadata.join` /
 * spawn pin / task, and a pipeline of fired thin ghosts would collapse to a
 * flat list. The thin record-only ghost remains the post-reload fallback.
 *
 * ponytail: a completed join collapses to a single fired row (`join <id>`)
 * rather than resurrecting each predecessor row — enough to show it fired +
 * retired. Per-predecessor ghosts if that granularity is ever needed.
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
    const key = t.trigger_id ?? t.subscription_id
    if (seen.has(key)) continue
    seen.add(key)
    if (t.trigger_id && liveIds.has(t.trigger_id)) {
      retiredLive.set(t.trigger_id, t) // stale poll row — mark, don't ghost
    } else {
      const remembered = t.trigger_id ? seenRows?.get(t.trigger_id) : undefined
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
