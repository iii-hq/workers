/**
 * Which chat-projects features this engine has (chat-projects spec §0.2).
 *
 * Every feature turns on only when its function is in the engine's catalogue
 * (`engine::functions::list`). An engine without it keeps today's screens:
 *
 * | Capability       | Function                    | Without it                                   |
 * |------------------|-----------------------------|----------------------------------------------|
 * | `patchMeta`      | `session::patch-meta`       | read-modify-write through `session::set-meta` |
 * | `folders`        | `harness::folders::list`    | the old `harness::projects::*` folder catalogue |
 * | `projects`       | `harness::projects::create` | no projects UI; the list groups by folder     |
 * | `setDescription` | `harness::set-description`  | descriptions by hand; no agent switch         |
 * | `contextItems`   | `harness::context-items`    | the pane parses the loaded transcript         |
 * | `chatSettings`   | `harness::chat-settings::get` | no Session descriptions setting             |
 *
 * One store for the whole tab: the catalogue is read once, then again (after a
 * short settle) on every `engine::on-functions-changed` and
 * `engine::on-workers-changed`, the console's presence watch. A failed read
 * keeps the last answer. Mock and Storybook backends never read; stories set
 * the answer they want with `setChatCapabilitiesForTests`.
 */
import { useEffect, useSyncExternalStore } from 'react'
import { getIiiClient } from '@/lib/iii-client'

export interface ChatCapabilities {
  /** The catalogue has been read at least once. */
  loaded: boolean
  patchMeta: boolean
  folders: boolean
  projects: boolean
  setDescription: boolean
  contextItems: boolean
  chatSettings: boolean
}

export const CAPABILITY_FUNCTIONS = {
  patchMeta: 'session::patch-meta',
  folders: 'harness::folders::list',
  projects: 'harness::projects::create',
  setDescription: 'harness::set-description',
  contextItems: 'harness::context-items',
  chatSettings: 'harness::chat-settings::get',
} as const satisfies Record<Exclude<keyof ChatCapabilities, 'loaded'>, string>

export const NO_CHAT_CAPABILITIES: ChatCapabilities = {
  loaded: false,
  patchMeta: false,
  folders: false,
  projects: false,
  setDescription: false,
  contextItems: false,
  chatSettings: false,
}

const WATCH_FN = 'ade::chat-capabilities-watch'
const SETTLE_MS = 300

let current: ChatCapabilities = NO_CHAT_CAPABILITIES
let started = false
let frozen = false
let settleTimer: ReturnType<typeof setTimeout> | null = null
let readSeq = 0
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of listeners) listener()
}

function same(a: ChatCapabilities, b: ChatCapabilities): boolean {
  return (Object.keys(a) as (keyof ChatCapabilities)[]).every(
    (key) => a[key] === b[key],
  )
}

function set(next: ChatCapabilities): void {
  if (same(current, next)) return
  current = next
  emit()
}

/** The capabilities in an `engine::functions::list` answer. */
export function capabilitiesFromCatalog(response: unknown): ChatCapabilities {
  const rows = (response as { functions?: unknown } | null)?.functions
  const ids = new Set<string>()
  if (Array.isArray(rows)) {
    for (const row of rows) {
      const id = (row as { function_id?: unknown } | null)?.function_id
      if (typeof id === 'string') ids.add(id)
    }
  }
  return {
    loaded: true,
    patchMeta: ids.has(CAPABILITY_FUNCTIONS.patchMeta),
    folders: ids.has(CAPABILITY_FUNCTIONS.folders),
    projects: ids.has(CAPABILITY_FUNCTIONS.projects),
    setDescription: ids.has(CAPABILITY_FUNCTIONS.setDescription),
    contextItems: ids.has(CAPABILITY_FUNCTIONS.contextItems),
    chatSettings: ids.has(CAPABILITY_FUNCTIONS.chatSettings),
  }
}

/** Read the catalogue now; a newer read wins and a failed one keeps the last answer. */
export async function refreshChatCapabilities(): Promise<ChatCapabilities> {
  if (frozen) return current
  const seq = ++readSeq
  try {
    const client = await getIiiClient()
    const response = await client.call('engine::functions::list', {})
    if (seq === readSeq && !frozen) set(capabilitiesFromCatalog(response))
  } catch {
    // keep the last known answer
  }
  return current
}

function scheduleRefresh(): void {
  if (settleTimer) clearTimeout(settleTimer)
  settleTimer = setTimeout(() => {
    settleTimer = null
    void refreshChatCapabilities()
  }, SETTLE_MS)
}

/**
 * Start watching the engine (real backend only). Idempotent: the first call
 * reads the catalogue and arms the two observers for the life of the tab.
 */
export function startChatCapabilities(): void {
  if (started || frozen) return
  started = true
  void refreshChatCapabilities()
  void (async () => {
    try {
      const client = await getIiiClient()
      const localFnId = `${WATCH_FN}::${Math.random().toString(36).slice(2, 10)}`
      client.on(localFnId, () => scheduleRefresh())
      const target = `${localFnId}::${client.browserId}`
      client.observe('engine::on-functions-changed', target)
      client.observe('engine::on-workers-changed', target)
    } catch {
      // No engine yet: the next start (a reconnect remounts) tries again.
      started = false
    }
  })()
}

export function getChatCapabilities(): ChatCapabilities {
  return current
}

export function subscribeChatCapabilities(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/**
 * The live capabilities. `enabled` is false on mock and Storybook backends:
 * nothing is read and the answer stays whatever was set (none by default).
 */
export function useChatCapabilities(enabled = true): ChatCapabilities {
  useEffect(() => {
    if (enabled) startChatCapabilities()
  }, [enabled])
  return useSyncExternalStore(
    subscribeChatCapabilities,
    getChatCapabilities,
    getChatCapabilities,
  )
}

/** Tests and stories: pin an answer (no engine reads until reset). */
export function setChatCapabilitiesForTests(
  next: Partial<ChatCapabilities> | null,
): void {
  if (next === null) {
    frozen = false
    started = false
    readSeq++
    current = NO_CHAT_CAPABILITIES
    emit()
    return
  }
  frozen = true
  set({ ...NO_CHAT_CAPABILITIES, loaded: true, ...next })
}
