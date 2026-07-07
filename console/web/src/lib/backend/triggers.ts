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
        once: typeof meta.once === 'boolean' ? meta.once : undefined,
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
