import { ArrowLeft } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FilesystemAccessDialog } from '@/components/permissions/FilesystemAccessDialog'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { FullPermissionsBanner } from '@/components/permissions/FullPermissionsBanner'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { PageHeader } from '@/components/ui/PageChrome'
import { StatusDot } from '@/components/ui/StatusDot'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { useApprovalSettings } from '@/hooks/use-approval-settings'
import { uid } from '@/hooks/use-conversations'
import { useFilesystemGrants } from '@/hooks/use-filesystem-grants'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import {
  harnessComposerPlaceholder,
  isChatBlockedByHarness,
  isHarnessAvailable,
} from '@/hooks/use-harness-status'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { useWorktreeBinding } from '@/hooks/use-worktree-binding'
import { useWorktreeEvents } from '@/hooks/use-worktree-events'
import { expandAttachments, hasExpandableAttachments } from '@/lib/attachments'
import type { ChatBackend } from '@/lib/backend'
import { approvalBelongsToConversationTree } from '@/lib/backend/approval-events-live'
import {
  type HarnessImageBlock,
  predictedUserEntryId,
} from '@/lib/backend/harness-send'
import { serialRefresh } from '@/lib/backend/serial-refresh'
import {
  mergeFiredTriggers,
  type SessionTriggerInfo,
} from '@/lib/backend/triggers'
import type {
  ApprovalStreamEvent,
  CompactResult,
  QueuedMessagePreview,
} from '@/lib/backend/types'
import { requestComposerFocus } from '@/lib/composer-insert'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { syncEditorWorkspace } from '@/lib/editor-sync'
import { expandFileMentions, parseFileMentions } from '@/lib/file-mentions'
import { formatStopReason } from '@/lib/format-stop-reason'
import { newMessageId } from '@/lib/session-id'
import {
  expandSlashInvocation,
  loadedSkillIds,
  slashChip,
} from '@/lib/slash-commands'
import {
  CHAT_FOCUS_DROP_GRACE_MS,
  clearChatMessageFocus,
  shouldDropChatFocus,
  useChatMessageFocus,
} from '@/lib/trace-links'
import { turnAnchorMessageId } from '@/lib/turn-anchor'
import { useExtSessionChips, useExtSessionTurnSummaries } from '@/lib/ui-slots'
import {
  activateWorkingDir,
  fetchDefaultWorkingDir,
  workingDirRecoveryNotice,
} from '@/lib/working-dir'
import { onWorkingDirectoryChangeRequest } from '@/lib/working-directory-request'
import {
  consoleClaimFor,
  recordConsoleClaim,
  releaseConsoleClaimIfAny,
} from '@/lib/worktree-claims'
import {
  claimWorktree,
  formatLandBlockedNotice,
  formatLandedNotice,
  type WorktreeInfo,
  type WorktreeLandBlockedEvent,
  type WorktreeLandedEvent,
} from '@/lib/worktrees'
import {
  type AssistantMessage,
  type Attachment,
  type Conversation,
  DEFAULT_THINKING_LEVEL,
  type FunctionTriggerMessage,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
  type ModelOption,
  type SystemMessage,
  type ThinkingLevel,
  type ThoughtMessage,
  type TriggerFiredData,
  type UserMessage,
} from '@/types/chat'
import type { PageCommandsApi } from '@/types/injectable-ui'
import { ActiveSubagentChips } from './ActiveSubagentChips'
import { Composer, type ComposerSubmitPayload } from './Composer'
import { ContextUsage } from './ContextUsage'
import { isSessionSubmitBlockedByHydration } from './chat-submit-blocking'
import { MessageList } from './MessageList'
import { SessionTriggers } from './SessionTriggers'
import {
  agentIdForSend,
  DEFAULT_SYSTEM_PROMPT_STATE,
  selectionForSend,
  skillSelectionForSend,
} from './system-prompt-selection'

/**
 * Order the header's injected chips deterministically. The registry appends
 * in registration order, which is worker-CONNECT order — so without this the
 * bar reshuffles itself between restarts. `context` leads (it is the widest
 * and the most-read), the rest sort by id.
 */
function compareChips(a: { id: string }, b: { id: string }): number {
  if (a.id === b.id) return 0
  if (a.id === 'context') return -1
  if (b.id === 'context') return 1
  return a.id < b.id ? -1 : 1
}

function isAbortError(err: unknown): boolean {
  return (
    err instanceof DOMException &&
    (err.name === 'AbortError' || err.code === DOMException.ABORT_ERR)
  )
}

function makeSystemNotice(
  content: string,
  tone: 'info' | 'warn' | 'error' = 'info',
): SystemMessage {
  return { id: uid(), role: 'system', content, tone, createdAt: Date.now() }
}

function formatCompactResult(result: CompactResult): string {
  switch (result.status) {
    case 'ok':
      return `compacted — ${result.tokensBefore.toLocaleString()} tokens summarised${
        result.autoContinued ? ' (continued)' : ''
      }`
    case 'busy':
      return 'compact: another compaction is in progress — try again in a moment'
    case 'overflow':
      return `compact failed: ${result.message}`
    case 'empty':
      return 'compact: session is too small to summarise — try again after a few more turns'
    case 'error':
      return `compact failed: ${result.message}`
  }
}

function compactResultTone(result: CompactResult): 'info' | 'warn' | 'error' {
  if (result.status === 'ok') return 'info'
  if (result.status === 'error') return 'error'
  return 'warn'
}

interface ChatViewProps {
  conversation: Conversation
  backend: ChatBackend
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  density?: 'route' | 'dock'
  /** Header label for a session-pinned workspace panel. */
  panelTitle?: string
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: chat's keys and palette rows. */
  commands?: PageCommandsApi
  /**
   * Drill-out affordance for narrow (one-pane-at-a-time) hosts: when
   * set, the header renders a ← back button returning to the session
   * list and the chrome tightens to the compact (dock) padding.
   */
  onBack?: () => void
  onUpdateModel: (id: string, model: ModelId) => void
  onUpdateThinkingLevel: (id: string, level: ThinkingLevel) => void
  onUpdateMode: (id: string, mode: Mode) => void
  onUpdateWorkingDir: (id: string, dir: string | null) => void
  onAppendMessage: (id: string, message: Message) => void
  onPatchMessage: (id: string, messageId: string, patch: MessagePatch) => void
  onCompactConversation: (id: string, marker: Message) => void
}

