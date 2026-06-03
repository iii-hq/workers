import { useCallback, useMemo, useRef, useState } from 'react'
import { ChatView } from '@/components/chat/ChatView'
import { uid } from '@/hooks/use-conversations'
import type { ChatBackend, StreamEvent } from '@/lib/backend'
import {
  type Conversation,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

const PLAYGROUND_MODEL_OPTIONS: ModelOption[] = [
  { id: 'openai::gpt-5', label: 'gpt-5', contextWindow: 400_000 },
  { id: 'anthropic::claude-opus-4-7', label: 'claude opus 4.7', contextWindow: 1_000_000 },
  { id: 'openai::gpt-5-mini', label: 'gpt-5 mini', contextWindow: 400_000 },
]
import { EventLog, type EventLogHandle } from './EventLog'

function makeConvo(mode: Mode): Conversation {
  const now = Date.now()
  return {
    id: uid(),
    title: 'playground',
    model: PLAYGROUND_MODEL_OPTIONS[0].id,
    mode,
    messages: [],
    createdAt: now,
    updatedAt: now,
  }
}

/** Wrap a backend so each yielded event is also pushed into the event log. */
function tapBackend(
  source: ChatBackend,
  onEvent: (event: StreamEvent) => void,
): ChatBackend {
  return {
    id: `${source.id}:tapped`,
    async *stream(prompt, mode, model, opts) {
      for await (const event of source.stream(prompt, mode, model, opts)) {
        onEvent(event)
        yield event
      }
    },
  }
}

export interface PlaygroundHarnessProps {
  /** The scenario backend to stream from. */
  backend: ChatBackend
  /** Human label shown in the harness top bar. */
  label?: string
  /** Mode the chat surface should start in so the scenario reads right. */
  preferredMode?: Mode
  /** Render the live event-log rail (default true). */
  showEventLog?: boolean
}

/**
 * Self-contained reproduction of the old `#/playground` page, lifted into a
 * reusable component so each streaming scenario can be its own Storybook
 * story. Mirrors the original master surface: a `ChatView` over local
 * conversation state, the backend wrapped by `tapBackend` so every
 * `StreamEvent` mirrors into the right-hand `EventLog`. The scenario picker
 * is gone — the Storybook sidebar selects scenarios now.
 */
export function PlaygroundHarness({
  backend,
  label,
  preferredMode = 'agent',
  showEventLog = true,
}: PlaygroundHarnessProps) {
  const [convo, setConvo] = useState<Conversation>(() =>
    makeConvo(preferredMode),
  )
  const [logOpen, setLogOpen] = useState(true)
  const eventLogRef = useRef<EventLogHandle>(null)

  const tappedBackend = useMemo(
    () => tapBackend(backend, (e) => eventLogRef.current?.push(e)),
    [backend],
  )

  const handleReset = useCallback(() => {
    setConvo(makeConvo(preferredMode))
    eventLogRef.current?.clear()
  }, [preferredMode])

  const setMode = useCallback((_id: string, mode: Mode) => {
    setConvo((c) => ({ ...c, mode, updatedAt: Date.now() }))
  }, [])

  const setModel = useCallback((_id: string, model: ModelId) => {
    setConvo((c) => ({ ...c, model, updatedAt: Date.now() }))
  }, [])

  const appendMessage = useCallback((_id: string, message: Message) => {
    setConvo((c) => ({
      ...c,
      messages: [...c.messages, message],
      updatedAt: Date.now(),
    }))
  }, [])

  const updateMessage = useCallback(
    (_id: string, messageId: string, patch: MessagePatch) => {
      setConvo((c) => ({
        ...c,
        messages: c.messages.map((m) =>
          m.id === messageId ? ({ ...m, ...patch } as Message) : m,
        ),
        updatedAt: Date.now(),
      }))
    },
    [],
  )

  const compactConversation = useCallback((_id: string, marker: Message) => {
    setConvo((c) => ({
      ...c,
      messages: [marker],
      updatedAt: Date.now(),
    }))
  }, [])

  return (
    <div className="flex h-full min-h-0">
      <div className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="px-9 py-2 border-b border-rule flex items-center justify-between">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint flex items-center gap-2 min-w-0">
            <span>scenario</span>
            <span className="text-ink-ghost">·</span>
            <span className="text-ink truncate">{label ?? backend.id}</span>
          </div>
          <button
            type="button"
            onClick={handleReset}
            className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint hover:text-ink transition-colors"
            aria-label="reset playground conversation"
          >
            reset
          </button>
        </div>

        <ChatView
          key={convo.id}
          conversation={convo}
          backend={tappedBackend}
          modelOptions={PLAYGROUND_MODEL_OPTIONS}
          onUpdateModel={setModel}
          onUpdateMode={setMode}
          onAppendMessage={appendMessage}
          onPatchMessage={updateMessage}
          onCompactConversation={compactConversation}
        />
      </div>

      {showEventLog ? (
        <EventLog
          ref={eventLogRef}
          open={logOpen}
          onToggle={() => setLogOpen((v) => !v)}
        />
      ) : null}
    </div>
  )
}
