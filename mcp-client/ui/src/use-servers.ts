import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useMemo, useRef } from 'react'
import { createApi } from './api'
import { holdSnapshot } from './presentation'
import type { EngineFunction, ServersList } from './types'

/** Per-tab handler id (the host namespaces it `::<browserId>`). */
const EVENTS_FN = 'iii::mcp-client-ui::events'
const SOURCES = ['mcp-client::on-servers-changed', 'engine::on-functions-changed'] as const

export interface ServersData {
  list: ServersList | null
  /** The entry's `mcpServers`, raw; null when it could not be read. */
  config: Record<string, unknown> | null
  /** Every `mcp::` function, keyed by its server segment. */
  functions: Record<string, EngineFunction[]>
  workerDown: boolean
  configDown: boolean
  /** Why the entry could not be read. */
  configError: string | null
  /** A refresh failed and the page shows the last good read of that part. */
  stale: boolean
  loading: boolean
  error: string | null
  refresh: () => void
}

export type Snapshot = Pick<
  ServersData,
  'list' | 'config' | 'functions' | 'workerDown' | 'configDown' | 'configError' | 'stale'
> & { functionsDown: boolean }

/** The servers, the entry and the registered functions, re-read on every server or function change. */
export function useServers(host: Host): ServersData {
  const api = useMemo(() => createApi(host.iii), [host])
  const held = useRef<Snapshot | null>(null)
  const { data, loading, error, refresh } = useWorkerLive<Snapshot>({
    iii: host.iii,
    sources: SOURCES,
    handlerId: EVENTS_FN,
    fetch: async () => {
      // Each read fails alone, and a failed one keeps its last good read. The entry's id comes from the worker,
      // so a worker never seen running leaves no entry to edit.
      const [list, config, fns] = await Promise.allSettled([api.listServers(), api.readConfig(), api.listFunctions()])
      const functions: Record<string, EngineFunction[]> = {}
      if (fns.status === 'fulfilled') {
        for (const fn of fns.value) (functions[fn.function_id.split('::')[1]] ??= []).push(fn)
      }
      held.current = holdSnapshot(held.current, {
        list: list.status === 'fulfilled' ? list.value : null,
        config: config.status === 'fulfilled' ? config.value.mcpServers : null,
        functions,
        workerDown: list.status === 'rejected',
        configDown: config.status === 'rejected',
        functionsDown: fns.status === 'rejected',
        configError: config.status === 'rejected' ? errorMessage(config.reason) : null,
        stale: false,
      })
      return held.current
    },
  })
  return {
    list: data?.list ?? null,
    config: data?.config ?? null,
    functions: data?.functions ?? {},
    workerDown: data?.workerDown ?? false,
    configDown: data?.configDown ?? false,
    configError: data?.configError ?? null,
    stale: data?.stale ?? false,
    loading,
    error,
    refresh,
  }
}
