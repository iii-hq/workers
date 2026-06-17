import { Copy } from 'lucide-react'
import { useCallback, useMemo, useRef, useState } from 'react'
import { FullPermissionsBanner } from '@/components/permissions/FullPermissionsBanner'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { StatusDot } from '@/components/ui/StatusDot'
import { useApprovalSettings } from '@/hooks/use-approval-settings'
import { uid } from '@/hooks/use-conversations'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import {
  harnessComposerPlaceholder,
  isChatBlockedByHarness,
  isHarnessAvailable,
} from '@/hooks/use-harness-status'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import type { ChatBackend } from '@/lib/backend'
import { predictedUserEntryId } from '@/lib/backend/harness-send'
import type { CompactResult } from '@/lib/backend/types'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { formatStopReason } from '@/lib/format-stop-reason'
import { newMessageId } from '@/lib/session-id'
import { cn } from '@/lib/utils'
import {
  type AssistantMessage,
  type Conversation,
  DEFAULT_THINKING_LEVEL,
  type FunctionCallMessage,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
  type ModelOption,
  type SystemMessage,
  type ThinkingLevel,
  type ThoughtMessage,
  type UserMessage,
} from '@/types/chat'
import { Composer, type ComposerSubmitPayload } from './Composer'
import { ContextUsage } from './ContextUsage'
import { ExportSessionButton } from './ExportSessionButton'
import { MessageList } from './MessageList'

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
  onAppendMessage,
  onPatchMessage,
  onCompactConversation,
}: ChatViewProps) {
  const [isStreaming, setIsStreaming] = useState(false)
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
  // a turn is in flight, and dedupe-by-functionCallId must see them.
  const messagesRef = useRef(conversation.messages)
  messagesRef.current = conversation.messages

  // Driver-owned session status from session-manager (status-changed events);
  // covers reloads and turns observed from another tab. Local isStreaming
  // covers the gap before the first status event lands.
  const serverWorking = conversation.status === 'working'
  const streamingIndicator = isStreaming || serverWorking

  // The conversation id IS the engine session_id (console-<uuid> for chats
  // created here). Matches iii.session.id on every span so the traces UI can
  // group by it.
  const sessionId = conversation.id

  const contextWindow = useMemo(() => {
    const match = modelOptions.find((o) => o.id === conversation.model)
    return match?.contextWindow
  }, [modelOptions, conversation.model])

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
      functionCallId: string,
      decision: 'allow' | 'deny',
    ) => fn(sessionId, functionCallId, decision)
  }, [backend])

  const approvalEnabled =
    backend.id === 'real' &&
    (conversationsCtx
      ? isHarnessAvailable(conversationsCtx.harnessStatus)
      : false)
  const approvalSettings = useApprovalSettings(sessionId, approvalEnabled)

  const handleAlwaysAllow = useMemo(() => {
    const resolveFn = backend.resolveApproval
    if (!resolveFn) return undefined
    return async (sId: string, functionCallId: string, functionId: string) => {
      await approvalSettings.approveAlways(functionId)
      await resolveFn(sId, functionCallId, 'allow')
      announcer.announce(`approved always this session: ${functionId}`)
    }
  }, [backend, approvalSettings, announcer])

  const handleCopySessionId = useCallback(() => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return
    void navigator.clipboard.writeText(sessionId).then(() => {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }, [sessionId])

  const ensureSession = conversationsCtx?.ensureSession

  const handleSubmit = useCallback(
    async (payload: ComposerSubmitPayload) => {
      if (harnessBlockedRef.current) return
      const conversationId = conversation.id
      const model = conversation.model
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
      onAppendMessage(conversationId, userMsg)

      const trimmed = payload.text.trim()
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

      const controller = new AbortController()
      abortRef.current = controller
      setIsStreaming(true)

      let thoughtId: string | null = null
      let thoughtBuffer = ''
      let fcallId: string | null = null
      const fcallMap = new Map<string, string>()
      let assistantId: string | null = null
      let assistantBuffer = ''

      try {
        for await (const event of backend.stream(
          payload.text || '(attachments only)',
          conversation.mode,
          model,
          { signal: controller.signal, sessionId, messageId, thinkingLevel },
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
              if (event.functionCallId) {
                // Dedupe against rows created earlier in this stream AND
                // rows the session-events reconciler derived from the
                // assistant entry's function_call blocks.
                const existing =
                  [...fcallMap.entries()].find(
                    ([, fcid]) => fcid === event.functionCallId,
                  )?.[0] ??
                  messagesRef.current.find(
                    (m) =>
                      m.role === 'function-call' &&
                      m.functionCallId === event.functionCallId,
                  )?.id
                if (existing) {
                  onPatchMessage(conversationId, existing, {
                    pendingApproval: event.pendingApproval,
                    running: !event.pendingApproval,
                    functionCallId: event.functionCallId,
                    sessionId: event.sessionId,
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
              const msg: FunctionCallMessage = {
                id: uid(),
                role: 'function-call',
                functionId: event.functionId,
                input: event.input,
                running: !event.pendingApproval,
                pendingApproval: event.pendingApproval,
                functionCallId: event.functionCallId,
                sessionId: event.sessionId,
                createdAt: Date.now(),
              }
              fcallId = msg.id
              if (event.functionCallId)
                fcallMap.set(msg.id, event.functionCallId)
              onAppendMessage(conversationId, msg)
              break
            }
            case 'fcall-end': {
              // Prefer targeting by functionCallId (parallel batches end out
              // of order; the row may also be an entry-derived segment).
              const targetId: string | null = event.functionCallId
                ? ([...fcallMap.entries()].find(
                    ([, fcid]) => fcid === event.functionCallId,
                  )?.[0] ??
                  messagesRef.current.find(
                    (m) =>
                      m.role === 'function-call' &&
                      m.functionCallId === event.functionCallId,
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
                  ([, fcid]) => fcid === event.functionCallId,
                )?.[0] ??
                messagesRef.current.find(
                  (m) =>
                    m.role === 'function-call' &&
                    m.functionCallId === event.functionCallId,
                )?.id
              if (clearedId) {
                onPatchMessage(conversationId, clearedId, {
                  pendingApproval: false,
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
            case 'stop-reason': {
              const noticeContent = formatStopReason(
                event.reason,
                event.message,
              )
              const notice: SystemMessage = {
                id: uid(),
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
          }
        }
      } catch (err) {
        if (!isAbortError(err)) {
          console.warn('[chat] stream errored', err)
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
        abortRef.current = null
      }
    },
    [
      conversation.id,
      conversation.mode,
      conversation.model,
      thinkingLevel,
      sessionId,
      contextWindow,
      backend,
      announcer,
      ensureSession,
      onAppendMessage,
      onPatchMessage,
      onCompactConversation,
    ],
  )

  const handleStop = useCallback(() => {
    abortRef.current?.abort()
    void backend.abortRun?.(sessionId).catch(() => {})
  }, [backend, sessionId])

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
        last.role === 'function-call' &&
        !last.running &&
        !last.pendingApproval
      ) {
        return true
      }
      return false
    })()

  const isDock = density === 'dock'
  const headerPad = isDock ? 'px-4' : 'px-9'
  const footerPad = isDock ? 'px-4 pb-4 pt-2' : 'px-9 pb-6 pt-2'

  return (
    <section className="flex-1 flex flex-col min-w-0 min-h-0">
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
          <span className="text-ink truncate min-w-0">
            {conversation.model}
          </span>
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
        density={density}
        onResolveApproval={resolveApproval}
        onAlwaysAllow={handleAlwaysAllow}
      />
      <LiveRegion announcement={announcer.announcement} />

      <footer className={footerPad}>
        <div className="mx-auto max-w-[760px]">
          <Composer
            mode={conversation.mode}
            model={conversation.model}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            functionEntries={functionEntries}
            permissionMode={approvalSettings.settings.mode}
            permissionModeLoading={!approvalSettings.loaded}
            thinkingLevel={thinkingLevel}
            onThinkingLevelChange={setThinkingLevel}
            onModeChange={(next) => onUpdateMode(conversation.id, next)}
            onModelChange={(next) => onUpdateModel(conversation.id, next)}
            onPermissionModeChange={(next) =>
              void approvalSettings.setMode(next)
            }
            onSubmit={handleSubmit}
            onStop={handleStop}
            isStreaming={streamingIndicator}
            blocked={harnessBlocked}
            blockedPlaceholder={
              conversationsCtx
                ? harnessComposerPlaceholder(conversationsCtx.harnessStatus)
                : undefined
            }
          />
        </div>
      </footer>
    </section>
  )
}
