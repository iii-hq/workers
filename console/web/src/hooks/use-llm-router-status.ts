import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `llm-router` worker. The router owns every
 * `router::*` RPC the model picker reads (`provider::list`, `models::list`)
 * and the change triggers it subscribes to, so provider/model UI gates on
 * THIS worker's presence — gating on the harness starved the picker whenever
 * the harness was slow or absent while the router was healthy. Thin wrapper
 * over the generic worker-presence probe.
 */

/** Engine worker name for the llm-router worker. */
export const LLM_ROUTER_WORKER_NAME = 'llm-router'
/** Base id for the browser-local handler bound to the `worker` trigger. */
export const LLM_ROUTER_WATCH_FN = 'console::llm-router-watch'

export type LlmRouterStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the router as present so the picker shows
 *   in isolation).
 */
export function useLlmRouterStatus(enabled: boolean): LlmRouterStatus {
  return useWorkerPresence({
    workerName: LLM_ROUTER_WORKER_NAME,
    watchFnId: LLM_ROUTER_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the router's `router::*` functions are registered and safe to
 * trigger. False during the initial presence probe and while the router is
 * absent.
 */
export function isLlmRouterAvailable(status: LlmRouterStatus): boolean {
  return isWorkerPresent(status)
}
