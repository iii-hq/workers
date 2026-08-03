import { Copy, Folder } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FilesystemAccessDialog } from '@/components/permissions/FilesystemAccessDialog'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { FullPermissionsBanner } from '@/components/permissions/FullPermissionsBanner'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { StatusDot } from '@/components/ui/StatusDot'
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
import { useWorktreeBinding } from '@/hooks/use-worktree-binding'
import { useWorktreeEvents } from '@/hooks/use-worktree-events'
import type { ChatBackend } from '@/lib/backend'
import { approvalBelongsToConversationTree } from '@/lib/backend/approval-events-live'
import { predictedUserEntryId } from '@/lib/backend/harness-send'
import {
  mergeFiredTriggers,
  type SessionTriggerInfo,
} from '@/lib/backend/triggers'
import type {
  ApprovalStreamEvent,
  CompactResult,
  QueuedMessagePreview,
} from '@/lib/backend/types'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { syncEditorWorkspace } from '@/lib/editor-sync'
import { expandFileMentions, parseFileMentions } from '@/lib/file-mentions'
import { formatStopReason } from '@/lib/format-stop-reason'
import {
  expandPdfAttachments,
  isPdfAttachment,
  summaryLabel,
} from '@/lib/pdf-attachments'
import { newMessageId } from '@/lib/session-id'
import { cn } from '@/lib/utils'
import { fetchDefaultWorkingDir, validateWorkspaceDir } from '@/lib/working-dir'
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
import { Composer, type ComposerSubmitPayload } from './Composer'
import { ContextUsage } from './ContextUsage'
import { ExportSessionButton } from './ExportSessionButton'
import { MessageList } from './MessageList'
import { SessionTriggers } from './SessionTriggers'
import { WorktreeBadge } from './WorktreeBadge'

/**
 * Saved dirs already re-validated this page load, keyed sessionId + dir —
 * one live check per activation, not one per render or per send.
 */
