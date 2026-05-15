import { useCallback, useMemo, useRef, useState } from 'react'
import { ChatView } from '@/components/chat/ChatView'
import { uid } from '@/hooks/use-conversations'
import type { ChatBackend, StreamEvent } from '@/lib/backend'
import {
  type Conversation,
  DEFAULT_MODEL,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
} from '@/types/chat'
import { EventLog, type EventLogHandle } from './EventLog'
import { ScenarioPicker } from './ScenarioPicker'
import { findScenario, SCENARIOS } from './scenarios'

function makeConvo(mode: Mode): Conversation {
  const now = Date.now()
  return {
    id: uid(),
    title: 'playground',
    model: DEFAULT_MODEL,
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

const FIRST = SCENARIOS[0]

export function Playground() {
  const [selectedId, setSelectedId] = useState<string>(FIRST.id)
  const [convo, setConvo] = useState<Conversation>(() =>
    makeConvo(FIRST.preferredMode ?? 'agent'),
  )
  const [logOpen, setLogOpen] = useState(true)
  const eventLogRef = useRef<EventLogHandle>(null)

  const scenario = findScenario(selectedId) ?? FIRST

  const tappedBackend = useMemo(
    () => tapBackend(scenario.backend, (e) => eventLogRef.current?.push(e)),
    [scenario.backend],
  )

  const handleSelect = useCallback((id: string) => {
    const next = findScenario(id)
    if (!next) return
    setSelectedId(id)
    setConvo(makeConvo(next.preferredMode ?? 'agent'))
    eventLogRef.current?.clear()
  }, [])

  const handleReset = useCallback(() => {
    setConvo(makeConvo(scenario.preferredMode ?? 'agent'))
    eventLogRef.current?.clear()
  }, [scenario.preferredMode])

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

  return (
    <div className="flex-1 flex min-h-0">
      <ScenarioPicker selectedId={selectedId} onSelect={handleSelect} />

      <div className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="px-9 py-2 border-b border-rule flex items-center justify-between">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint flex items-center gap-2 min-w-0">
            <span>scenario</span>
            <span className="text-ink-ghost">·</span>
            <span className="text-ink truncate">{scenario.label}</span>
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
          onUpdateModel={setModel}
          onUpdateMode={setMode}
          onAppendMessage={appendMessage}
          onPatchMessage={updateMessage}
        />
      </div>

      <EventLog
        ref={eventLogRef}
        open={logOpen}
        onToggle={() => setLogOpen((v) => !v)}
      />
    </div>
  )
}
