import { useCallback, useRef, useState } from 'react'
import { Prompt } from '@/components/ui/Prompt'
import { StatusDot } from '@/components/ui/StatusDot'
import { uid } from '@/hooks/use-conversations'
import type { ChatBackend } from '@/lib/backend'
import type {
  AssistantMessage,
  Conversation,
  FunctionCallMessage,
  Message,
  MessagePatch,
  Mode,
  ModelId,
  ModelOption,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import { Composer, type ComposerSubmitPayload } from './Composer'
import { MessageList } from './MessageList'

function isAbortError(err: unknown): boolean {
  return (
    err instanceof DOMException &&
    (err.name === 'AbortError' || err.code === DOMException.ABORT_ERR)
  )
}

interface ChatViewProps {
  conversation: Conversation
  backend: ChatBackend
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  onUpdateModel: (id: string, model: ModelId) => void
  onUpdateMode: (id: string, mode: Mode) => void
  onAppendMessage: (id: string, message: Message) => void
  onPatchMessage: (id: string, messageId: string, patch: MessagePatch) => void
}

export function ChatView({
  conversation,
  backend,
  modelOptions,
  catalogLoading,
  onUpdateModel,
  onUpdateMode,
  onAppendMessage,
  onPatchMessage,
}: ChatViewProps) {
  const [isStreaming, setIsStreaming] = useState(false)
  const abortRef = useRef<AbortController | null>(null)

  const handleSubmit = useCallback(
    async (payload: ComposerSubmitPayload) => {
      const conversationId = conversation.id

      /* user turn */
      const userMsg: UserMessage = {
        id: uid(),
        role: 'user',
        content: payload.text,
        attachments:
          payload.attachments.length > 0 ? payload.attachments : undefined,
        createdAt: Date.now(),
      }
      onAppendMessage(conversationId, userMsg)

      const controller = new AbortController()
      abortRef.current = controller
      setIsStreaming(true)

      /* per-turn pointers so we know which id to patch when events arrive */
      let thoughtId: string | null = null
      let thoughtBuffer = ''
      let fcallId: string | null = null
      // PR #150: same iii function_call_id can fire fcall-start twice
      // (once for function_execution_start, once for approval_requested).
      // Map UI message id → iii function_call_id so dedupe is reliable.
      const fcallMap = new Map<string, string>()
      let assistantId: string | null = null
      let assistantBuffer = ''

      try {
        for await (const event of backend.stream(
          payload.text || '(attachments only)',
          conversation.mode,
          conversation.model,
          { signal: controller.signal },
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
              /* reset so a subsequent fcall-start gets a fresh slot */
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
          }
          // Any inbound activity event re-arms the STREAMING pill
          // (the backend stream stays alive across multiple turns now).
          if (
            event.kind === 'fcall-start' ||
            event.kind === 'assistant-token' ||
            event.kind === 'thought-start'
          ) {
            setIsStreaming(true)
          }
        }
      } catch (err) {
        /* AbortError (caller aborted, or backend threw one) is a benign
           cancellation. Anything else gets surfaced for debugging — backends
           are expected to express semantic failures via fcall-end payloads
           rather than thrown errors. */
        if (!isAbortError(err)) {
          console.warn('[chat] stream errored', err)
        }
      } finally {
        /* defensive cleanup: if we aborted mid-stream, clear streaming flags. */
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
      backend,
      onAppendMessage,
      onPatchMessage,
    ],
  )

  const handleStop = useCallback(() => {
    abortRef.current?.abort()
  }, [])

  /* "agent is thinking" indicator: stream is active but no streaming
     thought/assistant message is currently visible. Covers two gaps:
     (a) right after submit, before the first token/thought/fcall arrives;
     (b) after each fcall-end, while the harness transitions through
     function_finalize → steering_check → next turn's provider stream.
     A streaming assistant/thought message renders its own "thinking…"
     shimmer when content is empty, so we don't double-render those. */
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

  return (
    <section className="flex-1 flex flex-col min-w-0 min-h-0">
      <header className="flex items-center justify-between px-9 py-3 border-b border-rule">
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint flex items-center gap-2">
          <Prompt symbol="$">{conversation.model}</Prompt>
          <span className="text-ink-ghost">·</span>
          <span className="text-ink-faint">{conversation.mode}</span>
        </div>
        <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.06em]">
          <StatusDot
            tone={isStreaming ? 'accent' : 'ink'}
            pulse={isStreaming}
          />
          <span className="text-ink-faint">
            {isStreaming ? 'streaming' : 'ready'}
          </span>
        </div>
      </header>

      <MessageList
        messages={conversation.messages}
        isThinking={isThinking}
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

      <footer className="px-9 pb-6 pt-2">
        <div className="mx-auto max-w-[760px]">
          <Composer
            mode={conversation.mode}
            model={conversation.model}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            onModeChange={(next) => onUpdateMode(conversation.id, next)}
            onModelChange={(next) => onUpdateModel(conversation.id, next)}
            onSubmit={handleSubmit}
            onStop={handleStop}
            isStreaming={isStreaming}
          />
        </div>
      </footer>
    </section>
  )
}