const validatedWorkingDirs = new Set<string>()

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
  onUpdateModel: (id: string, model: ModelId) => void
  onUpdateMode: (id: string, mode: Mode) => void
  onUpdateWorkingDir: (id: string, dir: string) => void
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
  onUpdateModel,
  onUpdateMode,
  onUpdateWorkingDir,
  onAppendMessage,
  onPatchMessage,
  onCompactConversation,
}: ChatViewProps) {
  const [isStreaming, setIsStreaming] = useState(false)
  // Pre-content phase of this tab's in-flight send, for the thinking
  // shimmer's detail line: submit → harness::send ack → turn-started.
  // Null once content streams (or for turns this tab didn't start).
  const [turnPhase, setTurnPhase] = useState<
    'sending' | 'accepted' | 'merged' | 'started' | null
  >(null)
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>(
    DEFAULT_THINKING_LEVEL,
  )
  const abortRef = useRef<AbortController | null>(null)
  const [copied, setCopied] = useState(false)
  const { functionEntries } = useFunctionsCatalog(backend.id)
  const conversationsCtx = useConversationsCtxOptional()
  const harnessBlocked = conversationsCtx
    ? isChatBlockedByHarness(conversationsCtx.harnessStatus)
    : false
  const harnessBlockedRef = useRef(harnessBlocked)
  harnessBlockedRef.current = harnessBlocked

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

  // Registered trigger subscriptions (notify/react bindings owned by this
  // session): shown above the composer, unregisterable, detail on click.
  // Polled — bindings come and go as the agent registers them mid-turn.
  const [sessionTriggers, setSessionTriggers] = useState<SessionTriggerInfo[]>(
    [],
  )
  // Every full row this tab has EVER polled, by engine trigger id. When a
  // once/join binding fires and retires, the poll drops it — this cache lets
  // the fired ghost keep its full metadata (join grouping, spawn pin, task)
  // so the workflow strip and flow DAG survive the pipeline completing.
  const seenTriggersRef = useRef<Map<string, SessionTriggerInfo>>(new Map())
  const refreshTriggers = useCallback(() => {
    const listTriggers = backend.listTriggers
    if (!listTriggers) return
    listTriggers(conversation.id)
      .then((rows) => {
        for (const row of rows) seenTriggersRef.current.set(row.id, row)
        setSessionTriggers(rows)
      })
      .catch(() => {})
  }, [backend.listTriggers, conversation.id])
  useEffect(() => {
    if (!backend.listTriggers) return
    seenTriggersRef.current = new Map()
    setSessionTriggers([])
    refreshTriggers()
    const timer = window.setInterval(refreshTriggers, 5000)
    return () => window.clearInterval(timer)
  }, [refreshTriggers, backend.listTriggers])

  const handleUnregisterTrigger = useCallback(
    async (triggerId: string) => {
      try {
        await backend.unregisterTrigger?.(triggerId)
        setSessionTriggers((rows) => rows.filter((t) => t.id !== triggerId))
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
    const results = await Promise.allSettled(ids.map((id) => unreg(id)))
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
  // once-trigger stays visible after the engine drops it from the poll.
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
        // Same expansion as the live send path: a queued message's PDFs have
        // to reach the agent as markdown too, or editing a queued message
        // would silently drop the document it carried.
        if (
          backend.id === 'real' &&
          payload.attachments.some(isPdfAttachment)
        ) {
          const expanded = await expandPdfAttachments(payload.attachments)
          if (expanded.blocks.length > 0) {
            attachedBlocks = [...(attachedBlocks ?? []), ...expanded.blocks]
          }
        }
        try {
          await backend.editQueued?.(
            conversationId,
            id,
            payload.text,
            attachedBlocks ? { attachedBlocks } : undefined,
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

  /* Shared live region: SR announcements for auto-accept, stop-reason
   * notices, and compaction markers route through this hook. Sighted
   * users see the same messages in the transcript; visually-impaired
   * users hear them via the polite/assertive ARIA live regions
   * rendered at the bottom of the component. */
  const announcer = useLiveAnnouncer()

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
      if (existing) {
        onPatchMessage(conversation.id, existing.id, {
          pendingApproval: false,
          ...(event.running ? { running: true } : {}),
        })
      }
    },
    [conversation.id, onAppendMessage, onPatchMessage],
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

  // The working-directory picker + banner only make sense with the `shell`
  // worker connected: the picker browses via shell-served `coder::*` functions
  // and the chosen dir scopes shell exec/file calls. Hide both when shell is
  // absent so we never render controls that call functions that don't exist.
  const workingDirEnabled =
    backend.id === 'real' &&
    (conversationsCtx ? conversationsCtx.shellAvailable : false)

  // The stack's default folder, resolved once (cached page-wide): pre-fills
  // fresh drafts below and feeds the picker's pinned "default" row so the
  // launch folder stays selectable after a chat re-scopes elsewhere.
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

  // A saved working dir can be gone by the next visit (deleted, unmounted,
  // denylisted). Re-validate once per activation against the live shell; on
  // failure surface the error through the picker (auto-opens) instead of
  // letting the chat silently operate against a dead folder.
  const [workingDirError, setWorkingDirError] = useState<string | null>(null)
  useEffect(() => {
    if (!workingDirEnabled || conversation.draft) return
    const dir = conversation.workingDir
    if (!dir) return
    // Switching to a conversation brings its working directory with it: the
    // editor page follows the active chat, not the last folder ever opened.
    // Best-effort with its own consecutive-root dedupe, so re-activations
    // and already-validated dirs stay cheap.
    void syncEditorWorkspace(dir)
    const key = `${conversation.id}:${dir}`
    if (validatedWorkingDirs.has(key)) return
    let cancelled = false
    void validateWorkspaceDir(dir).then((res) => {
      if (cancelled) return
      if (res.ok) {
        validatedWorkingDirs.add(key)
        setWorkingDirError(null)
      } else {
        setWorkingDirError(
          `working directory ${dir} is not usable — ${res.error}. choose another folder.`,
        )
      }
    })
    return () => {
      cancelled = true
    }
  }, [
    workingDirEnabled,
    conversation.draft,
    conversation.workingDir,
    conversation.id,
  ])

  // The worktrees tab, the working-directory badge, claim/release, and the
  // landed / land-blocked live events all require the optional `worktree`
  // worker; gate the whole surface on its presence like shell above.
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

  const handleCopySessionId = useCallback(() => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return
    void navigator.clipboard.writeText(sessionId).then(() => {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }, [sessionId])

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
      if (harnessBlockedRef.current) return
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
      const workingDir = conversation.workingDir
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

      // A PDF is not text: read as bytes it reaches the model as noise, so the
      // `pdf` worker converts it on this machine and the markdown is appended
      // as another attachment block. Failures never block the send — an
      // unreadable document becomes a placeholder block plus a warn notice, so
      // the model knows it was handed something it could not read.
      if (backend.id === 'real' && payload.attachments.some(isPdfAttachment)) {
        const expanded = await expandPdfAttachments(payload.attachments)
        if (expanded.blocks.length > 0) {
          attachedBlocks = [...(attachedBlocks ?? []), ...expanded.blocks]
        }
        // Relabel the chip with what the worker made of the document. The
        // expansion runs before the model is called, so it never shows up as a
        // function call — without this a person has no way to tell the PDF was
        // read at all.
        if (expanded.read.length > 0 && !willQueue) {
          const byId = new Map(expanded.read.map((r) => [r.id, r]))
          onPatchMessage(conversationId, userMsg.id, {
            attachments: (userMsg.attachments ?? []).map((a) => {
              const summary = byId.get(a.id)
              return summary ? { ...a, name: summaryLabel(a.name, summary) } : a
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
              workingDir: conversation.workingDir,
              approvalGateAvailable: approvalEnabled,
              ...(attachedBlocks && attachedBlocks.length > 0
                ? { attachedBlocks }
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
            workingDir: conversation.workingDir,
            approvalGateAvailable: approvalEnabled,
            approvalSessionMatcher,
            approvalEventsExternallyManaged: true,
            ...(attachedBlocks && attachedBlocks.length > 0
              ? { attachedBlocks }
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
              // `queued` renders in the queued-messages strip, not the shimmer.
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
          // sees only their message and an eternal shimmer.
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
      conversation.workingDir,
      effectiveModel,
      thinkingLevel,
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
  // where the assistant/thought shimmer hasn't yet rendered.
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

  // Pre-content phase text for the shimmer. Only trusted while the transcript
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

  const isDock = density === 'dock'
  const headerPad = isDock ? 'px-4' : 'px-9'
  const footerPad = isDock ? 'px-4 pb-4 pt-2' : 'px-9 pb-6 pt-2'

  // Resolve the working directory to its managed worktree (badge), and keep
  // it fresh across landed / land-blocked events.
  const [worktreeRefresh, setWorktreeRefresh] = useState(0)
  const worktreeInfo = useWorktreeBinding(
    conversation.workingDir ?? null,
    worktreeEnabled && workingDirEnabled,
    worktreeRefresh,
  )
  const worktreeInfoRef = useRef<WorktreeInfo | null>(worktreeInfo)
  worktreeInfoRef.current = worktreeInfo

  // Only surface events for the worktree this conversation points at (badge)
  // or claimed through this console flow — never every land on the bus.
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
      // Every picker selection is freshly shell-validated; replacing a stale
      // dir resolves the invalid-folder state.
      setWorkingDirError(null)
      validatedWorkingDirs.add(`${id}:${next}`)
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

  return (
    <section
      data-chat-session-id={conversation.id}
      data-chat-session-hydrated={conversation.hydrated}
      className="flex-1 flex flex-col min-w-0 min-h-0"
    >
      <header
        className={cn(
          'flex items-center justify-between py-3 border-b border-rule gap-3 whitespace-nowrap',
          headerPad,
        )}
      >
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint flex items-center gap-2 min-w-0 flex-1">
          <span className="text-accent flex-shrink-0" aria-hidden>
            $
          </span>
          <span className="text-ink truncate min-w-0">{effectiveModel}</span>
          <span className="text-ink-ghost flex-shrink-0">·</span>
          <span className="text-ink-faint flex-shrink-0">
            {conversation.mode}
          </span>
          {isDock ? null : (
            <>
              <span className="text-ink-ghost flex-shrink-0">·</span>
              <button
                type="button"
                onClick={handleCopySessionId}
                title={
                  copied
                    ? `copied ${sessionId}`
                    : `${sessionId} — click to copy. matches iii.session.id in the traces tab.`
                }
                className="flex items-center gap-1 text-ink-faint hover:text-ink transition-colors group normal-case tracking-normal min-w-0"
              >
                <span className="truncate font-mono text-[11px] tabular-nums">
                  {sessionId}
                </span>
                {copied ? (
                  <span className="text-accent text-[10px] uppercase tracking-[0.06em] flex-shrink-0">
                    copied
                  </span>
                ) : (
                  <Copy className="size-3 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" />
                )}
              </button>
            </>
          )}
        </div>
        <div className="flex items-center gap-3 font-mono text-[11px] uppercase tracking-[0.06em] flex-shrink-0">
          <ContextUsage
            messages={conversation.messages}
            contextWindow={contextWindow}
          />
          <ExportSessionButton
            conversation={conversation}
            onExported={(filename) =>
              announcer.announce(`session exported as ${filename}`)
            }
          />
          <div className="flex items-center gap-2">
            <StatusDot
              tone={
                conversation.status === 'error'
                  ? 'alert'
                  : streamingIndicator
                    ? 'accent'
                    : 'ink'
              }
              pulse={streamingIndicator}
            />
            <span
              className="text-ink-faint"
              title={
                conversation.status === 'error'
                  ? conversation.statusReason
                  : undefined
              }
            >
              {streamingIndicator
                ? 'working'
                : conversation.status === 'error'
                  ? 'error'
                  : 'ready'}
            </span>
          </div>
        </div>
      </header>

      {approvalSettings.settings.mode === 'full' ? (
        <FullPermissionsBanner
          onDisable={() => void approvalSettings.setMode('manual')}
        />
      ) : null}

      <MessageList
        messages={conversation.messages}
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
        workingDir={conversation.workingDir ?? null}
      />
      <LiveRegion announcement={announcer.announcement} />

      <footer className={footerPad}>
        <div className="mx-auto max-w-[760px]">
          {workingDirEnabled ? (
            <div className="mb-1 flex items-center gap-1.5 px-1 text-[11px] text-ink-faint">
              {worktreeInfo ? (
                // Managed worktree: branch + id + dirty/ahead + lifecycle
                // replace the raw path chip (path stays as the tooltip).
                <WorktreeBadge worktree={worktreeInfo} className="min-w-0" />
              ) : (
                <>
                  <Folder size={12} className="shrink-0" aria-hidden />
                  {conversation.workingDir ? (
                    <span
                      // dir=rtl keeps the trailing (most-distinguishing) path
                      // segment visible when truncated in the narrow dock.
                      dir="rtl"
                      className={cn(
                        'truncate text-left font-mono',
                        workingDirError && 'text-warn',
                      )}
                      title={workingDirError ?? conversation.workingDir}
                    >
                      {conversation.workingDir}
                    </span>
                  ) : (
                    <span className="lowercase text-ink-ghost">
                      no working directory — using default workspace
                    </span>
                  )}
                </>
              )}
              <button
                type="button"
                onClick={handleManageFilesystemAccess}
                title={
                  approvalEnabled
                    ? 'Access is limited to this workspace until you approve another folder.'
                    : 'The working directory sets where commands start; shell configuration controls access.'
                }
                className="ml-auto shrink-0 lowercase text-ink-ghost hover:text-ink transition-colors"
              >
                access: {approvalEnabled ? 'workspace' : 'shell defaults'}
                {approvalEnabled && filesystemGrants.grants.length > 0
                  ? ` · ${filesystemGrants.grants.length}`
                  : ''}
              </button>
            </div>
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
              className="mb-1 border border-rule bg-bg"
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
                    className="flex items-center gap-2 border-b border-rule-2 px-3 py-1.5 text-[12px] last:border-b-0"
                  >
                    <span className="min-w-0 flex-1 truncate">{row.text}</span>
                    <span className="shrink-0 lowercase text-ink-ghost">
                      {drainingQueue ? 'triggering…' : 'queued'}
                    </span>
                  </div>
                ))}
              {/* No edit hint while draining — the rows are being delivered,
                  so inviting an edit would be a race the user loses. */}
              {backend.editQueued &&
              queuedDrafts.length > 0 &&
              !drainingQueue ? (
                <div className="px-3 py-0.5 text-right text-[10px] lowercase text-ink-ghost">
                  {browsedQueuedId
                    ? '↑ / ↓ cycle · enter saves in place · empty + enter removes'
                    : 'press ↑ in the composer to edit queued messages'}
                </div>
              ) : null}
            </section>
          ) : null}
          <Composer
            mode={conversation.mode}
            model={effectiveModel}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            functionEntries={functionEntries}
            permissionMode={approvalSettings.settings.mode}
            permissionModeLoading={!approvalSettings.loaded}
            showPermissionMode={approvalEnabled}
            thinkingLevel={thinkingLevel}
            onThinkingLevelChange={setThinkingLevel}
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
            workingDirError={workingDirError}
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
