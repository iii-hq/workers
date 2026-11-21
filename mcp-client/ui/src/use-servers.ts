import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useMemo } from 'react'
import { createApi } from './api'
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
  loading: boolean
  error: string | null
  refresh: () => void
}

type Snapshot = Pick<ServersData, 'list' | 'config' | 'functions' | 'workerDown' | 'configDown' | 'configError'>

/** The servers, the entry and the registered functions, re-read on every server or function change. */
export function useServers(host: Host): ServersData {
  const api = useMemo(() => createApi(host.iii), [host])
  const { data, loading, error, refresh } = useWorkerLive<Snapshot>({
    iii: host.iii,
    sources: SOURCES,
    handlerId: EVENTS_FN,
    fetch: async () => {
      // Each read fails alone: a stopped worker still leaves the entry to edit.
      const [list, config, fns] = await Promise.allSettled([api.listServers(), api.readConfig(), api.listFunctions()])
      const functions: Record<string, EngineFunction[]> = {}
      if (fns.status === 'fulfilled') {
        for (const fn of fns.value) (functions[fn.function_id.split('::')[1]] ??= []).push(fn)
      }
      return {
        list: list.status === 'fulfilled' ? list.value : null,
        config: config.status === 'fulfilled' ? config.value.mcpServers : null,
        functions,
        workerDown: list.status === 'rejected',
        configDown: config.status === 'rejected',
        configError: config.status === 'rejected' ? errorMessage(config.reason) : null,
      }
    },
  })
  return {
    list: data?.list ?? null,
    config: data?.config ?? null,
    functions: data?.functions ?? {},
    workerDown: data?.workerDown ?? false,
    configDown: data?.configDown ?? false,
    configError: data?.configError ?? null,
    loading,
    error,
    refresh,
  }
}
