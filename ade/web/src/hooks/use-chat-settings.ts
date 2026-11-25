/**
 * The harness's chat settings (`harness::chat-settings::*`, chat-projects
 * §1.2.5) for the whole tab: read once when the engine has them, kept current
 * by `harness::chat-settings::on-changed`, written as a server-side merge.
 * Phase 1 reads `descriptions.agent_writes` (Session descriptions).
 */
import { useEffect, useMemo, useSyncExternalStore } from 'react'
import {
  CHAT_SETTINGS_CHANGED_FUNCTION_ID,
  type ChatSettings,
  getChatSettings,
  parseChatSettings,
  setChatSettings,
} from '@/lib/backend/projects'
import { useChatCapabilities } from '@/lib/chat-capabilities'
import { getIiiClient } from '@/lib/iii-client'

interface State {
  settings: ChatSettings | null
}

const WATCH_FN = 'ade::chat-settings-watch'
let state: State = { settings: null }
let active = false
let off: (() => void)[] = []
const listeners = new Set<() => void>()

function set(settings: ChatSettings | null): void {
  state = { settings }
  for (const listener of listeners) listener()
}

async function read(): Promise<void> {
  try {
    set(await getChatSettings())
  } catch {
    // keep the last answer
  }
}

function start(): void {
  if (active) return
  active = true
  void read()
  void (async () => {
    try {
      const client = await getIiiClient()
      if (!active) return
      const localFnId = `${WATCH_FN}::${Math.random().toString(36).slice(2, 10)}`
      off.push(
        client.on(localFnId, (payload: unknown) => {
          const settings = (payload as { settings?: unknown } | null)?.settings
          if (settings) set(parseChatSettings(settings))
          else void read()
        }),
      )
      off.push(
        client.observe(
          CHAT_SETTINGS_CHANGED_FUNCTION_ID,
          `${localFnId}::${client.browserId}`,
        ),
      )
    } catch {
      // read once is still right until the next start
    }
  })()
}

function stop(): void {
  if (!active) return
  active = false
  for (const dispose of off) dispose()
  off = []
  set(null)
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

const getState = () => state

export interface ChatSettingsApi {
  /** The engine has `harness::chat-settings::*`. */
  available: boolean
  settings: ChatSettings | null
  /** Agents write descriptions (`descriptions.agent_writes`, default true). */
  agentWrites: boolean
  setAgentWrites: (on: boolean) => Promise<void>
}

export function useChatSettings(enabled = true): ChatSettingsApi {
  const caps = useChatCapabilities(enabled)
  const available = caps.chatSettings
  useEffect(() => {
    if (!enabled) return
    if (available) start()
    else if (caps.loaded) stop()
  }, [enabled, available, caps.loaded])
  const current = useSyncExternalStore(subscribe, getState, getState)
  return useMemo(
    () => ({
      available,
      settings: current.settings,
      agentWrites: current.settings?.descriptions.agent_writes ?? true,
      setAgentWrites: async (on: boolean) => {
        const before = current.settings
        if (before) {
          set({
            ...before,
            descriptions: { ...before.descriptions, agent_writes: on },
          })
        }
        try {
          set(await setChatSettings({ descriptions: { agent_writes: on } }))
        } catch (err) {
          set(before)
          throw err
        }
      },
    }),
    [available, current],
  )
}

/** Tests and stories. */
export function seedChatSettingsForTests(settings: ChatSettings | null): void {
  set(settings)
}
