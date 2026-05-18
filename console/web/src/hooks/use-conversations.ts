import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  loadActiveId,
  loadConversations,
  loadLastModel,
  saveActiveId,
  saveConversations,
  saveLastModel,
} from '@/lib/storage'
import {
  type Conversation,
  DEFAULT_MODE,
  DEFAULT_MODEL,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
} from '@/types/chat'

function uid(): string {
  return Math.random().toString(36).slice(2) + Date.now().toString(36)
}

function deriveTitle(text: string): string {
  const clean = text.replace(/\s+/g, ' ').trim().toLowerCase()
  if (!clean) return 'new chat'
  return clean.length > 32 ? `${clean.slice(0, 32)}…` : clean
}

function emptyConversation(
  defaultModel: ModelId = DEFAULT_MODEL,
): Conversation {
  const now = Date.now()
  return {
    id: uid(),
    title: 'new chat',
    model: defaultModel,
    mode: DEFAULT_MODE,
    messages: [],
    createdAt: now,
    updatedAt: now,
  }
}

export interface ConversationsApi {
  conversations: Conversation[]
  activeId: string | null
  active: Conversation | null
  createNew: () => string
  select: (id: string) => void
  rename: (id: string, title: string) => void
  remove: (id: string) => void
  setModel: (id: string, model: ModelId) => void
  setMode: (id: string, mode: Mode) => void
  appendMessage: (id: string, message: Message) => void
  updateMessage: (id: string, messageId: string, patch: MessagePatch) => void
}

// Non-matching `conversation.model` values are migrated to the first
// catalog key; `catalogReady` gates the migration so it doesn't run
// against `STATIC_MODEL_OPTIONS` before the live catalog loads.
export function useConversations(
  catalogKeysForValidation?: readonly string[],
  catalogReady?: boolean,
): ConversationsApi {
  const catalogSig =
    catalogKeysForValidation && catalogKeysForValidation.length > 0
      ? [...catalogKeysForValidation].sort().join('\u0001')
      : ''

  const [conversations, setConversations] = useState<Conversation[]>(() => {
    const loaded = loadConversations()
    // Done in the initializer so StrictMode's double-invoke can't create two.
    return loaded.length > 0
      ? loaded
      : [emptyConversation(loadLastModel() ?? DEFAULT_MODEL)]
  })
  const [activeId, setActiveId] = useState<string | null>(() => {
    const stored = loadActiveId()
    return stored
  })

  // Debounced via rAF to avoid thrashing storage.
  const persistRef = useRef<number | null>(null)
  useEffect(() => {
    if (persistRef.current) cancelAnimationFrame(persistRef.current)
    persistRef.current = requestAnimationFrame(() =>
      saveConversations(conversations),
    )
    return () => {
      if (persistRef.current) cancelAnimationFrame(persistRef.current)
    }
  }, [conversations])

  // Wait for catalogReady before rewriting model ids — otherwise
  // catalog-only picks (e.g. claude-haiku-4-5) get clobbered to a
  // STATIC_MODEL_OPTIONS entry during the brief load window.
  useEffect(() => {
    if (!catalogSig) return
    if (catalogReady === false) return
    const keys = catalogSig.split('\u0001')
    const valid = new Set(keys)
    const fallback = keys[0]
    setConversations((prev) => {
      let changed = false
      const next = prev.map((c) => {
        if (valid.has(c.model)) return c
        changed = true
        return { ...c, model: fallback, updatedAt: Date.now() }
      })
      return changed ? next : prev
    })
    const lastModel = loadLastModel()
    if (lastModel && !valid.has(lastModel)) {
      saveLastModel(fallback)
    }
  }, [catalogSig, catalogReady])

  useEffect(() => {
    saveActiveId(activeId)
  }, [activeId])

  useEffect(() => {
    if (conversations.length === 0) return
    if (!activeId || !conversations.some((c) => c.id === activeId)) {
      setActiveId(conversations[0].id)
    }
  }, [conversations, activeId])

  const active = useMemo(
    () => conversations.find((c) => c.id === activeId) ?? null,
    [conversations, activeId],
  )

  const patchConversation = useCallback(
    (id: string, patch: (c: Conversation) => Conversation) => {
      setConversations((list) => list.map((c) => (c.id === id ? patch(c) : c)))
    },
    [],
  )

  const createNew = useCallback(() => {
    const next = emptyConversation(loadLastModel() ?? DEFAULT_MODEL)
    setConversations((list) => [next, ...list])
    setActiveId(next.id)
    return next.id
  }, [])

  const select = useCallback((id: string) => setActiveId(id), [])

  const rename = useCallback(
    (id: string, title: string) =>
      patchConversation(id, (c) => ({
        ...c,
        title: title.trim() || c.title,
        titleManual: true,
        updatedAt: Date.now(),
      })),
    [patchConversation],
  )

  const remove = useCallback((id: string) => {
    setConversations((list) => list.filter((c) => c.id !== id))
    setActiveId((current) => (current === id ? null : current))
  }, [])

  const setModel = useCallback(
    (id: string, model: ModelId) => {
      patchConversation(id, (c) => ({ ...c, model, updatedAt: Date.now() }))
      saveLastModel(model)
    },
    [patchConversation],
  )

  const setMode = useCallback(
    (id: string, mode: Mode) =>
      patchConversation(id, (c) => ({ ...c, mode, updatedAt: Date.now() })),
    [patchConversation],
  )

  const appendMessage = useCallback(
    (id: string, message: Message) =>
      patchConversation(id, (c) => {
        const messages = [...c.messages, message]
        const next: Conversation = {
          ...c,
          messages,
          updatedAt: Date.now(),
        }
        if (
          !c.titleManual &&
          message.role === 'user' &&
          c.messages.every((m) => m.role !== 'user')
        ) {
          next.title = deriveTitle(message.content)
        }
        return next
      }),
    [patchConversation],
  )

  const updateMessage = useCallback(
    (id: string, messageId: string, patch: MessagePatch) =>
      patchConversation(id, (c) => ({
        ...c,
        messages: c.messages.map((m) =>
          m.id === messageId ? ({ ...m, ...patch } as Message) : m,
        ),
        updatedAt: Date.now(),
      })),
    [patchConversation],
  )

  return {
    conversations,
    activeId,
    active,
    createNew,
    select,
    rename,
    remove,
    setModel,
    setMode,
    appendMessage,
    updateMessage,
  }
}

export { uid }
