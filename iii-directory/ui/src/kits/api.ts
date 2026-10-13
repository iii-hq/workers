/**
 * Calls to `directory::download-kit` / `directory::kits::*` and the
 * `directory::kits::on-change` subscription the Kits page and the chat
 * cards share.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import type { ApplyProgress, KitInfo, KitPlanResponse, KitsChange, KitsListing, Plan, PlanContents } from './types'

export const KITS_ON_CHANGE = 'directory::kits::on-change'

/** Applies can wait on Compose downloading worker binaries. */
const APPLY_TIMEOUT_MS = 20 * 60 * 1000
const PLAN_TIMEOUT_MS = 2 * 60 * 1000

export interface PlanRecord {
  plan: Plan
  progress?: ApplyProgress | null
  contents?: PlanContents
}

export function kitsApi(host: Host) {
  const call = <T>(fn: string, payload: Record<string, unknown> = {}, timeoutMs?: number) =>
    host.iii.trigger<T>(fn, payload, timeoutMs ? { timeoutMs } : undefined)
  return {
    list: () => call<KitsListing>('directory::kits::list'),
    get: (kit: string) => call<KitInfo>('directory::kits::get', { kit, registry: true }, PLAN_TIMEOUT_MS),
    plan: (planId: string) =>
      call<PlanRecord>('directory::kits::plan', { plan_id: planId, contents: true }, PLAN_TIMEOUT_MS),
    download: (kit: string) => call<KitPlanResponse>('directory::download-kit', { kit }, PLAN_TIMEOUT_MS),
    planUpdate: (kit: string, version?: string) =>
      call<KitPlanResponse>('directory::kits::plan-update', version ? { kit, version } : { kit }, PLAN_TIMEOUT_MS),
    remove: (kit: string) => call<KitPlanResponse>('directory::kits::remove', { kit }, PLAN_TIMEOUT_MS),
    apply: (planId: string, decisions: Record<string, unknown>, removeWorkers: string[] = []) =>
      call<KitPlanResponse>(
        'directory::kits::apply',
        { plan_id: planId, decisions, remove_workers: removeWorkers },
        APPLY_TIMEOUT_MS,
      ),
    discard: (planId: string) => call<{ discarded: boolean }>('directory::kits::discard', { plan_id: planId }),
    ignore: (kit: string, version: string, ignore = true) =>
      call<{ ignored_versions: string[] }>('directory::kits::ignore', { kit, version, ignore }),
    checkUpdates: () =>
      call<{ available: number; error?: string | null }>('directory::kits::check-updates', {}, PLAN_TIMEOUT_MS),
    diff: (kit: string, path: string) =>
      call<{ base: string | null; local: string | null; state: string }>(
        'directory::kits::diff',
        { kit, path },
        PLAN_TIMEOUT_MS,
      ),
  }
}

export type KitsApi = ReturnType<typeof kitsApi>

/**
 * Tab-scoped binding on `directory::kits::on-change`. `scope` keeps two
 * subscribers in one tab (the page badge and the Kits view) on distinct
 * function ids.
 */
export function useKitsChange(host: Host, scope: string, onEvent: (change: KitsChange) => void) {
  const ref = useRef(onEvent)
  ref.current = onEvent
  useEffect(() => {
    // `on` registers the handler tab-scoped as `<id>::<browserId>`; the
    // trigger must name that full id.
    const handlerId = `iii::iii-directory-ui::kits-on-change::${scope}`
    const off = host.iii.on<KitsChange>(handlerId, (payload) => {
      ref.current((payload ?? { op: 'updates' }) as KitsChange)
    })
    const offTrigger = host.iii.registerTrigger({
      type: KITS_ON_CHANGE,
      function_id: `${handlerId}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      off()
    }
  }, [host, scope])
}

/** Error text from a rejected trigger, without the engine's wrapper prefix. */
export function errorText(error: unknown): string {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : JSON.stringify(error)
  return raw
    .replace(/^remote error \(invocation_failed\):\s*/i, '')
    .replace(/^handler error:\s*/i, '')
    .trim()
}
