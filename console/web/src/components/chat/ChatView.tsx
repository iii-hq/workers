import { Copy, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { uid } from '@/hooks/use-conversations'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import type { ChatBackend } from '@/lib/backend'
import {
  nextApprovalsToAutoResolve,
  type PendingApprovalCandidate,
} from '@/lib/backend/auto-accept'
import { translateUiHistoryForBackend } from '@/lib/backend/history'
import type { CompactResult } from '@/lib/backend/types'
import { makeSessionId } from '@/lib/session-id'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage,
  Conversation,
  FunctionCallMessage,
  Message,
  MessagePatch,
  Mode,
  ModelId,
  ModelOption,
  SystemMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import { Composer, type ComposerSubmitPayload } from './Composer'
import { ContextUsage } from './ContextUsage'
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

/**
 * Render a `stop-reason` StreamEvent as user-readable notice text.
 * Used when an assistant turn ends abnormally (max_tokens hit, provider
 * error, abort) so the user sees a clear cause instead of a silently
 * truncated reply.
 */
function formatStopReason(
  reason: 'length' | 'error' | 'aborted' | 'function_call',
  message?: string,
): string {
  switch (reason) {
    case 'length':
      return (
        'response truncated — the model hit its max output tokens before finishing. ' +
        'increase the provider worker\'s `default_max_tokens` (or the model\'s `max_output_tokens` in the catalog), ' +
        'or send "continue" to resume.'
      )
    case 'error':
      return message
        ? `response failed: ${message}`
        : 'response failed mid-stream (no error message from the provider).'
    case 'aborted':
      return 'response aborted before completion.'
    case 'function_call':
      return 'response paused for tool call.'
  }
}

interface ChatViewProps {
  conversation: Conversation
  backend: ChatBackend
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  density?: 'route' | 'dock'
  /** When provided, renders a close affordance in the header (dock mode). */
  onClose?: () => void
  onUpdateModel: (id: string, model: ModelId) => void
  onUpdateMode: (id: string, mode: Mode) => void
  onUpdateAutoAccept: (id: string, autoAccept: boolean) => void
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
  onClose,
  onUpdateModel,
  onUpdateMode,
  onUpdateAutoAccept,
  onAppendMessage,
  onPatchMessage,
  onCompactConversation,
}: ChatViewProps) {
  const [isStreaming, setIsStreaming] = useState(false)
  const abortRef = useRef<AbortController | null>(null)
  const [copied, setCopied] = useState(false)
  const { functionEntries } = useFunctionsCatalog(backend.id)

  // Matches iii.session.id on every span so the traces UI can group by it.
  const sessionId = useMemo(
    () => makeSessionId(conversation.id),
    [conversation.id],
  )

  const contextWindow = useMemo(() => {
    const match = modelOptions.find((o) => o.id === conversation.model)
    return match?.contextWindow
  }, [modelOptions, conversation.model])

  /* Per-conversation auto-accept. When `conversation.autoAccept` is
   * true, any pending function-call approval that surfaces in the
   * transcript is auto-resolved with `decision: "allow"` so the user
   * doesn't have to click. The approval card still renders briefly
   * for audit — we just press the button on their behalf.
   *
   * Dedupe by iii `function_call_id` in a ref so the effect (which
   * re-runs on every message-list patch) doesn't fire `resolve`
   * twice for the same pending entry. The ref resets when the
   * conversation id changes — a fresh conversation starts with a
   * clean slate. */
  const autoResolvedRef = useRef<Set<string>>(new Set())
  useEffect(() => {
    autoResolvedRef.current = new Set()
  }, [conversation.id])

  useEffect(() => {
    if (!conversation.autoAccept) return
    if (!backend.resolveApproval) return
    const candidates: PendingApprovalCandidate[] = []
    for (const m of conversation.messages) {
      if (
        m.role === 'function-call' &&
        m.pendingApproval === true &&
        typeof m.functionCallId === 'string' &&
        typeof m.sessionId === 'string'
      ) {
        candidates.push({
          functionCallId: m.functionCallId,
          sessionId: m.sessionId,
        })
      }
    }
    const todo = nextApprovalsToAutoResolve(candidates, autoResolvedRef.current)
    if (todo.length === 0) return
    for (const p of todo) {
      autoResolvedRef.current.add(p.functionCallId)
      void backend.resolveApproval(p.sessionId, p.functionCallId, 'allow').catch(
        (err) => {
          /* Same warn-and-continue posture as the per-card resolve
           * handler — a transient failure shouldn't break the loop
           * for the next pending entry. The orchestrator will keep
           * the approval pending; the user can still click manually. */
          console.warn(
            '[chat] auto-accept: approval::resolve failed',
            { functionCallId: p.functionCallId, sessionId: p.sessionId, err },
          )
          autoResolvedRef.current.delete(p.functionCallId)
        },
      )
    }
  }, [conversation.autoAccept, conversation.messages, backend])

  const handleCopySessionId = useCallback(() => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return
    void navigator.clipboard.writeText(sessionId).then(() => {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }, [sessionId])

  const handleSubmit = useCallback(
    async (payload: ComposerSubmitPayload) => {
      const conversationId = conversation.id

      // Snapshot prior history BEFORE appending the new user msg —
      // run::start overwrites flat state with whatever we send.
      const priorHistory = translateUiHistoryForBackend(conversation.messages)

      const userMsg: UserMessage = {
        id: uid(),
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
            conversation.model,
            priorHistory,
            contextWindow,
          )
          if (result.status === 'ok') {
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
            // empty | busy | overflow | error: nothing rewritten server-side.
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
          conversation.model,
          { signal: controller.signal, sessionId, history: priorHistory },
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
                const existing = [...fcallMap.entries()].find(
                  ([, fcid]) => fcid === event.functionCallId,
                )?.[0]
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
              if (!fcallId) break
              onPatchMessage(conversationId, fcallId, {
                output: event.output,
                durationMs: event.durationMs,
                running: false,
                pendingApproval: false,
              })
              fcallMap.delete(fcallId)
              fcallId = null
              break
            }
            case 'assistant-token': {
              if (!assistantId) {
                const msg: AssistantMessage = {
                  id: uid(),
                  role: 'assistant',
                  content: '',
                  model: conversation.model,
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
              // Server-side context-compaction finished rewriting the
              // session's flat-state. Append a marker so:
              //   1. The user sees that compaction happened.
              //   2. estimateConversationTokens stops counting pre-marker
              //      messages → the CTX bar drops to the real value.
              // We append (not replace) so the transcript stays scrollable.
              const marker: SystemMessage = {
                id: uid(),
                role: 'system',
                kind: 'compaction',
                content:
                  event.mode === 'sync'
                    ? `compacted ${event.tokensBefore.toLocaleString()} tokens before continuing`
                    : `compacted ${event.tokensBefore.toLocaleString()} tokens (background)`,
                tone: 'info',
                summaryText: event.summaryText,
                tokensBefore: event.tokensBefore,
                createdAt: Date.now(),
              }
              onAppendMessage(conversationId, marker)
              break
            }
            case 'stop-reason': {
              // The assistant turn ended abnormally — show the user why
              // instead of leaving the response looking like it just
              // ran out of words. Pre-fix this event didn't exist and
              // the same condition produced a silently truncated reply.
              const notice: SystemMessage = {
                id: uid(),
                role: 'system',
                kind: 'notice',
                content: formatStopReason(event.reason, event.message),
                tone: event.reason === 'error' ? 'error' : 'warn',
                createdAt: Date.now(),
              }
              onAppendMessage(conversationId, notice)
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
        // Backends signal semantic failures via fcall-end; thrown = abort or bug.
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
      conversation.messages,
      sessionId,
      backend,
      onAppendMessage,
      onPatchMessage,
      onCompactConversation,
    ],
  )

  const handleStop = useCallback(() => {
    abortRef.current?.abort()
  }, [])

  // Covers the gap between submit / fcall-end and the next streamed token,
  // where the assistant/thought shimmer hasn't yet rendered.
  const isThinking =
    isStreaming &&
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
          <div className="flex items-center gap-2">
            <StatusDot
              tone={isStreaming ? 'accent' : 'ink'}
              pulse={isStreaming}
            />
            <span className="text-ink-faint">
              {isStreaming ? 'streaming' : 'ready'}
            </span>
          </div>
          {onClose ? (
            <button
              type="button"
              onClick={onClose}
              aria-label="close chat dock"
              className="flex items-center justify-center size-6 -mr-1 text-ink-faint hover:text-ink transition-colors"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>
      </header>

      <MessageList
        messages={conversation.messages}
        isThinking={isThinking}
        density={density}
        onResolveApproval={
          backend.resolveApproval
            ? async (sessionId, functionCallId, decision) => {
                await backend.resolveApproval?.(
                  sessionId,
                  functionCallId,
                  decision,
                )
              }
            : undefined
        }
      />

      <footer className={footerPad}>
        <div className="mx-auto max-w-[760px]">
          <Composer
            mode={conversation.mode}
            model={conversation.model}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            functionEntries={functionEntries}
            autoAccept={conversation.autoAccept}
            onModeChange={(next) => onUpdateMode(conversation.id, next)}
            onModelChange={(next) => onUpdateModel(conversation.id, next)}
            onAutoAcceptChange={(next) =>
              onUpdateAutoAccept(conversation.id, next)
            }
            onSubmit={handleSubmit}
            onStop={handleStop}
            isStreaming={isStreaming}
          />
        </div>
      </footer>
    </section>
  )
}