export function ChatView({
  conversation,
  backend,
  modelOptions,
  catalogLoading,
  density = 'route',
  panelTitle,
  onRequestClose,
  commands,
  onBack,
  onUpdateModel,
  onUpdateThinkingLevel,
  onUpdateMode,
  onUpdateWorkingDir,
  onAppendMessage,
  onPatchMessage,
  onCompactConversation,
}: ChatViewProps) {
  const [isStreaming, setIsStreaming] = useState(false)
  // Pre-content phase of this tab's in-flight send, for the thinking
  // waiting indicator detail: submit → harness::send ack → turn-started.
  // Null once content streams (or for turns this tab didn't start).
  const [turnPhase, setTurnPhase] = useState<
    'sending' | 'accepted' | 'merged' | 'started' | null
  >(null)
  const thinkingLevel = conversation.thinkingLevel ?? DEFAULT_THINKING_LEVEL
  const [modelPickerOpenRequest, setModelPickerOpenRequest] = useState<
    number | undefined
  >(undefined)
  const handleOpenModelPicker = useCallback(() => {
    setModelPickerOpenRequest((current) => (current ?? 0) + 1)
  }, [])
  const handleThinkingLevelChange = useCallback(
    (next: ThinkingLevel) => onUpdateThinkingLevel(conversation.id, next),
    [conversation.id, onUpdateThinkingLevel],
  )
  /* Lives on the conversation record, not in local state: the interactive
     picker is on the new-session screen, so a reset on a tab switch (ChatPanel
     keys this view by conversation id) would be invisible. */
  const effectiveSystemPrompt =
    conversation.systemPrompt ?? DEFAULT_SYSTEM_PROMPT_STATE
  const abortRef = useRef<AbortController | null>(null)
  const { functionEntries } = useFunctionsCatalog(backend.id)
  const conversationsCtx = useConversationsCtxOptional()
  const workingDirEnabled =
    backend.id === 'real' &&
    (conversationsCtx ? conversationsCtx.shellAvailable : false)
  const workingDirRef = useRef(conversation.workingDir ?? null)
  workingDirRef.current = conversation.workingDir ?? null
  const workingDirActivationRef = useRef<{
    path: string
    token: symbol
    promise: Promise<string | null>
  } | null>(null)
  const [workingDirResolving, setWorkingDirResolving] = useState(false)
  const harnessBlocked = conversationsCtx
    ? isChatBlockedByHarness(conversationsCtx.harnessStatus)
    : false
  // The session list can render a server session before its transcript read
  // finishes. Until then, an empty local message list says nothing about
  // whether this is the first turn, so defer the whole request.
  const sessionHydrating = isSessionSubmitBlockedByHydration({
    realBackend: backend.id === 'real',
    draft: conversation.draft,
    hydrated: conversation.hydrated,
  })
  const submitBlocked =
    harnessBlocked || sessionHydrating || workingDirResolving
  const submitBlockedRef = useRef(submitBlocked)
  submitBlockedRef.current = submitBlocked
  // This view is keyed by conversation, so mounting IS opening a session:
  // the caret belongs in the composer, on the devices where that is free.
  const focusComposerOnOpen = useMediaQuery(DESKTOP_POINTER_QUERY)

  /* What the model on the other end can do with a picture, read at send time
     rather than closed over: the send and edit-queued callbacks are built
     before the catalog lookup below, and a model switched between typing and
     sending has to be the one the guard judges. Filled in further down. */
  const visionRef = useRef<{ supports?: boolean; model: string | null }>({
    supports: undefined,
    model: null,
  })

  // Live view of the transcript for the long-running stream loop: the
  // session-events reconciler (use-conversations) may add/replace rows while
  // a turn is in flight, and dedupe-by-functionTriggerId must see them.
  const messagesRef = useRef(conversation.messages)
  messagesRef.current = conversation.messages

  // Driver-owned session status from session-manager (status-changed events);
  // covers reloads and turns observed from another tab. Local isStreaming
  // covers the gap before the first status event lands.
  const serverWorking = conversation.status === 'working'
  const streamingIndicator = isStreaming || serverWorking

  // Stop requested but not yet finalized server-side. Disables the stop button
  // until status-changed flips the indicator off. The ref is the dedupe guard:
  // synchronous (two clicks in one frame can't both pass, unlike closure
  // state) and readable outside a state updater (React forbids side effects
  // inside updaters — Strict Mode double-invokes them).
  const [stopping, setStopping] = useState(false)
  const stopRequestedRef = useRef(false)
  // Sibling latch for the queue-drain flag: unlike stopRequestedRef it is
  // NEVER reset by the abortRun-failure retry path or the indicator effect —
  // only by the next stream this tab opens. A stream that saw ANY stop
  // attempt must not claim its leftovers are `triggering…` (a cancelled
  // turn's queue lands in the transcript unreacted).
  const stopSeenRef = useRef(false)
  useEffect(() => {
    if (!streamingIndicator) {
      stopRequestedRef.current = false
      setStopping(false)
    }
  }, [streamingIndicator])

  // Messages queued mid-stream (MOT-3837): shown above the composer until the
  // harness drains them into the transcript. Each draft carries the predicted
  // entry id of its eventual transcript row.
  const [queuedDrafts, setQueuedDrafts] = useState<UserMessage[]>([])
  // Queue rows read `triggering…` instead of `queued` while this is set —
  // see the drain-window block below `queuedStrip` for who sets/clears it.
  const [drainingQueue, setDrainingQueue] = useState(false)

  // Drafts belong to one conversation; never leak across switches. The
  // drain flag rides along — it describes the previous conversation's turn.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on id change only
  useEffect(() => {
    setQueuedDrafts([])
    setDrainingQueue(false)
  }, [conversation.id])

  // Pop a draft into the chat the moment its drained row (same predicted
  // entry id) arrives via session events — the transcript owns it from there.
  useEffect(() => {
    if (queuedDrafts.length === 0) return
    const ids = new Set(conversation.messages.map((m) => m.id))
    setQueuedDrafts((drafts) => {
      const next = drafts.filter((d) => !ids.has(d.id))
      return next.length === drafts.length ? drafts : next
    })
  }, [conversation.messages, queuedDrafts.length])

  // Safety flush: the turn ended (the harness's finalize drain already
  // appended any leftovers server-side), so surviving drafts become
  // optimistic transcript rows that reconcile in place when events land.
  useEffect(() => {
    if (streamingIndicator || queuedDrafts.length === 0) return
    setQueuedDrafts([])
    for (const draft of queuedDrafts) {
      onAppendMessage(conversation.id, draft)
    }
  }, [streamingIndicator, queuedDrafts, conversation.id, onAppendMessage])

  // Server-side queue: while a step streams, `harness::message-queued` events
  // signal that another tab or a subagent/subscription notification parked a
  // row — refetch `harness::status` → `queued` so the strip shows them, not
  // just this tab's drafts. One catch-up fetch on stream start covers rows
  // queued before the subscription bound; cleared when idle (the transcript
  // owns everything by then).
  const [serverQueued, setServerQueued] = useState<QueuedMessagePreview[]>([])
  useEffect(() => {
    const listQueued = backend.listQueued
    if (!streamingIndicator || !listQueued) {
      setServerQueued([])
      return
    }
    let alive = true
    // The conversation id IS the engine session_id (see `sessionId` below).
    const refresh = () =>
      listQueued(conversation.id)
        .then((rows) => {
          if (alive) setServerQueued(rows)
        })
        .catch(() => {})
    void refresh()
    const off = backend.onQueuedMessage?.(conversation.id, () => void refresh())
    return () => {
      alive = false
      off?.()
    }
  }, [
    streamingIndicator,
    backend.listQueued,
    backend.onQueuedMessage,
    conversation.id,
  ])

  // Registered trigger subscriptions (the harness's durable binding rows,
  // owned by this session): shown above the composer, unregisterable, detail
  // on click. Pushed — `harness::triggers-changed` rings on every binding
  // mutation (any tab, fires, expiry, GC) and the handler refetches the list.
  const [sessionTriggers, setSessionTriggers] = useState<SessionTriggerInfo[]>(
    [],
  )
  // Every full row this tab has EVER fetched, by subscription id. When a once
  // binding fires and retires, the refetch drops it — this cache lets the
  // fired ghost keep its full config/conditions after retirement.
  const seenTriggersRef = useRef<Map<string, SessionTriggerInfo>>(new Map())
  // The current conversation's serialized list loader. Doorbells arrive
  // at-least-once and burst on rapid fires; serialRefresh coalesces them
  // behind one in-flight read so snapshots never apply out of order.
  const triggersLoaderRef = useRef<{ refresh: () => void } | null>(null)
  const refreshTriggers = useCallback(() => {
    triggersLoaderRef.current?.refresh()
  }, [])
  useEffect(() => {
    const listTriggers = backend.listTriggers
    if (!listTriggers) return
    seenTriggersRef.current = new Map()
    setSessionTriggers([])
    const loader = serialRefresh(
      () => listTriggers(conversation.id),
      (rows) => {
        for (const row of rows) seenTriggersRef.current.set(row.id, row)
        setSessionTriggers(rows)
      },
    )
    triggersLoaderRef.current = loader
    // Subscribe BEFORE the first snapshot so a mutation in the setup gap
    // rings instead of being missed (both ride the same client bootstrap, so
    // the registration frames are queued ahead of the list read).
    const off = backend.onTriggersChanged?.(conversation.id, loader.refresh)
    loader.refresh()
    // Catch-up for doorbells missed while hidden (throttled tab). Missed
    // doorbells across a socket outage are reseeded by the backend's
    // reconnect listener.
    const onVisible = () => {
      if (document.visibilityState === 'visible') loader.refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      off?.()
      document.removeEventListener('visibilitychange', onVisible)
      // Discard any in-flight snapshot so the old conversation's rows can't
      // land in the next conversation's state.
      loader.reset()
      if (triggersLoaderRef.current === loader) triggersLoaderRef.current = null
    }
  }, [backend.listTriggers, backend.onTriggersChanged, conversation.id])

  const handleUnregisterTrigger = useCallback(
    async (subscriptionId: string) => {
      try {
        await backend.unregisterTrigger?.(subscriptionId, conversation.id)
        setSessionTriggers((rows) =>
          rows.filter((t) => t.id !== subscriptionId),
        )
      } catch (err) {
        onAppendMessage(
          conversation.id,
          makeSystemNotice(
            `could not unregister the trigger — ${err instanceof Error ? err.message : String(err)}`,
            'error',
          ),
        )
      }
      refreshTriggers()
    },
    [backend, conversation.id, onAppendMessage, refreshTriggers],
  )

  const handleClearAllTriggers = useCallback(async () => {
    const unreg = backend.unregisterTrigger
    if (!unreg) return
    const ids = sessionTriggers.map((t) => t.id)
    // Fire all unregisters, tolerate partial failure, surface a single notice.
    const results = await Promise.allSettled(
      ids.map((id) => unreg(id, conversation.id)),
    )
    const cleared = new Set(
      ids.filter((_, i) => results[i].status === 'fulfilled'),
    )
    setSessionTriggers((rows) => rows.filter((t) => !cleared.has(t.id)))
    const failed = results.filter((r) => r.status === 'rejected').length
    if (failed > 0) {
      onAppendMessage(
        conversation.id,
        makeSystemNotice(
          `could not unregister ${failed} of ${ids.length} triggers — they may have already fired or been removed`,
          'error',
        ),
      )
    }
    refreshTriggers()
  }, [
    backend,
    sessionTriggers,
    conversation.id,
    onAppendMessage,
    refreshTriggers,
  ])

  // Fired-trigger history: durable `trigger_fired` transcript entries (mapped to
  // system messages). Drives the panel's fired/unregistered ghost rows so a
  // once-trigger stays visible after the engine drops it from the list.
  const firedTriggers = useMemo<TriggerFiredData[]>(() => {
    const out: TriggerFiredData[] = []
    for (const m of conversation.messages) {
      if (m.role === 'system' && m.kind === 'trigger-fired' && m.trigger) {
        out.push(m.trigger)
      }
    }
    return out
  }, [conversation.messages])
  const mergedTriggers = useMemo(
    () =>
      mergeFiredTriggers(
        sessionTriggers,
        firedTriggers,
        seenTriggersRef.current,
      ),
    [sessionTriggers, firedTriggers],
  )
  // Registration lookup for trigger-fired cards: a fire record carries only
  // the subscription id — the binding's config/conditions live in these rows.
  const triggersById = useMemo(
    () => new Map(mergedTriggers.map((t) => [t.id, t])),
    [mergedTriggers],
  )

  // The strip's rows: this tab's drafts first, then server-queued rows not
  // already covered by a draft or an arrived transcript row (a stale poll
  // must not re-show a message that just drained into the chat).
  const queuedStrip = useMemo(() => {
    const seen = new Set<string>(queuedDrafts.map((d) => d.id))
    for (const m of conversation.messages) seen.add(m.id)
    const drafts = queuedDrafts.map((d) => ({
      id: d.id,
      text: d.content || '(attachments only)',
    }))
    const server = serverQueued
      .filter((row) => !seen.has(row.id))
      .map((row) => ({ id: row.id, text: row.text || '(notification)' }))
    return [...drafts, ...server]
  }, [queuedDrafts, serverQueued, conversation.messages])

  // Delivery window: the turn stream this tab owned finished normally while
  // messages were still parked — the harness is draining them into the
  // transcript now (`serverWorking` keeps the strip mounted until each
  // drained row arrives and pops its entry). Rows read `triggering…` instead
  // of `queued` for that stretch. The flag is set only by the stream loop's
  // own clean exit — never by /compact's isStreaming toggle, a stream error,
  // or a user stop (a cancelled turn's leftovers land in the transcript
  // unreacted, so `queued` stays truthful there) — and a tab that never
  // owned the stream just keeps `queued` until the rows pop.
  const queuedStripCountRef = useRef(0)
  queuedStripCountRef.current = queuedStrip.length
  // Empty strip: the drain finished. GROWING strip: a fresh message parked
  // (this tab or another) — it belongs to the follow-on turn's queue, so the
  // whole strip honestly reverts to `queued`. Rows popping out (shrinking)
  // keep the flag.
  const prevStripLenRef = useRef(0)
  useEffect(() => {
    const prev = prevStripLenRef.current
    prevStripLenRef.current = queuedStrip.length
    if (queuedStrip.length === 0 || queuedStrip.length > prev) {
      setDrainingQueue(false)
    }
  }, [queuedStrip.length])

  // The conversation id IS the engine session_id (console-<uuid> for chats
  // created here). Matches iii.session.id on every span so the traces UI can
  // group by it.
  const sessionId = conversation.id
  const approvalConversationsRef = useRef(
    conversationsCtx?.conversations ?? [conversation],
  )
  approvalConversationsRef.current = conversationsCtx?.conversations ?? [
    conversation,
  ]
  const approvalSessionMatcher = useCallback(
    (approval: {
      session_id: string
      session_metadata?: Record<string, unknown> | null
    }) =>
      approvalBelongsToConversationTree(
        approval,
        sessionId,
        approvalConversationsRef.current,
      ),
    [sessionId],
  )
  const approvalTreeRevision = (
    conversationsCtx?.conversations ?? [conversation]
  )
    .map((item) => `${item.id}:${item.parentId ?? ''}`)
    .join('|')

  // ↑/↓ browse this tab's queued messages for editing (oldest→newest). The
  // composer owns navigation; ChatView just supplies the list and the id of
  // whichever is being edited (for the strip highlight).
  const [browsedQueuedId, setBrowsedQueuedId] = useState<string | null>(null)
  const queuedForEdit = useMemo(
    () =>
      queuedDrafts.map((d) => ({
        id: d.id,
        text: d.content,
        attachments: d.attachments ?? [],
      })),
    [queuedDrafts],
  )

  // Submitting a browsed queued message: edit it IN PLACE (`payload`), or
  // remove it (`null` — the composer was emptied). Both keep the message where
  // it is in the queue; an edit rebuilds content the same way a send does
  // (re-expanding `#file(...)` mentions). Best-effort server call — a row that
  // already drained is a no-op; a failure means the stale version may still
  // deliver, so surface it.
  const handleEditQueued = useCallback(
    (
      id: string,
      payload: { text: string; attachments: Attachment[] } | null,
    ) => {
      const conversationId = conversation.id
      if (payload === null) {
        setQueuedDrafts((current) => current.filter((d) => d.id !== id))
        void backend.removeQueued?.(conversationId, id).catch((err) => {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not remove the queued message (${err instanceof Error ? err.message : String(err)}) — it may still be delivered when the turn ends`,
              'warn',
            ),
          )
        })
        return
      }
      // Optimistic: reflect the new content in the strip immediately, in place.
      setQueuedDrafts((current) =>
        current.map((d) =>
          d.id === id
            ? {
                ...d,
                content: payload.text,
                attachments:
                  payload.attachments.length > 0
                    ? payload.attachments
                    : undefined,
              }
            : d,
        ),
      )
      void (async () => {
        let attachedBlocks: string[] | undefined
        const workingDir = conversation.workingDir
        if (backend.id === 'real' && workingDir) {
          const mentionPaths = parseFileMentions(payload.text)
          if (mentionPaths.length > 0) {
            attachedBlocks = (
              await expandFileMentions(workingDir, mentionPaths)
            ).blocks
          }
        }
        // Same expansion as the live send path: an edited queued message
        // keeps its `/skill:<id>` block instead of silently
        // dropping it. Staying silent on a failed re-resolution would strip
        // the body the queued message already carried with no explanation.
        const slashExpansion =
          backend.id === 'real'
            ? await expandSlashInvocation(
                payload.text.trim(),
                loadedSkillIds(messagesRef.current),
              )
            : null
        if (slashExpansion?.status === 'attached') {
          attachedBlocks = [...(attachedBlocks ?? []), slashExpansion.block]
        } else if (slashExpansion) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not attach ${slashExpansion.command} — the edited message will be sent as typed`,
              'warn',
            ),
          )
        }
        // Same expansion as the live send path: a queued message's documents
        // and pictures have to reach the agent too, or editing a queued
        // message would silently drop what it carried.
        let attachedImages: HarnessImageBlock[] | undefined
        if (
          backend.id === 'real' &&
          hasExpandableAttachments(payload.attachments)
        ) {
          const expanded = await expandAttachments(payload.attachments, {
            vision: visionRef.current.supports,
            model: visionRef.current.model,
          })
          if (expanded.blocks.length > 0) {
            attachedBlocks = [...(attachedBlocks ?? []), ...expanded.blocks]
          }
          if (expanded.images.length > 0) attachedImages = expanded.images
          // Same reporting as the live send path. Staying silent here would let
          // an edited queued message lose its document with no explanation.
          for (const failure of expanded.failures) {
            onAppendMessage(
              conversationId,
              makeSystemNotice(
                `could not read ${failure.name} — ${failure.reason}`,
                'warn',
              ),
            )
          }
        }
        try {
          await backend.editQueued?.(
            conversationId,
            id,
            payload.text,
            attachedBlocks || attachedImages
              ? { attachedBlocks, attachedImages }
              : undefined,
          )
        } catch (err) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not save the edit (${err instanceof Error ? err.message : String(err)}) — the original may still be delivered when the turn ends`,
              'warn',
            ),
          )
        }
      })()
    },
    [backend, conversation.id, conversation.workingDir, onAppendMessage],
  )

  // Discovered sessions (sub-agents especially) carry no client-side model
  // choice — `conversation.model` is null. Fall back to the model the latest
  // assistant reply actually used (transcript entries carry it), resolved
  // against the catalog's composite ids so the picker preselects when it can.
  const effectiveModel = useMemo(() => {
    if (conversation.model) return conversation.model
    const last = [...conversation.messages]
      .reverse()
      .find(
        (m): m is AssistantMessage =>
          m.role === 'assistant' && Boolean(m.model),
      )
    if (!last?.model) return null
    const catalog = modelOptions.find(
      (o) => o.id === last.model || o.id.endsWith(`::${last.model}`),
    )
    return catalog?.id ?? last.model
  }, [conversation.model, conversation.messages, modelOptions])

  const contextWindow = useMemo(() => {
    const match = modelOptions.find((o) => o.id === effectiveModel)
    return match?.contextWindow
  }, [modelOptions, effectiveModel])

  /* What the send path may do with an attached picture. `undefined` when the
     catalog has no row or the router said nothing — the attachment router
     treats that as "send it", so a missing capability flag never silently
     eats an image. */
  const modelVision = useMemo(() => {
    const match = modelOptions.find((o) => o.id === effectiveModel)
    return match?.supportsVision
  }, [modelOptions, effectiveModel])
  visionRef.current = { supports: modelVision, model: effectiveModel }

  /* Injected session chips (the `chat` extension slot), rendered in the
   * header's right cluster where the built-in context meter sits. A chip
   * with id `context` supersedes the estimate-based ContextUsage meter —
   * workers with real per-turn numbers own the surface. */
  const extSessionChips = useExtSessionChips()
  const sessionChips = useMemo(() => {
    if (extSessionChips.length === 0) return null
    return [...extSessionChips].sort(compareChips).map((chip) => {
      const Chip = chip.render
      return (
        <Chip
          key={chip.id}
          sessionId={conversation.id}
          modelId={effectiveModel ?? undefined}
          contextWindow={contextWindow}
        />
      )
    })
  }, [extSessionChips, conversation.id, effectiveModel, contextWindow])
  const hasInjectedContextChip = extSessionChips.some(
    (chip) => chip.id === 'context',
  )

  /* Injected turn summaries live beside the composer rather than in the
   * transcript. Workers own their data and subscribe by session id; the host
   * only gives them the active turn state. */
  const extSessionTurnSummaries = useExtSessionTurnSummaries()
  const sessionTurnSummaries = useMemo(() => {
    if (extSessionTurnSummaries.length === 0) return null
    return [...extSessionTurnSummaries].sort(compareChips).map((summary) => {
      const Summary = summary.render
      return (
        <Summary
          key={summary.id}
          sessionId={conversation.id}
          isStreaming={streamingIndicator}
        />
      )
    })
  }, [extSessionTurnSummaries, conversation.id, streamingIndicator])

  /* Shared live region: SR announcements for auto-accept, stop-reason
   * notices, and compaction markers route through this hook. Sighted
   * users see the same messages in the transcript; visually-impaired
   * users hear them via the polite/assertive ARIA live regions
   * rendered at the bottom of the component. */
  const announcer = useLiveAnnouncer()
  const announcedApprovalIdsRef = useRef(new Set<string>())

  /* Wrap the backend's resolver in a stable callback so MessageList
   * row-level memoization isn't broken by a fresh lambda identity on
   * every render, and so the auto-accept hook's deps don't shift
   * every render. */
  const resolveApproval = useMemo(() => {
    const fn = backend.resolveApproval
    if (!fn) return undefined
    return (
      sessionId: string,
      functionTriggerId: string,
      decision: 'allow' | 'deny',
    ) => fn(sessionId, functionTriggerId, decision)
  }, [backend])

  // Approval UI + `approval::*` RPC require BOTH the harness AND the optional
  // standalone approval-gate worker. The gate owns `approval::*`; without it,
  // enabling approval would trigger "function not found", so we treat approval
  // as off and let calls run ungated.
  const approvalEnabled =
    backend.id === 'real' &&
    (conversationsCtx
      ? isHarnessAvailable(conversationsCtx.harnessStatus) &&
        conversationsCtx.approvalGateAvailable
      : false)
  const approvalSettings = useApprovalSettings(sessionId, approvalEnabled)

  const handleApprovalEvent = useCallback(
    (event: ApprovalStreamEvent) => {
      if (event.kind === 'fcall-start') {
        const existing = event.functionTriggerId
          ? messagesRef.current.find(
              (message) =>
                message.role === 'function-trigger' &&
                message.functionTriggerId === event.functionTriggerId,
            )
          : undefined
        const announcementId = event.functionTriggerId ?? existing?.id
        if (
          !announcementId ||
          !announcedApprovalIdsRef.current.has(announcementId)
        ) {
          if (announcementId) {
            announcedApprovalIdsRef.current.add(announcementId)
          }
          announcer.announceAssertive(
            event.filesystemAccess
              ? `Action required: ${event.functionId} needs approval to access ${event.filesystemAccess.requestedRoot}.`
              : `Action required: approve or deny ${event.functionId}.`,
          )
        }
        if (existing) {
          onPatchMessage(conversation.id, existing.id, {
            pendingApproval: true,
            running: false,
            functionTriggerId: event.functionTriggerId,
            sessionId: event.sessionId,
            filesystemAccess: event.filesystemAccess,
          })
          return
        }
        const message: FunctionTriggerMessage = {
          id: uid(),
          role: 'function-trigger',
          functionId: event.functionId,
          input: event.input,
          running: false,
          pendingApproval: true,
          functionTriggerId: event.functionTriggerId,
          sessionId: event.sessionId,
          filesystemAccess: event.filesystemAccess,
          createdAt: Date.now(),
        }
        onAppendMessage(conversation.id, message)
        return
      }

      const existing = messagesRef.current.find(
        (message) =>
          message.role === 'function-trigger' &&
          message.functionTriggerId === event.functionTriggerId,
      )
      announcedApprovalIdsRef.current.delete(event.functionTriggerId)
      if (existing) {
        onPatchMessage(conversation.id, existing.id, {
          pendingApproval: false,
          ...(event.running ? { running: true } : {}),
        })
      }
    },
    [
      announcer.announceAssertive,
      conversation.id,
      onAppendMessage,
      onPatchMessage,
    ],
  )

  // Subagent approvals may arrive after the parent `harness::spawn` turn has
  // completed. Keep this watcher alive for the selected conversation, with a
  // catch-up read whenever its child-session tree changes.
  useEffect(() => {
    const watchApprovals = backend.watchApprovals
    if (!approvalEnabled || !watchApprovals) return
    return watchApprovals(
      {
        sessionId,
        approvalGateAvailable: approvalEnabled,
        approvalSessionMatcher,
        refreshKey: approvalTreeRevision,
      },
      handleApprovalEvent,
    )
  }, [
    approvalEnabled,
    approvalSessionMatcher,
    approvalTreeRevision,
    backend.watchApprovals,
    handleApprovalEvent,
    sessionId,
  ])

  // The stack's default folder, resolved once (cached page-wide): pre-fills
  // fresh drafts below and keeps the launch folder selectable after a chat
  // re-scopes elsewhere.
  const [defaultWorkingDir, setDefaultWorkingDir] = useState<string | null>(
    null,
  )
  useEffect(() => {
    if (!workingDirEnabled) return
    let cancelled = false
    void fetchDefaultWorkingDir().then((dir) => {
      if (!cancelled) setDefaultWorkingDir(dir)
    })
    return () => {
      cancelled = true
    }
  }, [workingDirEnabled])

  // Default a fresh draft to the stack's current folder (MOT-3897): the
  // harness-reported launch dir, shell-validated, set only while the chat is
  // still an untouched draft (prefillWorkingDir re-checks under the patch so
  // an explicit pick or the first send racing this fetch wins). Visible
  // immediately in the picker chip — a default, not a silent inheritance.
  const prefillWorkingDir = conversationsCtx?.prefillWorkingDir
  useEffect(() => {
    if (!workingDirEnabled || !prefillWorkingDir) return
    if (!conversation.draft || conversation.workingDir != null) return
    let cancelled = false
    void fetchDefaultWorkingDir().then((dir) => {
      if (!cancelled && dir) prefillWorkingDir(conversation.id, dir)
    })
    return () => {
      cancelled = true
    }
  }, [
    workingDirEnabled,
    prefillWorkingDir,
    conversation.draft,
    conversation.workingDir,
    conversation.id,
  ])

  const reconcileWorkingDir = useCallback(
    (dir: string): Promise<string | null> => {
      const active = workingDirActivationRef.current
      if (active?.path === dir) return active.promise

      const token = Symbol(dir)
      setWorkingDirResolving(true)
      const promise = activateWorkingDir(dir)
        .then((result) => {
          if (workingDirRef.current !== dir) return workingDirRef.current

          const next = result.path
          if (result.status === 'unavailable') return next
          if (result.status === 'recovered') setDefaultWorkingDir(next)
          if (next !== null) void syncEditorWorkspace(next)
          if (result.status === 'recovered' || next !== dir) {
            workingDirRef.current = next
            onUpdateWorkingDir(conversation.id, next)
            if (
              result.status === 'recovered' &&
              !conversation.draft &&
              next !== dir
            ) {
              onAppendMessage(
                conversation.id,
                makeSystemNotice(workingDirRecoveryNotice(dir, next)),
              )
            }
          }
          return next
        })
        .finally(() => {
          if (workingDirActivationRef.current?.token !== token) return
          workingDirActivationRef.current = null
          setWorkingDirResolving(false)
        })
      workingDirActivationRef.current = { path: dir, token, promise }
      return promise
    },
    [conversation.draft, conversation.id, onAppendMessage, onUpdateWorkingDir],
  )

  // Temporary Harness projects can disappear between turns. Reconcile only
  // the conversation's current scope; per-turn filesystem metadata remains
  // bound to the original path for historical review.
  useEffect(() => {
    if (!workingDirEnabled || conversation.draft || streamingIndicator) return
    const dir = conversation.workingDir
    if (!dir) return
    void reconcileWorkingDir(dir)
  }, [
    workingDirEnabled,
    conversation.draft,
    conversation.workingDir,
    streamingIndicator,
    reconcileWorkingDir,
  ])

  // The worktrees tab, claim/release, and the landed / land-blocked live
  // events all require the optional `worktree` worker; gate the whole surface
  // on its presence like shell above.
  const worktreeEnabled =
    backend.id === 'real' &&
    (conversationsCtx ? conversationsCtx.worktreeAvailable : false)

  const handleAlwaysAllow = useMemo(() => {
    const resolveFn = backend.resolveApproval
    if (!resolveFn) return undefined
    return async (
      sId: string,
      functionTriggerId: string,
      functionId: string,
    ) => {
      await approvalSettings.approveAlways(functionId)
      await resolveFn(sId, functionTriggerId, 'allow')
      announcer.announce(`approved always this session: ${functionId}`)
    }
  }, [backend, approvalSettings, announcer])

  const filesystemGrants = useFilesystemGrants(sessionId)
  const [filesystemDialogOpen, setFilesystemDialogOpen] = useState(false)
  const handleManageFilesystemAccess = useCallback(() => {
    setFilesystemDialogOpen(true)
  }, [])

  const handleFilesystemResolve = useMemo(() => {
    const resolveFn = backend.resolveApproval
    if (!resolveFn) return undefined
    return async (
      sId: string,
      functionTriggerId: string,
      action: FilesystemAccessAction,
    ) => {
      const requestedRoot = messagesRef.current.find(
        (m): m is FunctionTriggerMessage =>
          m.role === 'function-trigger' &&
          m.functionTriggerId === functionTriggerId,
      )?.filesystemAccess?.requestedRoot

      if (action === 'deny') {
        await resolveFn(sId, functionTriggerId, 'deny')
        announcer.announce(
          requestedRoot
            ? `denied filesystem access to ${requestedRoot}`
            : 'denied filesystem access',
        )
        return
      }

      await resolveFn(sId, functionTriggerId, 'allow', {
        accessDuration: action,
      })
      if (requestedRoot && (action === 'session' || action === 'always')) {
        filesystemGrants.addOptimistic(requestedRoot)
      }
      if (requestedRoot) {
        announcer.announce(
          action === 'session'
            ? `allowed ${requestedRoot} for this session`
            : action === 'always'
              ? `permanently allowed ${requestedRoot}`
              : `allowed ${requestedRoot} for this call`,
        )
      }
    }
  }, [backend, announcer, filesystemGrants])

  const ensureSession = conversationsCtx?.ensureSession

  // Composer draft persistence: the live text is recorded per conversation
  // (and, for server-backed sessions, saved through the debounced
  // `session::set-draft`) so a page refresh restores what was typed.
  // `getDraftText` already falls back to the meta-restored value — no `??`
  // here, or a known-emptied draft (sent message) would resurrect the stale
  // boot snapshot on switch-back. The direct read covers ctx-less mounts
  // (Storybook).
  const composerInitialText = conversationsCtx
    ? conversationsCtx.getDraftText(conversation.id)
    : conversation.draftText
  const handleComposerTextChange = useCallback(
    (text: string) => {
      conversationsCtx?.setDraftText(conversation.id, text)
    },
    [conversationsCtx, conversation.id],
  )

  const handleSubmit = useCallback(
    async (payload: ComposerSubmitPayload) => {
      if (submitBlockedRef.current) return
      const conversationId = conversation.id
      // Steering a discovered/sub-agent session: inherit the model the
      // transcript shows when the conversation carries none of its own.
      const model = conversation.model ?? effectiveModel
      if (!model) {
        onAppendMessage(
          conversationId,
          makeSystemNotice('select a model before sending.', 'warn'),
        )
        return
      }

      // Materialise draft conversations in session-manager before the first
      // write (no-op for existing sessions and mock backends).
      if (backend.id === 'real' && ensureSession) {
        try {
          await ensureSession(conversationId, payload.text)
        } catch (err) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not create the session — ${err instanceof Error ? err.message : String(err)}`,
              'error',
            ),
          )
          return
        }
      }

      let workingDirForSend = conversation.workingDir
      if (workingDirEnabled && workingDirForSend) {
        workingDirForSend = await reconcileWorkingDir(workingDirForSend)
      }

      // `harness::send` carries `idempotency_key: messageId`, so the harness
      // appends the user message with the deterministic entry id
      // `e_idem_<messageId>`; using the same id here lets the
      // session::message-added snapshot reconcile this optimistic row in place.
      const messageId = newMessageId()
      const userMsg: UserMessage = {
        id: predictedUserEntryId(messageId),
        role: 'user',
        content: payload.text,
        attachments:
          payload.attachments.length > 0 ? payload.attachments : undefined,
        createdAt: Date.now(),
      }

      // Mid-stream sends are queued by the harness (MOT-3837): the message
      // waits above the composer instead of rendering mid-transcript, and pops
      // into the chat when its drained row arrives via session events.
      // `/compact` keeps the normal path (compactSession refuses while live).
      const trimmed = payload.text.trim()
      const isCompact =
        trimmed === '/compact' || trimmed.startsWith('/compact ')
      const willQueue =
        !isCompact &&
        (isStreaming || serverWorking) &&
        Boolean(backend.queueMessage)

      // Only the session's first send carries the prompt selection; the
      // harness inherits it afterwards (see selectionForSend). Gate on an
      // assistant row rather than a user row: if an earlier send failed
      // before a turn ran, there is nothing to inherit yet and the retry
      // must carry the prompt again.
      const turnEstablished = messagesRef.current.some(
        (m) => m.role === 'assistant',
      )
      // An agent selection resolves server-side (options.agent) and supplies
      // prompt + skills itself; suppressing the client-side selection also
      // covers pre-upgrade drafts whose persisted namedBody would otherwise
      // collide with the agent field.
      const agentId = agentIdForSend(effectiveSystemPrompt, {
        turnEstablished,
        willQueue,
      })
      const systemPrompt = agentId
        ? null
        : selectionForSend(effectiveSystemPrompt, turnEstablished)
      const skills = agentId
        ? undefined
        : skillSelectionForSend(conversation.skills, {
            turnEstablished,
            willQueue,
          })

      if (!willQueue) onAppendMessage(conversationId, userMsg)

      if (trimmed === '/compact' || trimmed.startsWith('/compact ')) {
        if (!backend.compactSession) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              '/compact not supported by this backend.',
              'error',
            ),
          )
          return
        }
        const pendingId = uid()
        onAppendMessage(conversationId, {
          id: pendingId,
          role: 'system',
          content: 'compacting session…',
          tone: 'info',
          createdAt: Date.now(),
        })
        setIsStreaming(true)
        try {
          const result = await backend.compactSession(
            sessionId,
            model,
            contextWindow,
          )
          if (result.status === 'ok' && backend.id === 'real') {
            // Server-backed transcript: the compaction custom entry arrives
            // via session::message-added and renders the marker (which the
            // CTX estimator anchors on); just resolve the pending notice.
            onPatchMessage(conversationId, pendingId, {
              content: formatCompactResult(result),
              tone: 'info',
            })
          } else if (result.status === 'ok') {
            const marker: SystemMessage = {
              id: uid(),
              role: 'system',
              kind: 'compaction',
              content: formatCompactResult(result),
              tone: 'info',
              summaryText: result.summaryText,
              tokensBefore: result.tokensBefore,
              createdAt: Date.now(),
            }
            onCompactConversation(conversationId, marker)
          } else {
            onPatchMessage(conversationId, pendingId, {
              content: formatCompactResult(result),
              tone: compactResultTone(result),
            })
          }
        } catch (err) {
          onPatchMessage(conversationId, pendingId, {
            content: `compact failed: ${err instanceof Error ? err.message : String(err)}`,
            tone: 'error',
          })
        } finally {
          setIsStreaming(false)
        }
        return
      }

      // Expand `#file(...)` mentions into attachment blocks (real backend
      // with a working dir only). Failures never block the send — a failed
      // mention becomes a placeholder block plus a warn notice.
      let attachedBlocks: string[] | undefined
      const workingDir = workingDirForSend
      const mentionPaths =
        backend.id === 'real' && workingDir
          ? parseFileMentions(payload.text)
          : []
      if (workingDir && mentionPaths.length > 0) {
        const expanded = await expandFileMentions(workingDir, mentionPaths)
        attachedBlocks = expanded.blocks
        if (expanded.attachments.length > 0 && !willQueue) {
          onPatchMessage(conversationId, userMsg.id, {
            attachments: [
              ...(userMsg.attachments ?? []),
              ...expanded.attachments.map((a) => ({
                id: `mention-${a.path}`,
                name: a.path,
                size: a.size,
                type: 'text/x-file-mention',
              })),
            ],
          })
        }
        for (const failure of expanded.failures) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not attach ${failure.path} — ${failure.reason}`,
              'warn',
            ),
          )
        }
      }

      // Attachments are not text. A PDF or an office document read as bytes
      // reaches the model as noise, and an image reaches it as nothing at all,
      // so each kind is expanded on this machine first: documents into
      // `<attached-file …>` markdown blocks, pictures into image content
      // blocks. Failures never block the send — an unreadable attachment
      // becomes a placeholder block plus a warn notice, so the model knows it
      // was handed something that could not be read.
      let attachedImages: HarnessImageBlock[] | undefined
      if (
        backend.id === 'real' &&
        hasExpandableAttachments(payload.attachments)
      ) {
        const expanded = await expandAttachments(payload.attachments, {
          vision: visionRef.current.supports,
          model: visionRef.current.model,
        })
        if (expanded.blocks.length > 0) {
          attachedBlocks = [...(attachedBlocks ?? []), ...expanded.blocks]
        }
        if (expanded.images.length > 0) attachedImages = expanded.images
        // Drop the source bytes and relabel the chip with what the expansion
        // made of each attachment. The relabel runs before the model is called,
        // so it never shows up as a function call — without it a person has no
        // way to tell the document was read at all.
        //
        // The `file` removal is NOT conditional on anything having been read:
        // an attachment that failed, or an image refused for a model that
        // cannot see, has finished its job too, and keeping its bytes would
        // hold the whole file in memory for as long as the conversation stays
        // open. Only the label depends on a matching entry.
        if (!willQueue) {
          const byId = new Map(expanded.read.map((r) => [r.id, r.label]))
          onPatchMessage(conversationId, userMsg.id, {
            attachments: (userMsg.attachments ?? []).map(({ file, ...a }) => {
              void file
              const label = byId.get(a.id)
              return label ? { ...a, name: label } : a
            }),
          })
        }
        for (const failure of expanded.failures) {
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not read ${failure.name} — ${failure.reason}`,
              'warn',
            ),
          )
        }
      }

      // A leading `/skill:<id>` the palette offered expands here: the resolved body rides as another attachment
      // block while the typed text (command + args) stays the user message.
      // Prose that merely starts with a slash never resolves (the expander
      // is gated on the palette's fetched entries).
      const slashExpansion =
        backend.id === 'real'
          ? await expandSlashInvocation(
              trimmed,
              loadedSkillIds(messagesRef.current),
            )
          : null
      if (slashExpansion?.status === 'attached') {
        attachedBlocks = [...(attachedBlocks ?? []), slashExpansion.block]
        // The body travels as a block, never as visible text — show the same
        // chip the hydrated transcript will collapse the block into.
        if (!willQueue) {
          onPatchMessage(conversationId, userMsg.id, {
            attachments: [
              ...(userMsg.attachments ?? []),
              slashChip(slashExpansion.inv, slashExpansion.block.length),
            ],
          })
        }
      } else if (slashExpansion) {
        onAppendMessage(
          conversationId,
          makeSystemNotice(
            `could not attach ${slashExpansion.command} — sending the message as typed`,
            'warn',
          ),
        )
      }

      // Mid-stream send (MOT-3837): a turn is already streaming, so the
      // harness queues the message and delivers it when the stream ends. No
      // second stream loop — the live one keeps rendering. The draft chip
      // above the composer stands in until the drained row (same predicted
      // entry id) arrives via session events and pops it into the chat.
      if (willQueue && backend.queueMessage) {
        setQueuedDrafts((drafts) => [...drafts, userMsg])
        try {
          await backend.queueMessage(
            payload.text || '(attachments only)',
            conversation.mode,
            model,
            {
              sessionId,
              messageId,
              thinkingLevel,
              systemPrompt,
              skills,
              ...(agentId ? { agent: agentId } : {}),
              workingDir: workingDirForSend,
              approvalGateAvailable: approvalEnabled,
              ...(attachedBlocks && attachedBlocks.length > 0
                ? { attachedBlocks }
                : {}),
              ...(attachedImages && attachedImages.length > 0
                ? { attachedImages }
                : {}),
            },
          )
        } catch (err) {
          setQueuedDrafts((drafts) => drafts.filter((d) => d.id !== userMsg.id))
          onAppendMessage(
            conversationId,
            makeSystemNotice(
              `could not queue the message — ${err instanceof Error ? err.message : String(err)}`,
              'error',
            ),
          )
        }
        return
      }

      const controller = new AbortController()
      abortRef.current = controller
      setIsStreaming(true)
      setDrainingQueue(false)
      stopSeenRef.current = false
      setTurnPhase('sending')

      let thoughtId: string | null = null
      let thoughtBuffer = ''
      let fcallId: string | null = null
      const fcallMap = new Map<string, string>()
      let assistantId: string | null = null
      let assistantBuffer = ''
      let streamEndedNormally = false

      try {
        for await (const event of backend.stream(
          payload.text || '(attachments only)',
          conversation.mode,
          model,
          {
            signal: controller.signal,
            sessionId,
            messageId,
            thinkingLevel,
            systemPrompt,
            skills,
            ...(agentId ? { agent: agentId } : {}),
            workingDir: workingDirForSend,
            approvalGateAvailable: approvalEnabled,
            approvalSessionMatcher,
            approvalEventsExternallyManaged: true,
            ...(attachedBlocks && attachedBlocks.length > 0
              ? { attachedBlocks }
              : {}),
            ...(attachedImages && attachedImages.length > 0
              ? { attachedImages }
              : {}),
          },
        )) {
          switch (event.kind) {
            case 'thought-start': {
              const msg: ThoughtMessage = {
                id: uid(),
                role: 'thought',
                content: '',
                durationMs: 0,
                streaming: true,
                createdAt: Date.now(),
              }
              thoughtId = msg.id
              thoughtBuffer = ''
              onAppendMessage(conversationId, msg)
              break
            }
            case 'thought-token': {
              if (!thoughtId) break
              thoughtBuffer += event.token
              onPatchMessage(conversationId, thoughtId, {
                content: thoughtBuffer,
              })
              break
            }
            case 'thought-end': {
              if (!thoughtId) break
              onPatchMessage(conversationId, thoughtId, {
                streaming: false,
                durationMs: event.durationMs,
              })
              break
            }
            case 'fcall-start': {
              if (event.functionTriggerId) {
                // Dedupe against rows created earlier in this stream AND
                // rows the session-events reconciler derived from the
                // assistant entry's function_call blocks.
                const existing =
                  [...fcallMap.entries()].find(
                    ([, fcid]) => fcid === event.functionTriggerId,
                  )?.[0] ??
                  messagesRef.current.find(
                    (m) =>
                      m.role === 'function-trigger' &&
                      m.functionTriggerId === event.functionTriggerId,
                  )?.id
                if (existing) {
                  onPatchMessage(conversationId, existing, {
                    pendingApproval: event.pendingApproval,
                    running: !event.pendingApproval,
                    functionTriggerId: event.functionTriggerId,
                    sessionId: event.sessionId,
                    filesystemAccess: event.filesystemAccess,
                  })
                  fcallId = existing
                  break
                }
              }
              if (assistantId) {
                onPatchMessage(conversationId, assistantId, {
                  streaming: false,
                })
                assistantId = null
                assistantBuffer = ''
              }
              const msg: FunctionTriggerMessage = {
                id: uid(),
                role: 'function-trigger',
                functionId: event.functionId,
                input: event.input,
                running: !event.pendingApproval,
                pendingApproval: event.pendingApproval,
                functionTriggerId: event.functionTriggerId,
                sessionId: event.sessionId,
                filesystemAccess: event.filesystemAccess,
                createdAt: Date.now(),
              }
              fcallId = msg.id
              if (event.functionTriggerId)
                fcallMap.set(msg.id, event.functionTriggerId)
              onAppendMessage(conversationId, msg)
              break
            }
            case 'fcall-end': {
              // Prefer targeting by functionTriggerId (parallel batches end out
              // of order; the row may also be an entry-derived segment).
              const targetId: string | null = event.functionTriggerId
                ? ([...fcallMap.entries()].find(
                    ([, fcid]) => fcid === event.functionTriggerId,
                  )?.[0] ??
                  messagesRef.current.find(
                    (m) =>
                      m.role === 'function-trigger' &&
                      m.functionTriggerId === event.functionTriggerId,
                  )?.id ??
                  null)
                : fcallId
              if (!targetId) break
              onPatchMessage(conversationId, targetId, {
                output: event.output,
                durationMs: event.durationMs,
                running: false,
                pendingApproval: false,
              })
              fcallMap.delete(targetId)
              if (targetId === fcallId) fcallId = null
              break
            }
            case 'fcall-approval-cleared': {
              const clearedId =
                [...fcallMap.entries()].find(
                  ([, fcid]) => fcid === event.functionTriggerId,
                )?.[0] ??
                messagesRef.current.find(
                  (m) =>
                    m.role === 'function-trigger' &&
                    m.functionTriggerId === event.functionTriggerId,
                )?.id
              if (clearedId) {
                onPatchMessage(conversationId, clearedId, {
                  pendingApproval: false,
                  // Allowed calls execute now; their result pairs in from
                  // the transcript and flips running back off.
                  ...(event.running ? { running: true } : {}),
                })
              }
              break
            }
            case 'assistant-token': {
              if (!assistantId) {
                const msg: AssistantMessage = {
                  id: uid(),
                  role: 'assistant',
                  content: '',
                  model,
                  mode: conversation.mode,
                  streaming: true,
                  createdAt: Date.now(),
                }
                assistantId = msg.id
                assistantBuffer = ''
                onAppendMessage(conversationId, msg)
              }
              assistantBuffer += event.token
              onPatchMessage(conversationId, assistantId, {
                content: assistantBuffer,
              })
              break
            }
            case 'assistant-end': {
              if (assistantId) {
                onPatchMessage(conversationId, assistantId, {
                  streaming: false,
                })
              }
              setIsStreaming(false)
              setTurnPhase(null)
              assistantId = null
              assistantBuffer = ''
              break
            }
            case 'compaction': {
              const compactionContent =
                event.mode === 'sync'
                  ? `compacted ${event.tokensBefore.toLocaleString()} tokens before continuing`
                  : `compacted ${event.tokensBefore.toLocaleString()} tokens (background)`
              const marker: SystemMessage = {
                id: uid(),
                role: 'system',
                kind: 'compaction',
                content: compactionContent,
                tone: 'info',
                summaryText: event.summaryText,
                tokensBefore: event.tokensBefore,
                createdAt: Date.now(),
              }
              onAppendMessage(conversationId, marker)
              announcer.announce(compactionContent)
              break
            }
            case 'turn-status': {
              // `queued` renders in the queued-messages strip, not the waiting indicator.
              setTurnPhase(event.phase === 'queued' ? null : event.phase)
              break
            }
            case 'stop-reason': {
              let noticeContent = formatStopReason(event.reason, event.message)
              if (event.partialResultAvailable) {
                noticeContent +=
                  ' Partial output above was preserved and may be incomplete.'
              }
              const notice: SystemMessage = {
                id: event.entryId ?? uid(),
                role: 'system',
                kind: 'notice',
                content: noticeContent,
                tone: event.reason === 'error' ? 'error' : 'warn',
                // The transcript owns the authoritative lifecycle notice under
                // this id. Trigger delivery is unordered, so a late live
                // fallback may fill a gap but must not overwrite that record.
                provisional: true,
                createdAt: Date.now(),
              }
              onAppendMessage(conversationId, notice)
              if (event.reason === 'error') {
                announcer.announceAssertive(noticeContent)
              } else {
                announcer.announce(noticeContent)
              }
              break
            }
          }
          if (
            event.kind === 'fcall-start' ||
            event.kind === 'assistant-token' ||
            event.kind === 'thought-start'
          ) {
            setIsStreaming(true)
            // Content arrived — the pre-content phase line is over.
            setTurnPhase(null)
          }
        }
        // NOTE: an aborted generator also exits the loop without throwing;
        // the stop/rescue cases are filtered below via stopSeenRef and the
        // strip emptying when the session leaves `working`.
        streamEndedNormally = true
      } catch (err) {
        if (!isAbortError(err)) {
          console.warn('[chat] stream errored', err)
          // A dead send must never be silent: without this notice the user
          // sees only their message and an eternal waiting indicator.
          const detail = err instanceof Error ? err.message : String(err)
          const noticeContent = `send failed — ${detail}`
          const notice: SystemMessage = {
            id: uid(),
            role: 'system',
            kind: 'notice',
            content: noticeContent,
            tone: 'error',
            createdAt: Date.now(),
          }
          onAppendMessage(conversationId, notice)
          announcer.announceAssertive(noticeContent)
        }
      } finally {
        if (thoughtId) {
          onPatchMessage(conversationId, thoughtId, { streaming: false })
        }
        if (fcallId) {
          onPatchMessage(conversationId, fcallId, { running: false })
        }
        if (assistantId) {
          onPatchMessage(conversationId, assistantId, { streaming: false })
        }
        setIsStreaming(false)
        setTurnPhase(null)
        // Turn finished cleanly with messages still parked: the harness is
        // delivering them now — flip the strip's rows to `triggering…`.
        if (
          streamEndedNormally &&
          !stopSeenRef.current &&
          queuedStripCountRef.current > 0
        ) {
          setDrainingQueue(true)
        }
        abortRef.current = null
      }
    },
    [
      conversation.id,
      conversation.mode,
      conversation.model,
      conversation.skills,
      conversation.workingDir,
      effectiveModel,
      thinkingLevel,
      effectiveSystemPrompt,
      sessionId,
      contextWindow,
      backend,
      approvalEnabled,
      approvalSessionMatcher,
      announcer,
      ensureSession,
      isStreaming,
      serverWorking,
      onAppendMessage,
      onPatchMessage,
      onCompactConversation,
      reconcileWorkingDir,
      workingDirEnabled,
    ],
  )

  const handleStop = useCallback(() => {
    if (stopRequestedRef.current) return
    stopRequestedRef.current = true
    stopSeenRef.current = true
    setStopping(true)
    abortRef.current?.abort()
    // Re-enable the button if the stop RPC fails while the server still
    // reports working — otherwise `stopping` never clears (the reset
    // effect waits on the indicator) and the user can't retry.
    void backend.abortRun?.(sessionId).catch(() => {
      stopRequestedRef.current = false
      setStopping(false)
    })
  }, [backend, sessionId])

  // Rescue a parked stream loop: the session hit a terminal error server-side
  // (status-changed arrives on the session-directory subscription) but the
  // local `for await` is still waiting on a turn-completed that may never
  // come. Abort locally — the generator returns silently on abort, and the
  // red notice renders from the transcript's durable `error` entry.
  useEffect(() => {
    if (isStreaming && conversation.status === 'error') {
      abortRef.current?.abort()
    }
  }, [isStreaming, conversation.status])

  // Covers the gap between submit / fcall-end and the next streamed content,
  // where the assistant/thought output hasn't yet rendered.
  const isThinking =
    streamingIndicator &&
    (() => {
      const last =
        conversation.messages[conversation.messages.length - 1] ?? null
      if (!last) return true
      if (last.role === 'user') return true
      if (
        last.role === 'function-trigger' &&
        !last.running &&
        !last.pendingApproval
      ) {
        return true
      }
      return false
    })()

  // Pre-content phase text for the waiting indicator. Only trusted while the transcript
  // still ends at the user's message — on the real backend content arrives via
  // session events (not stream events), so once anything streamed the phase is
  // stale and mid-turn gaps fall back to the model line instead.
  const phaseDetail = (() => {
    if (!turnPhase) return null
    const last = conversation.messages[conversation.messages.length - 1] ?? null
    if (last && last.role !== 'user') return null
    switch (turnPhase) {
      case 'sending':
        return 'sending…'
      case 'accepted':
        return 'queued — waiting to start…'
      case 'merged':
        return 'added to the running turn…'
      case 'started':
        return null
    }
  })()

  /* Trace → message landing: a pending turn-focus for THIS session resolves
     to the transcript row to center (see lib/turn-anchor), recomputed as the
     transcript hydrates. Consumed when MessageList lands on it. A missing
     anchor drops the request only when nothing can still produce it — the
     transcript is hydrated AND no turn is running (a live turn writes its
     durable rows as it goes, so the click means "land there once it
     exists") — and only after a grace, because completion flips the status
     idle before the turn's last rows reach the transcript. So a stale
     request still can't fire on a later visit. */
  const chatFocusEvent = useChatMessageFocus()
  const chatFocus =
    chatFocusEvent && chatFocusEvent.sessionId === conversation.id
      ? chatFocusEvent
      : undefined
  const focusMessageId = useMemo(
    () =>
      chatFocus
        ? turnAnchorMessageId(conversation.messages, chatFocus.turnId)
        : null,
    [chatFocus, conversation.messages],
  )
  useEffect(() => {
    if (!chatFocus) return
    if (
      !shouldDropChatFocus({
        hydrated: conversation.hydrated,
        working: conversation.status === 'working',
        anchored: focusMessageId !== null,
      })
    ) {
      return
    }
    // Any dep change — anchor resolved, a turn (re)started, a new request —
    // cancels the pending drop; the id guard in clearChatMessageFocus keeps
    // a stale timer from ever dropping a newer request.
    const timer = window.setTimeout(
      () => clearChatMessageFocus(chatFocus.id),
      CHAT_FOCUS_DROP_GRACE_MS,
    )
    return () => window.clearTimeout(timer)
  }, [chatFocus, conversation.hydrated, conversation.status, focusMessageId])
  const chatFocusIdRef = useRef<number | null>(null)
  chatFocusIdRef.current = chatFocus?.id ?? null
  const handleFocusMessageHandled = useCallback(() => {
    if (chatFocusIdRef.current !== null) {
      clearChatMessageFocus(chatFocusIdRef.current)
    }
  }, [])

  const isDock = density === 'dock'
  const compact = isDock || onBack !== undefined
  const headerPad = compact ? 'px-3 sm:px-4' : 'px-3 sm:px-6 lg:px-9'
  const footerPad = compact
    ? 'px-3 pb-3 pt-2 sm:px-4 sm:pb-4'
    : 'px-3 pb-3 pt-2 sm:px-6 sm:pb-5 lg:px-9 lg:pb-6'

  // Resolve the working directory to its managed worktree so landed /
  // land-blocked events can be scoped to this conversation.
  const [worktreeRefresh, setWorktreeRefresh] = useState(0)
  const worktreeInfo = useWorktreeBinding(
    conversation.workingDir ?? null,
    worktreeEnabled && workingDirEnabled,
    worktreeRefresh,
  )
  const worktreeInfoRef = useRef<WorktreeInfo | null>(worktreeInfo)
  worktreeInfoRef.current = worktreeInfo

  // Only surface events for the worktree this conversation points at or
  // claimed through this console flow — never every land on the bus.
  const eventConcernsConversation = useCallback(
    (worktreeId: string) =>
      worktreeInfoRef.current?.worktree_id === worktreeId ||
      consoleClaimFor(conversation.id)?.worktreeId === worktreeId,
    [conversation.id],
  )

  const handleLanded = useCallback(
    (evt: WorktreeLandedEvent) => {
      if (!eventConcernsConversation(evt.worktree_id)) return
      const content = formatLandedNotice(evt)
      onAppendMessage(conversation.id, makeSystemNotice(content))
      announcer.announce(content)
      setWorktreeRefresh((t) => t + 1)
    },
    [conversation.id, eventConcernsConversation, onAppendMessage, announcer],
  )

  const handleLandBlocked = useCallback(
    (evt: WorktreeLandBlockedEvent) => {
      if (!eventConcernsConversation(evt.worktree_id)) return
      const content = formatLandBlockedNotice(evt)
      onAppendMessage(conversation.id, makeSystemNotice(content, 'warn'))
      announcer.announceAssertive(content)
      setWorktreeRefresh((t) => t + 1)
    },
    [conversation.id, eventConcernsConversation, onAppendMessage, announcer],
  )

  useWorktreeEvents({
    enabled: worktreeEnabled,
    onLanded: handleLanded,
    onLandBlocked: handleLandBlocked,
  })

  // Re-scope the working directory. Allowed mid-conversation (no irreversible
  // lock); a change after the chat has started drops a visible marker so the
  // directory the agent operates in is never silently swapped.
  const handleWorkingDirChange = useCallback(
    (next: string) => {
      const id = conversation.id
      const prev = conversation.workingDir ?? null
      workingDirRef.current = next
      onUpdateWorkingDir(id, next)
      // The editor follows the chat: picking a folder here repoints the
      // shared editor workspace so the editor page shows this project.
      void syncEditorWorkspace(next)
      if (!conversation.draft && next !== prev) {
        onAppendMessage(
          id,
          makeSystemNotice(
            `working directory changed to ${next} — applies to the messages that follow`,
          ),
        )
      }
    },
    [
      conversation.id,
      conversation.workingDir,
      conversation.draft,
      onUpdateWorkingDir,
      onAppendMessage,
    ],
  )

  useEffect(
    () =>
      onWorkingDirectoryChangeRequest(({ sessionId, path }) => {
        if (!workingDirEnabled || sessionId !== conversation.id) return false
        handleWorkingDirChange(path)
        return true
      }),
    [conversation.id, handleWorkingDirChange, workingDirEnabled],
  )

  // Picking a worktree claims it for this session; the working dir itself
  // arrives through the picker's shell-validated selection flow
  // (onWorkingDirChange with the worker-echoed canonical path), like every
  // other selection. Release any previous console claim first — the
  // registry holds one claim per session, so it must be released before it
  // is overwritten. The claim RPC is best-effort: a takeover failure (W210)
  // surfaces as a notice while the dir change still applies.
  const handlePickWorktree = useCallback(
    (wt: WorktreeInfo) => {
      const id = conversation.id
      void (async () => {
        // Strict release -> record -> claim: overwriting the local record
        // before the release settles would make the release read the NEW
        // claim (keepPath matches) and leak the previous one server-side.
        await releaseConsoleClaimIfAny(id, { keepPath: wt.path })
        recordConsoleClaim(id, { worktreeId: wt.worktree_id, path: wt.path })
        try {
          await claimWorktree(wt.worktree_id, id)
        } catch (err) {
          onAppendMessage(
            id,
            makeSystemNotice(
              `could not claim worktree ${wt.branch} — ${err instanceof Error ? err.message : String(err)}`,
              'warn',
            ),
          )
        }
        setWorktreeRefresh((t) => t + 1)
      })()
    },
    [conversation.id, onAppendMessage],
  )

  // Chat's keyboard, through the same contract a worker page uses, so the
  // palette lists these rows under "Chat" with their keys.
  const viewRef = useRef<HTMLElement>(null)
  const working = conversation.status === 'working'
  useEffect(() => {
    if (!commands) return
    const messageNodes = () =>
      Array.from(
        viewRef.current?.querySelectorAll<HTMLElement>('[data-message-row]') ??
          [],
      )
    const focusedRow = () =>
      messageNodes().find((node) => node.contains(document.activeElement))
    const actOnFocused = (action: string) => {
      const row = focusedRow()
      const button = row?.querySelector<HTMLButtonElement>(
        `[data-message-action="${action}"]`,
      )
      if (button && !button.disabled) button.click()
    }
    const pendingApproval = () => {
      const view = viewRef.current
      return (
        view !== null && view.querySelector('[data-approval-actions]') !== null
      )
    }
    const answerApproval = (action: 'approve' | 'deny' | 'always-allow') => {
      const row = focusedRow()
      const waiting = Array.from(
        viewRef.current?.querySelectorAll('[data-approval-actions]') ?? [],
      )
      // The focused row if it is waiting; else the only waiting call. Two
      // waiting calls and no focus is a choice the keyboard must not make.
      const scope = row?.querySelector('[data-approval-actions]')
        ? row
        : waiting.length === 1
          ? waiting[0]
          : null
      scope
        ?.querySelector<HTMLButtonElement>(`[data-message-action="${action}"]`)
        ?.click()
    }
    const focusMessage = (delta: 1 | -1) => {
      const nodes = messageNodes()
      if (nodes.length === 0) return
      const current = nodes.findIndex((node) =>
        node.contains(document.activeElement),
      )
      const start = delta === 1 ? 0 : nodes.length - 1
      const index =
        current === -1
          ? start
          : Math.min(nodes.length - 1, Math.max(0, current + delta))
      const node = nodes[index]
      node.tabIndex = -1
      node.focus({ preventScroll: true })
      node.scrollIntoView({ block: 'nearest' })
    }
    return commands.register([
      {
        id: 'focus-composer',
        title: 'Focus the composer',
        detail: 'Put the caret in the message box',
        keywords: ['type', 'write', 'input', 'message'],
        shortcut: 'I',
        run: () => requestComposerFocus(),
      },
      {
        id: 'next-message',
        title: 'Next message',
        detail: 'Move the focus down the conversation',
        keywords: ['down', 'read', 'inspect'],
        shortcut: 'J',
        run: () => focusMessage(1),
      },
      {
        id: 'previous-message',
        title: 'Previous message',
        detail: 'Move the focus up the conversation',
        keywords: ['up', 'read', 'inspect'],
        shortcut: 'K',
        run: () => focusMessage(-1),
      },
      {
        id: 'latest',
        title: 'Jump to the latest message',
        detail: 'Scroll to the end of the conversation',
        keywords: ['bottom', 'end', 'scroll', 'tail'],
        shortcut: 'End',
        run: () => {
          const list = viewRef.current?.querySelector<HTMLElement>(
            '[data-message-list]',
          )
          list?.scrollTo({ top: list.scrollHeight })
        },
      },
      {
        id: 'approve',
        title: 'Approve the pending call',
        detail: 'Let the focused (or the only waiting) function call run',
        keywords: ['allow', 'yes', 'permission'],
        shortcut: 'A',
        enabled: pendingApproval,
        run: () => answerApproval('approve'),
      },
      {
        id: 'deny',
        title: 'Deny the pending call',
        detail: 'Refuse the focused (or the only waiting) function call',
        keywords: ['reject', 'no', 'permission'],
        shortcut: 'D',
        enabled: pendingApproval,
        run: () => answerApproval('deny'),
      },
      {
        id: 'always-allow',
        title: 'Always allow the pending call',
        detail: 'Approve it and stop asking for this function',
        keywords: ['allow', 'permission', 'session', 'trust'],
        shortcut: 'S',
        enabled: pendingApproval,
        run: () => answerApproval('always-allow'),
      },
      {
        id: 'expand',
        title: 'Expand or collapse the focused message',
        detail: 'Open a function call card, or fold it',
        keywords: ['open', 'fold', 'details'],
        shortcut: 'O',
        run: () => actOnFocused('toggle'),
      },
      {
        id: 'copy',
        title: 'Copy the focused message',
        detail: 'Copy its text to the clipboard',
        keywords: ['clipboard'],
        shortcut: 'Y',
        run: () => actOnFocused('copy'),
      },
      {
        id: 'model',
        title: 'Switch model',
        detail: 'Open the model picker',
        keywords: ['provider', 'picker', 'llm'],
        shortcut: 'M',
        run: handleOpenModelPicker,
      },
      {
        id: 'stop',
        title: 'Stop the turn',
        detail: 'Interrupt the running generation',
        keywords: ['cancel', 'interrupt', 'abort'],
        shortcut: 'Escape',
        firesWhileTyping: true,
        enabled: () => working || streamingIndicator,
        run: handleStop,
      },
    ])
  }, [commands, handleOpenModelPicker, handleStop, working, streamingIndicator])

  return (
    <section
      ref={viewRef}
      data-chat-session-id={conversation.id}
      data-chat-session-hydrated={conversation.hydrated}
      className="flex-1 flex flex-col min-w-0 min-h-0"
    >
      <PageHeader
        className={headerPad}
        onClose={onRequestClose}
        actions={
          <div className="flex items-center gap-1.5 font-sans text-sm">
            {/* Header read-outs share ONE surface. This system draws no
                lines (index.css:44-52 — rule/rule-2 are transparent in both
                themes), so a group is a fill, not a run of dividers. It also
                keeps the related session metadata visually together. */}
            <div className="flex h-7 items-center gap-3 rounded-md bg-surface px-2.5 max-lg:hidden">
              {sessionChips}
              {hasInjectedContextChip ? null : (
                <ContextUsage
                  messages={conversation.messages}
                  contextWindow={contextWindow}
                />
              )}
            </div>
            {/* Status sits OUTSIDE the group — it is state, not a control.
                The dot alone carries it (green ready, pulsing accent
                working, red error); the word lives in the tooltip and in an
                sr-only role="status" span so transitions still announce.
                `self-stretch px-1` turns a 6px dot into a full-height hover
                target without letting an error widen the header. */}
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex size-12 items-center justify-center max-sm:hidden sm:size-10 lg:self-stretch lg:size-auto lg:px-1">
                  <StatusDot
                    tone={
                      conversation.status === 'error'
                        ? 'alert'
                        : streamingIndicator
                          ? 'accent'
                          : 'ok'
                    }
                    pulse={streamingIndicator}
                  />
                  <span role="status" className="sr-only">
                    {streamingIndicator
                      ? 'working'
                      : conversation.status === 'error'
                        ? 'error'
                        : 'ready'}
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>
                {conversation.status === 'error'
                  ? `error${conversation.statusReason ? ` — ${conversation.statusReason}` : ''}`
                  : streamingIndicator
                    ? 'working'
                    : 'ready'}
              </TooltipContent>
            </Tooltip>
          </div>
        }
      >
        <div className="flex min-w-0 flex-1 items-center gap-2 font-sans text-sm text-ink-faint">
          {onBack ? (
            <button
              type="button"
              onClick={onBack}
              aria-label="back to conversations"
              title="back to conversations"
              className="relative -ml-1 flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:size-7"
            >
              <span
                className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <ArrowLeft aria-hidden className="size-4 shrink-0" />
            </button>
          ) : null}
          <span className="min-w-0 truncate font-medium text-ink">
            {panelTitle ?? 'Chat'}
          </span>
          <span className="shrink-0 text-ink-ghost">·</span>
          <span className="min-w-0 truncate">{effectiveModel}</span>
        </div>
      </PageHeader>

      {approvalSettings.settings.mode === 'full' ? (
        <FullPermissionsBanner
          onDisable={() => void approvalSettings.setMode('manual')}
        />
      ) : null}

      <MessageList
        messages={conversation.messages}
        spawnContext={{
          title: conversation.title,
          model: effectiveModel,
          appearance: conversation.subagentAppearance,
        }}
        transcriptHydrated={conversation.hydrated !== false}
        isThinking={isThinking}
        thinkingDetail={
          conversation.status === 'working' && conversation.statusReason
            ? conversation.statusReason
            : (phaseDetail ??
              (effectiveModel ? `dispatching ${effectiveModel}` : undefined))
        }
        density={density}
        onResolveApproval={resolveApproval}
        onAlwaysAllow={handleAlwaysAllow}
        onResolveFilesystemAccess={handleFilesystemResolve}
        onManageFilesystemAccess={handleManageFilesystemAccess}
        onConfigureProvider={handleOpenModelPicker}
        workingDir={conversation.workingDir ?? null}
        onWorkingDirChange={
          workingDirEnabled ? handleWorkingDirChange : undefined
        }
        defaultWorkingDir={defaultWorkingDir}
        worktreePicker={
          worktreeEnabled
            ? { enabled: true, onPick: handlePickWorktree }
            : undefined
        }
        triggersById={triggersById}
        focusMessageId={focusMessageId}
        onFocusMessageHandled={handleFocusMessageHandled}
      />
      <LiveRegion announcement={announcer.announcement} />

      <footer className={footerPad}>
        <div className="mx-auto max-w-[760px]">
          {conversationsCtx ? (
            <ActiveSubagentChips
              className="mb-1 px-1"
              conversations={conversationsCtx.conversations}
              rootSessionId={conversation.id}
              connectionState={conversationsCtx.connectionState}
              onOpen={conversationsCtx.openConversationInPanel}
            />
          ) : null}
          <SessionTriggers
            triggers={mergedTriggers}
            onUnregister={handleUnregisterTrigger}
            onClearAll={
              backend.unregisterTrigger ? handleClearAllTriggers : undefined
            }
            checkStateKey={backend.stateKeyExists}
          />
          {queuedStrip.length > 0 ? (
            <section
              className="mb-1 rounded-md bg-surface"
              aria-label="queued messages"
            >
              {/* The message being edited is pulled out of the queue and lives
                  only in the composer — hidden here until it's saved back (in
                  place, so it reappears at its spot) or removed. */}
              {queuedStrip
                .filter((row) => row.id !== browsedQueuedId)
                .map((row) => (
                  <div
                    key={row.id}
                    className="flex items-center gap-2 border-b border-rule-2 px-3 py-2.5 text-base last:border-b-0 sm:py-1.5 sm:text-[12px]"
                  >
                    <span className="min-w-0 flex-1 truncate">{row.text}</span>
                    <span className="shrink-0 text-ink-ghost">
                      {drainingQueue ? 'Triggering…' : 'Queued'}
                    </span>
                  </div>
                ))}
              {/* No edit hint while draining — the rows are being delivered,
                  so inviting an edit would be a race the user loses. */}
              {backend.editQueued &&
              queuedDrafts.length > 0 &&
              !drainingQueue ? (
                <div className="px-3 py-0.5 text-right text-[11px] text-ink-ghost max-sm:hidden">
                  {browsedQueuedId
                    ? '↑ / ↓ cycle · Enter saves in place · Empty + Enter removes'
                    : 'Press ↑ in the composer to edit queued messages'}
                </div>
              ) : null}
            </section>
          ) : null}
          {sessionTurnSummaries ? (
            <div
              className="flex flex-wrap items-center justify-end gap-1.5 px-1"
              data-chat-turn-summary-slot
            >
              {sessionTurnSummaries}
            </div>
          ) : null}
          <Composer
            mode={conversation.mode}
            model={effectiveModel}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            modelPickerOpenRequest={modelPickerOpenRequest}
            functionEntries={functionEntries}
            permissionMode={approvalSettings.settings.mode}
            permissionModeLoading={!approvalSettings.loaded}
            showPermissionMode={approvalEnabled}
            thinkingLevel={thinkingLevel}
            onThinkingLevelChange={handleThinkingLevelChange}
            onModeChange={(next) => onUpdateMode(conversation.id, next)}
            onModelChange={(next) => onUpdateModel(conversation.id, next)}
            showWorkingDir={workingDirEnabled}
            workingDir={conversation.workingDir ?? null}
            showMemoryBank={
              backend.id === 'real' &&
              (conversationsCtx?.memoryAvailable ?? false)
            }
            memoryBank={conversation.memoryBank ?? null}
            onMemoryBankChange={(next) =>
              conversationsCtx?.setMemoryBank(conversation.id, next)
            }
            workingDirLocked={false}
            defaultWorkingDir={defaultWorkingDir}
            onWorkingDirChange={handleWorkingDirChange}
            worktreePicker={
              worktreeEnabled
                ? { enabled: true, onPick: handlePickWorktree }
                : undefined
            }
            onPermissionModeChange={(next) =>
              void approvalSettings.setMode(next)
            }
            initialText={composerInitialText}
            onTextChange={handleComposerTextChange}
            onSubmit={handleSubmit}
            onStop={handleStop}
            stopping={stopping}
            queuedForEdit={backend.editQueued ? queuedForEdit : undefined}
            onEditQueued={backend.editQueued ? handleEditQueued : undefined}
            onBrowseChange={setBrowsedQueuedId}
            isStreaming={streamingIndicator}
            queueWhileStreaming={!!backend.queueMessage}
            blocked={harnessBlocked}
            submitBlocked={submitBlocked}
            autoFocus={focusComposerOnOpen && !harnessBlocked}
            blockedPlaceholder={
              conversationsCtx
                ? harnessComposerPlaceholder(conversationsCtx.harnessStatus)
                : undefined
            }
          />
        </div>
      </footer>

      {workingDirEnabled ? (
        <FilesystemAccessDialog
          open={filesystemDialogOpen}
          onOpenChange={setFilesystemDialogOpen}
          workingDir={conversation.workingDir ?? null}
          grants={filesystemGrants.grants}
          grantsSupported={filesystemGrants.supported}
          onRevoke={filesystemGrants.revoke}
          onRefreshGrants={filesystemGrants.refresh}
          sessionBusy={streamingIndicator}
          workspaceScoped={approvalEnabled}
        />
      ) : null}
    </section>
  )
}
