/**
 * A `TraceSource` over an iii engine: `engine::traces::*`,
 * `engine::logs::list`, `engine::functions::list` (for `trace_hidden`
 * functions) and the `observability::on-span` activity feed.
 */

import type { TraceSource } from '../host'
import { startTraceActivityFeed } from './activity'
import { fetchTraceHiddenFunctionIds } from './hidden-functions'
import { fetchOtelLogs } from './otel-logs'
import { fetchTraceSpans, fetchTraces, fetchTracesGroupBy } from './traces'

/** The part of a browser iii client the engine source uses. */
export interface EngineClient {
  /** This client's id, the suffix `on()` registers its functions under. */
  browserId: string
  call<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
  ): Promise<T>
  /** Register a browser-side function; returns its unregister. */
  on(
    functionId: string,
    handler: (payload: unknown) => void | Promise<void>,
  ): () => void
  /** Observe `source` with the function registered as `target`. */
  observe(source: string, target: string): () => void
  /** Called with `connected` after every (re)connect; returns its remover. */
  addConnectionStateListener(listener: (state: string) => void): () => void
}

/** Resolves the client; called on every read, so a host can swap it. */
export type EngineConnect = () => EngineClient | Promise<EngineClient>

export function createEngineTraceSource(client: EngineConnect): TraceSource {
  return {
    listTraces: (options) => fetchTraces(client, options),
    traceSpans: (options) => fetchTraceSpans(client, options),
    groupTraces: (params) => fetchTracesGroupBy(client, params),
    logs: (options) => fetchOtelLogs(client, options),
    hiddenFunctionIds: () => fetchTraceHiddenFunctionIds(client),
    watch(onTraceIds, onResync) {
      let stop: (() => void) | undefined
      let active = true
      // `then(client)`: a connect that throws instead of rejecting is still
      // one rejection here, never a throw out of `watch`.
      void Promise.resolve()
        .then(client)
        .then(
          (resolved) => {
            if (!active) return
            const offFeed = startTraceActivityFeed(resolved, onTraceIds)
            // Set before the connection listener, so a throw there cannot
            // leave the feed running past the unsubscribe.
            stop = offFeed
            if (onResync) {
              const offConnection = resolved.addConnectionStateListener(
                (state) => {
                  if (state === 'connected') onResync()
                },
              )
              stop = () => {
                offFeed()
                offConnection()
              }
            }
          },
          // No client, no live updates: the reads report the failure. (Only
          // the client's rejection: a throw while subscribing stays loud.)
          () => {},
        )
      return () => {
        active = false
        stop?.()
      }
    },
  }
}
