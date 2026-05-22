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
    autoAccept: false,
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
  setAutoAccept: (id: string, autoAccept: boolean) => void
  appendMessage: (id: string, message: Message) => void
  updateMessage: (id: string, messageId: string, patch: MessagePatch) => void
  compactConversation: (id: string, marker: Message) => void
}

/** When set, non-matching `conversation.model` values are rewritten to the first key (catalog load / migration). `catalogReady` gates the migration so it doesn't run against `STATIC_MODEL_OPTIONS` before the live catalog has loaded. */
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
    /* Always boot with at least one empty conversation so the chat surface
       has something to render. Done in the initializer so StrictMode's
       double-invoke can't create two. */
    return loaded.length > 0
      ? loaded
      : [emptyConversation(loadLastModel() ?? DEFAULT_MODEL)]
  })
  const [activeId, setActiveId] = useState<string | null>(() => {
    const stored = loadActiveId()
    return stored
  })

  /* Persist on every mutation. Debounced via microtask to avoid thrashing. */
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

  /* Migrate persisted model ids once catalog-backed keys are known. Gated on
     catalogReady so we don't rewrite catalog-only picks (e.g. claude-haiku-4-5)
     against STATIC_MODEL_OPTIONS during the brief window before the real
     catalog fetch resolves. Also reconciles the persisted last-model slot. */
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

  /* Ensure there's always a sensible "active" pointer at the start. */
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

  const setAutoAccept = useCallback(
    (id: string, autoAccept: boolean) =>
      patchConversation(id, (c) => ({
        ...c,
        autoAccept,
        updatedAt: Date.now(),
      })),
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

  const compactConversation = useCallback(
    (id: string, marker: Message) =>
      patchConversation(id, (c) => ({
        ...c,
        messages: [marker],
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
    setAutoAccept,
    appendMessage,
    updateMessage,
    compactConversation,
  }
}

export { uid }
