/**
 * The Chat's projects for the whole tab (chat-projects §1.4): one store fed by
 * `harness::projects::list {version: 2}` and kept current by
 * `harness::projects::on-changed`, read with `useSyncExternalStore`.
 *
 * The store wakes only when the engine has projects
 * (`ChatCapabilities.projects`) and a component asks for them; without them
 * it stays empty and nothing is called. An event carries the new revision:
 * the store re-lists (after a short settle that folds a burst into one read)
 * and keeps a list only when its revision is not older than the one held.
 */
import { useEffect, useMemo, useSyncExternalStore } from 'react'
import {
  type ChatProject,
  listProjects,
  PROJECTS_CHANGED_FUNCTION_ID,
  type ProjectList,
} from '@/lib/backend/projects'
import { useChatCapabilities } from '@/lib/chat-capabilities'
import { getIiiClient } from '@/lib/iii-client'

export interface ChatProjectsState {
  /** The first list has answered. */
  loaded: boolean
  revision: number
  projects: readonly ChatProject[]
  error: string | null
}

const EMPTY: ChatProjectsState = {
  loaded: false,
  revision: -1,
  projects: [],
  error: null,
}

const WATCH_FN = 'ade::chat-projects-watch'
const SETTLE_MS = 150

let state: ChatProjectsState = EMPTY
let active = false
let stopWatch: (() => void) | null = null
let settle: ReturnType<typeof setTimeout> | null = null
let seq = 0
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of listeners) listener()
}

/** Keep `list` unless the store already holds a newer revision. */
export function applyProjectList(list: ProjectList): void {
  if (state.loaded && list.revision < state.revision) return
  state = {
    loaded: true,
    revision: list.revision,
    projects: list.projects,
    error: null,
  }
  emit()
}

/** Read the list now (after a write, or on an event). */
export async function refreshChatProjects(): Promise<void> {
  if (!active) return
  const mine = ++seq
  try {
    const list = await listProjects()
    if (mine === seq || list.revision > state.revision) applyProjectList(list)
  } catch (err) {
    if (mine !== seq) return
    state = {
      ...state,
      loaded: true,
      error: err instanceof Error ? err.message : String(err),
    }
    emit()
  }
}

function scheduleRefresh(): void {
  if (settle) clearTimeout(settle)
  settle = setTimeout(() => {
    settle = null
    void refreshChatProjects()
  }, SETTLE_MS)
}

function start(): void {
  if (active) return
  active = true
  void refreshChatProjects()
  let cancelled = false
  let off: (() => void)[] = []
  stopWatch = () => {
    cancelled = true
    for (const dispose of off) dispose()
    off = []
  }
  void (async () => {
    try {
      const client = await getIiiClient()
      if (cancelled) return
      const localFnId = `${WATCH_FN}::${Math.random().toString(36).slice(2, 10)}`
      off.push(client.on(localFnId, () => scheduleRefresh()))
      off.push(
        client.observe(
          PROJECTS_CHANGED_FUNCTION_ID,
          `${localFnId}::${client.browserId}`,
        ),
      )
    } catch {
      // The list still answered (or will on the next start).
    }
  })()
}

function stop(): void {
  if (!active) return
  active = false
  stopWatch?.()
  stopWatch = null
  if (settle) clearTimeout(settle)
  settle = null
  seq++
  state = EMPTY
  emit()
}

export function getChatProjectsState(): ChatProjectsState {
  return state
}

export function subscribeChatProjects(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/**
 * The projects, when the engine has them. `enabled` is false on mock and
 * Storybook backends (stories seed the store with `seedChatProjectsForTests`).
 */
export function useChatProjects(enabled = true): ChatProjectsState & {
  available: boolean
} {
  const caps = useChatCapabilities(enabled)
  const available = caps.projects
  useEffect(() => {
    if (!enabled) return
    if (available) start()
    else if (caps.loaded) stop()
  }, [enabled, available, caps.loaded])
  const current = useSyncExternalStore(
    subscribeChatProjects,
    getChatProjectsState,
    getChatProjectsState,
  )
  return useMemo(() => ({ ...current, available }), [current, available])
}

/** Tests and stories: put a list in the store, or empty it with null. */
export function seedChatProjectsForTests(
  projects: readonly ChatProject[] | null,
  revision = 1,
): void {
  if (projects === null) {
    stop()
    state = EMPTY
    emit()
    return
  }
  state = { loaded: true, revision, projects, error: null }
  emit()
}
