/**
 * Server-backed conversation store over the session-manager worker.
 *
 * Sessions ARE the conversations: the sidebar lists `session::list`, the
 * transcript hydrates from `session::messages`, and live rendering
 * reconciles the six `session::*` trigger types (per
 * `session-manager/architecture/integration.md`). localStorage no longer
 * persists transcripts — only UI affordances (active id, last model).
 *
 * Lifecycle:
 * - "new chat" is a LOCAL DRAFT (`draft: true`, id `console-<uuid>`); the
 *   session is materialised by `ensureSession` on the first send so empty
 *   chats never litter the store.
 * - rename / model / mode changes write through `session::set-meta`. The
 *   console owns the metadata convention `{ surface, model, mode,
 *   title_manual }`; metadata replaces WHOLESALE, so the full object is
 *   always sent.
 * - delete writes through `session::delete`; the sidebar prunes on the
 *   `session::deleted` event (and optimistically).
 * - transcript content reconciles `message-added` / `message-updated`
 *   snapshots by entry, keeping the highest revision per entry
 *   (at-least-once, unordered delivery).
 *
 * When `serverEnabled` is false (mock backends / Storybook) everything is
 * in-memory drafts with the same API surface.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  deleteSession,
  ensureSession as ensureSessionApi,
  fetchTranscript,
  getSession,
  listSessions,
  setSessionMeta,
} from '@/lib/sessions/api'
import {
  applyEntryUpsert,
  clearTransientFlags,
  transcriptToMessages,
} from '@/lib/sessions/entry-mapper'
import {
  subscribeSessionDirectory,
  subscribeSessionTranscript,
} from '@/lib/sessions/events'
import type { SessionMeta } from '@/lib/sessions/types'
import {
  loadActiveId,
  loadLastModel,
  saveActiveId,
  saveLastModel,
  saveRecentProject,
} from '@/lib/storage'
import {
  type Conversation,
  DEFAULT_MODE,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
} from '@/types/chat'

function uid(): string {
  return Math.random().toString(36).slice(2) + Date.now().toString(36)
}

/** Draft ids double as engine session ids — minted with the console prefix. */
function newConversationId(): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `console-${crypto.randomUUID()}`
  }
  return `console-${uid()}`
}

function deriveTitle(text: string): string {
  const clean = text.replace(/\s+/g, ' ').trim().toLowerCase()
  if (!clean) return 'new chat'
  return clean.length > 32 ? `${clean.slice(0, 32)}…` : clean
}

function emptyConversation(defaultModel: ModelId | null): Conversation {
  const now = Date.now()
  return {
    id: newConversationId(),
    title: 'new chat',
    model: defaultModel,
    mode: DEFAULT_MODE,
    // No silent pre-fill: a new chat starts with NO working dir so the user
    // makes an explicit, visible choice (the picker opens to recent projects).
    // Silently inheriting the last-used dir is exactly what made a chat operate
    // in the wrong directory without the user choosing it.
    workingDir: null,
    messages: [],
    status: 'idle',
    draft: true,
    hydrated: true,
    createdAt: now,
    updatedAt: now,
  }
}

function isMode(v: unknown): v is Mode {
  return v === 'plan' || v === 'ask' || v === 'agent'
}

/** The console's session metadata convention (replaces wholesale on writes). */
function metadataFor(
  c: Pick<Conversation, 'model' | 'mode' | 'titleManual' | 'workingDir'>,
): Record<string, unknown> {
  return {
    surface: 'console',
    ...(c.model ? { model: c.model } : {}),
    mode: c.mode,
    ...(c.titleManual ? { title_manual: true } : {}),
    ...(c.workingDir ? { working_dir: c.workingDir } : {}),
  }
}

