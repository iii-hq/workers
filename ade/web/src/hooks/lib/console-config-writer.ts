import type { QueryClient } from '@tanstack/react-query'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'
import { SerializedConfigWriter } from './serialized-config-writer'

export const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig'] as const

const writers = new WeakMap<QueryClient, SerializedConfigWriter>()

/**
 * The one writer for the `console` configuration entry.
 *
 * Every hook that read-modify-writes the entry (workspace tabs, trace views,
 * follow turns, span filters) must go through the same queue: two writers
 * racing on one value lose whichever update reads first, and publish the
 * stale copy into the shared query cache on the way, so a pane that was just
 * split blinks out for a frame and then is gone from the server.
 */
export function consoleConfigWriter(qc: QueryClient): SerializedConfigWriter {
  const existing = writers.get(qc)
  if (existing) return existing
  const writer = new SerializedConfigWriter({
    readRemote: fetchConsoleConfigValue,
    writeRemote: setConsoleConfigValue,
    readCached: () =>
      qc.getQueryData<ConsoleConfigValue | null>(CONSOLE_CONFIG_QUERY_KEY),
    publish: (value) => {
      qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, value)
    },
    cancelReads: () => {
      void qc.cancelQueries({ queryKey: CONSOLE_CONFIG_QUERY_KEY, exact: true })
    },
  })
  writers.set(qc, writer)
  return writer
}
