/**
 * Provider installs in flight, shared by every Add a provider surface (the
 * picker's page, the Models dialog's page and its search results).
 *
 * A `compose::add` keeps running after the panel that started it unmounts
 * (the search cleared, the page closed), so its status cannot live in that
 * panel: a remounted panel would show the worker as not installed and a
 * second click would start a second install. Here one install per worker
 * runs at a time, and every panel reads the same status.
 */

import { useSyncExternalStore } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { fetchEngineWorkersList } from '@/pages/Workers/api/workers'

export type ProviderInstallStatus =
  | { kind: 'adding' }
  | { kind: 'done' }
  | { kind: 'failed'; error: string }

let installs: ReadonlyMap<string, ProviderInstallStatus> = new Map()
const listeners = new Set<() => void>()

function set(name: string, status: ProviderInstallStatus): void {
  installs = new Map(installs).set(name, status)
  for (const listener of [...listeners]) listener()
}

/** Install a provider worker; a second call while one is adding is a no-op. */
export function startProviderInstall(name: string): void {
  if (installs.get(name)?.kind === 'adding') return
  set(name, { kind: 'adding' })
  void (async () => {
    try {
      const client = await getIiiClient()
      await client.call(
        'compose::add',
        { workers: [name] },
        { timeoutMs: 600_000 },
      )
      set(name, { kind: 'done' })
    } catch (err) {
      // A late timeout can reject after the add actually landed.
      try {
        const list = await fetchEngineWorkersList()
        if (list.workers.some((worker) => worker.name === name)) {
          set(name, { kind: 'done' })
          return
        }
      } catch {
        // fall through to the original error
      }
      set(name, { kind: 'failed', error: normalizeErrorMessage(err) })
    }
  })()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function useProviderInstalls(): ReadonlyMap<
  string,
  ProviderInstallStatus
> {
  return useSyncExternalStore(
    subscribe,
    () => installs,
    () => installs,
  )
}

/** Tests only: forget every install. */
export function resetProviderInstalls(): void {
  installs = new Map()
  for (const listener of [...listeners]) listener()
}
