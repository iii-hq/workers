import type { QueryClient } from '@tanstack/react-query'
import {
  fetchWorkspaceLayout,
  setWorkspaceLayout,
  type WorkspaceLayoutValue,
} from '@/lib/workspace-layout'
import { SerializedConfigWriter } from './serialized-config-writer'

export const WORKSPACE_LAYOUT_QUERY_KEY = ['workspaceLayout'] as const

const writers = new WeakMap<QueryClient, SerializedConfigWriter>()

/**
 * The one writer for the workspace layout document
 * (`console::workspace::get` / `set`).
 *
 * The layout used to share the `console` configuration entry — and its
 * writer — with the traces preferences. It now has its own store, so it gets
 * its own queue and cache entry: a tab switch must never rebase a saved-view
 * edit, and vice versa. Every read-modify-write of the layout still funnels
 * through this single writer, for the same reason as before: two writers
 * racing on one document lose whichever update reads first.
 */
export function workspaceLayoutWriter(qc: QueryClient): SerializedConfigWriter {
  const existing = writers.get(qc)
  if (existing) return existing
  const writer = new SerializedConfigWriter({
    readRemote: fetchWorkspaceLayout,
    writeRemote: setWorkspaceLayout,
    readCached: () =>
      qc.getQueryData<WorkspaceLayoutValue | null>(WORKSPACE_LAYOUT_QUERY_KEY),
    publish: (value) => {
      qc.setQueryData(WORKSPACE_LAYOUT_QUERY_KEY, value)
    },
    cancelReads: () => {
      void qc.cancelQueries({
        queryKey: WORKSPACE_LAYOUT_QUERY_KEY,
        exact: true,
      })
    },
  })
  writers.set(qc, writer)
  return writer
}