function conversationFromMeta(meta: SessionMeta): Conversation {
  const md = meta.metadata ?? {}
  return {
    id: meta.session_id,
    title: meta.title || meta.session_id,
    titleManual: md.title_manual === true,
    model:
      typeof md.model === 'string' && md.model.length > 0 ? md.model : null,
    mode: isMode(md.mode) ? md.mode : DEFAULT_MODE,
    workingDir:
      typeof md.working_dir === 'string' && md.working_dir.length > 0
        ? md.working_dir
        : null,
    parentId:
      typeof md.parent_session_id === 'string'
        ? md.parent_session_id
        : undefined,
    depth: typeof md.depth === 'number' ? md.depth : undefined,
    messages: [],
    status: meta.status,
    statusReason: meta.status_reason,
    hydrated: false,
    createdAt: meta.created_at,
    updatedAt: meta.updated_at,
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
  /** Per-session working directory; only meaningful while the chat is a draft. */
  setWorkingDir: (id: string, dir: string) => void
  appendMessage: (id: string, message: Message) => void
  updateMessage: (id: string, messageId: string, patch: MessagePatch) => void
  compactConversation: (id: string, marker: Message) => void
  /**
   * Materialise a draft conversation in session-manager before the first
   * send (idempotent). `titleHint` seeds the session title from the prompt.
   */
  ensureSession: (id: string, titleHint?: string) => Promise<void>
}

/**
 * @param catalogKeysForValidation When set, non-matching `conversation.model`
 *   values are rewritten to the first key (catalog load / migration);
 *   `catalogReady` gates it so a stale placeholder catalog can't clobber picks.
 * @param serverEnabled Wire the store to session-manager (real backend only).
 */
export function useConversations(
  catalogKeysForValidation?: readonly string[],
  catalogReady?: boolean,
  serverEnabled?: boolean,
): ConversationsApi {
  const catalogSig =
    catalogKeysForValidation && catalogKeysForValidation.length > 0
      ? [...catalogKeysForValidation].sort().join('\u0001')
      : ''

  const [conversations, setConversations] = useState<Conversation[]>(() => [
    /* Always boot with one local draft so the chat surface has something to
       render. Done in the initializer so StrictMode's double-invoke can't
       create two. */
    emptyConversation(loadLastModel()),
  ])
  const [activeId, setActiveId] = useState<string | null>(() => loadActiveId())

  /** Highest seen `message-updated` revision per (session, entry). */
  const revisionsRef = useRef(new Map<string, Map<string, number>>())

  const patchConversation = useCallback(
    (id: string, patch: (c: Conversation) => Conversation) => {
      setConversations((list) => list.map((c) => (c.id === id ? patch(c) : c)))
    },
    [],
  )

  /* ── Boot: list sessions from session-manager ─────────────────────────── */
  useEffect(() => {
    if (!serverEnabled) return
    let cancelled = false
    void listSessions()
      .then((metas) => {
        if (cancelled) return
        setConversations((prev) => {
          const drafts = prev.filter((c) => c.draft)
          const byId = new Map(prev.map((c) => [c.id, c]))
          const listed = metas.map((meta) => {
            const existing = byId.get(meta.session_id)
            const mapped = conversationFromMeta(meta)
            if (!existing || existing.draft) return mapped
            // Keep already-reconciled transcript state across re-lists.
            return {
              ...mapped,
              messages: existing.messages,
              hydrated: existing.hydrated,
            }
          })
          const listedIds = new Set(listed.map((c) => c.id))
          return [...drafts.filter((c) => !listedIds.has(c.id)), ...listed]
        })
      })
      .catch((err) => {
        if (import.meta.env.DEV) {
          console.warn('[conversations] session::list failed', err)
        }
      })
    return () => {
      cancelled = true
    }
  }, [serverEnabled])

  /* ── Sidebar-level live events (all sessions) ─────────────────────────── */
  useEffect(() => {
    if (!serverEnabled) return
    let cancelled = false
    let off: (() => void) | null = null
    void getIiiClient().then((client) => {
      if (cancelled) return
      off = subscribeSessionDirectory(client, {
        onCreated: (event) => {
          setConversations((prev) => {
            const existing = prev.find((c) => c.id === event.session_id)
            if (existing) {
              // The first send materialises the draft; flip it server-backed.
              return prev.map((c) =>
                c.id === event.session_id
                  ? {
                      ...c,
                      draft: false,
                      title: c.draft ? c.title : event.title,
                    }
                  : c,
              )
            }
            const stub: Conversation = {
              id: event.session_id,
              title: event.title || event.session_id,
              model: null,
              mode: DEFAULT_MODE,
              messages: [],
              status: event.status,
              hydrated: false,
              createdAt: event.created_at,
              updatedAt: event.created_at,
            }
            return [stub, ...prev]
          })
          // `session::created` carries no metadata (by contract), so a spawned
          // sub-agent / workflow node arrives here with no parent link and would
          // sit flat until a reload re-lists it. Fetch its meta once to learn
          // parent_session_id so it nests live in the sidebar tree.
          void getSession(event.session_id)
            .then((meta) => {
              if (cancelled || !meta) return
              const md = meta.metadata ?? {}
              const parentId =
                typeof md.parent_session_id === 'string'
                  ? md.parent_session_id
                  : undefined
              if (!parentId) return
              const depth = typeof md.depth === 'number' ? md.depth : undefined
              patchConversation(event.session_id, (c) => ({
                ...c,
                parentId,
                depth,
              }))
            })
            .catch(() => {})
        },
        onMetaUpdated: (event) => {
          patchConversation(event.session_id, (c) => {
            const md = event.metadata ?? {}
            return {
              ...c,
              title: event.title || c.title,
              titleManual: md.title_manual === true || c.titleManual,
              model:
                typeof md.model === 'string' && md.model.length > 0
                  ? md.model
                  : c.model,
              mode: isMode(md.mode) ? md.mode : c.mode,
              parentId:
                typeof md.parent_session_id === 'string'
                  ? md.parent_session_id
                  : c.parentId,
              depth: typeof md.depth === 'number' ? md.depth : c.depth,
              updatedAt: event.timestamp,
            }
          })
        },
        onStatusChanged: (event) => {
          patchConversation(event.session_id, (c) => ({
            ...c,
            status: event.status,
            statusReason: event.status_reason,
            // Turn over: drop dangling streaming/running flags.
            messages:
              event.status === 'working'
                ? c.messages
                : clearTransientFlags(c.messages),
            updatedAt: event.timestamp,
          }))
        },
        onDeleted: (event) => {
          setConversations((list) =>
            list.filter((c) => c.id !== event.session_id),
          )
          revisionsRef.current.delete(event.session_id)
          setActiveId((current) =>
            current === event.session_id ? null : current,
          )
        },
      })
    })
    return () => {
      cancelled = true
      off?.()
    }
  }, [serverEnabled, patchConversation])

  /* ── Active-session transcript: live reconcile + hydration ────────────── */
  const activeIsServerBacked = useMemo(() => {
    if (!serverEnabled || !activeId) return false
    const conv = conversations.find((c) => c.id === activeId)
    return Boolean(conv && !conv.draft)
  }, [serverEnabled, activeId, conversations])

  useEffect(() => {
    if (!activeIsServerBacked || !activeId) return
    const sessionId = activeId
    let cancelled = false
    let off: (() => void) | null = null

    const revisionsFor = (sid: string) => {
      let revs = revisionsRef.current.get(sid)
      if (!revs) {
        revs = new Map()
        revisionsRef.current.set(sid, revs)
      }
      return revs
    }

    void getIiiClient().then((client) => {
      if (cancelled) return
      off = subscribeSessionTranscript(client, sessionId, {
        onMessageAdded: (event) => {
          patchConversation(sessionId, (c) => ({
            ...c,
            messages: applyEntryUpsert(
              c.messages,
              {
                entry_id: event.entry_id,
                message: event.message,
                custom: event.custom,
              },
              { sessionId },
            ),
            updatedAt: event.timestamp,
          }))
        },
        onMessageUpdated: (event) => {
          const revs = revisionsFor(sessionId)
          const prev = revs.get(event.entry_id) ?? -1
          if (event.revision <= prev) return
          revs.set(event.entry_id, event.revision)
          patchConversation(sessionId, (c) => ({
            ...c,
            messages: applyEntryUpsert(
              c.messages,
              { entry_id: event.entry_id, message: event.message },
              { sessionId, streaming: c.status === 'working' },
            ),
            updatedAt: event.timestamp,
          }))
        },
      })
    })

    return () => {
      cancelled = true
      off?.()
    }
  }, [activeIsServerBacked, activeId, patchConversation])

  /* Hydrate the active conversation's transcript once (read-back, then the
     live subscription above keeps it current). Folding through
     applyEntryUpsert makes the read idempotent against events that raced in
     while the fetch was in flight. */
  useEffect(() => {
    if (!activeIsServerBacked || !activeId) return
    const conv = conversations.find((c) => c.id === activeId)
    if (!conv || conv.hydrated) return
    const sessionId = activeId
    let cancelled = false
    void fetchTranscript(sessionId)
      .then((items) => {
        if (cancelled) return
        patchConversation(sessionId, (c) => {
          let messages = transcriptToMessages(items, sessionId)
          // Re-apply anything the live feed already reconciled on top.
          for (const m of c.messages) {
            if (!messages.some((existing) => existing.id === m.id)) {
              messages = [...messages, m]
            }
          }
          return { ...c, messages, hydrated: true }
        })
      })
      .catch((err) => {
        if (import.meta.env.DEV) {
          console.warn(
            '[conversations] transcript hydration failed',
            sessionId,
            err,
          )
        }
      })
    return () => {
      cancelled = true
    }
  }, [activeIsServerBacked, activeId, conversations, patchConversation])

  /* Migrate model ids once catalog-backed keys are known (local-only; the
     server metadata is rewritten on the next explicit model change). Gated
     on catalogReady so a stale placeholder catalog can't clobber picks. */
  useEffect(() => {
    if (!catalogSig) return
    if (catalogReady === false) return
    const keys = catalogSig.split('\u0001')
    const valid = new Set(keys)
    const fallback = keys[0]
    setConversations((prev) => {
      let changed = false
      const next = prev.map((c) => {
        if (c.model && valid.has(c.model)) return c
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

  const createNew = useCallback(() => {
    const next = emptyConversation(loadLastModel())
    setConversations((list) => [next, ...list])
    setActiveId(next.id)
    return next.id
  }, [])

  const select = useCallback((id: string) => setActiveId(id), [])

  const rename = useCallback(
    (id: string, title: string) => {
      const trimmed = title.trim()
      patchConversation(id, (c) => ({
        ...c,
        title: trimmed || c.title,
        titleManual: true,
        updatedAt: Date.now(),
      }))
      if (!serverEnabled || !trimmed) return
      const conv = conversations.find((c) => c.id === id)
      if (!conv || conv.draft) return
      void setSessionMeta({
        session_id: id,
        title: trimmed,
        metadata: metadataFor({ ...conv, titleManual: true }),
      }).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] rename failed', err)
      })
    },
    [patchConversation, serverEnabled, conversations],
  )

  const remove = useCallback(
    (id: string) => {
      const conv = conversations.find((c) => c.id === id)
      setConversations((list) => list.filter((c) => c.id !== id))
      revisionsRef.current.delete(id)
      setActiveId((current) => (current === id ? null : current))
      if (!serverEnabled || !conv || conv.draft) return
      void deleteSession(id).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] delete failed', err)
      })
    },
    [serverEnabled, conversations],
  )

  const writeMeta = useCallback(
    (conv: Conversation) => {
      if (!serverEnabled || conv.draft) return
      void setSessionMeta({
        session_id: conv.id,
        metadata: metadataFor(conv),
      }).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] set_meta failed', err)
      })
    },
    [serverEnabled],
  )

  const setModel = useCallback(
    (id: string, model: ModelId) => {
      patchConversation(id, (c) => ({ ...c, model, updatedAt: Date.now() }))
      saveLastModel(model)
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta({ ...conv, model })
    },
    [patchConversation, conversations, writeMeta],
  )

  const setMode = useCallback(
    (id: string, mode: Mode) => {
      patchConversation(id, (c) => ({ ...c, mode, updatedAt: Date.now() }))
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta({ ...conv, mode })
    },
    [patchConversation, conversations, writeMeta],
  )

  const setWorkingDir = useCallback(
    (id: string, dir: string) => {
      patchConversation(id, (c) => ({
        ...c,
        workingDir: dir,
        updatedAt: Date.now(),
      }))
      saveRecentProject(dir)
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta({ ...conv, workingDir: dir })
    },
    [patchConversation, conversations, writeMeta],
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

  const ensureSession = useCallback(
    async (id: string, titleHint?: string) => {
      const conv = conversations.find((c) => c.id === id)
      if (!serverEnabled || !conv?.draft) return
      const title = conv.titleManual
        ? conv.title
        : titleHint
          ? deriveTitle(titleHint)
          : conv.title
      try {
        await ensureSessionApi({
          session_id: id,
          title,
          metadata: metadataFor(conv),
        })
        patchConversation(id, (c) => ({
          ...c,
          draft: false,
          hydrated: true,
          title,
          status: 'idle',
          updatedAt: Date.now(),
        }))
      } catch (err) {
        if (import.meta.env.DEV) {
          console.warn('[conversations] session::ensure failed', err)
        }
        throw err
      }
    },
    [serverEnabled, conversations, patchConversation],
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
    setWorkingDir,
    appendMessage,
    updateMessage,
    compactConversation,
    ensureSession,
  }
}

export { uid }
