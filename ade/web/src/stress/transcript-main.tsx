/**
 * The transcript stress page (`/transcript-stress.html`, dev server only): the
 * real `MessageList` over a generated transcript, with no engine behind it.
 * `pnpm test:stress` drives it to prove what jsdom cannot, since it has no
 * layout: the window stays bounded while a session grows, and scrolling keeps
 * the reader's place.
 *
 * URL params:
 *   ?turns=100     turns in the transcript (each adds a few rows)
 *   ?more=1        report older history to load, so prepends can land
 *   ?headbatch=1   start the transcript on a batch, so a page can merge into it
 *   ?uniform=46    one prose row per turn, this many repeats tall (46 is about
 *                  800px, well over the estimate) and short every fifth turn
 *   ?sidebar=300   the real `ConversationSidebar` over this many chats instead
 */

import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/shadcn/tooltip'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import type { Conversation, FunctionCallMessage, Message } from '@/types/chat'
import { MessageList } from '../components/chat/MessageList'
import '../index.css'

interface TranscriptDriver {
  /** Add turns at the tail, as a live session does. */
  append: (turns: number) => void
  /** Land older history above the current head. */
  prepend: (turns: number) => void
  /** Land calls directly above the head, merging into the batch there. */
  prependCalls: (id: string, calls: number) => void
  /** Start a batch at the tail with a call still running. */
  startBatch: (id: string, calls: number) => void
  settleBatch: (id: string) => void
  /** Grow the reply at the tail, as a streaming token does. */
  stream: (text: string) => void
  finishStream: () => void
  focus: (id: string | null) => void
  /** Hold a call for approval at the given turn. */
  holdForApproval: (turn: number) => void
  /** Whether a turn is running: what tells a batch between calls from an over one. */
  setWorking: (working: boolean) => void
  /** Park one call of a batch for approval. */
  holdCall: (id: string) => void
}

interface SidebarDriver {
  /** Bring a chat to the top of the list, as new activity on it does. */
  touch: (id: string) => void
}

declare global {
  interface Window {
    __transcript?: TranscriptDriver
    __sidebar?: SidebarDriver
  }
}

const params = new URLSearchParams(window.location.search)
const initialTurns = Number(params.get('turns')) || 100
const hasMore = params.get('more') === '1'
const headBatch = params.get('headbatch') === '1'
const uniform = Number(params.get('uniform')) || 0

const PROSE =
  'The transcript has to keep its reader in place while rows come and go. '

function call(
  id: string,
  extra: Partial<FunctionCallMessage> = {},
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId: 'shell::run',
    description: `call ${id}`,
    input: { command: id },
    output: { ok: true },
    functionCallId: `fc_${id}`,
    createdAt: 0,
    ...extra,
  }
}

/**
 * Real rows measure p50 431px, p90 1858px against a 320px estimate, so these
 * are wrong the way real ones are: mostly over it, some by thousands of pixels.
 */
function proseRepeats(shape: number): number {
  if (shape % 9 === 0) return 150
  if (shape % 4 === 0) return 40
  return 2 + ((shape * 5) % 20)
}

function turn(index: number): Message[] {
  const shape = Math.abs(index)
  if (uniform) {
    return [
      {
        id: `row-${index}`,
        role: 'assistant',
        content: `Row ${index}. ${PROSE.repeat(shape % 5 === 0 ? 1 : uniform)}`,
        stopReason: 'end',
        createdAt: 0,
      },
    ]
  }
  const batch = (shape * 7) % 5
  return [
    {
      id: `user-${index}`,
      role: 'user',
      content: `Question ${index}`,
      createdAt: 0,
    },
    {
      id: `intro-${index}`,
      role: 'assistant',
      content: `Intro ${index}. ${PROSE.repeat(1 + ((shape * 3) % 8))}`,
      stopReason: 'function_call',
      createdAt: 0,
    },
    ...Array.from({ length: batch + 1 }, (_, position) =>
      call(`call-${index}-${position}`),
    ),
    // Prose after a batch that more calls follow: a phase summary, the row a
    // working agent produces most, and the one whose neighbors are batches.
    {
      id: `progress-${index}`,
      role: 'assistant',
      content: `Progress ${index}. ${PROSE.repeat(1 + (shape % 3))}`,
      stopReason: 'function_call',
      createdAt: 0,
    },
    call(`next-${index}`),
    {
      id: `reply-${index}`,
      role: 'assistant',
      content: `Reply ${index}. ${PROSE.repeat(proseRepeats(shape))}`,
      stopReason: 'end',
      createdAt: 0,
    },
  ]
}

const range = (from: number, count: number) =>
  Array.from({ length: count }, (_, offset) => turn(from + offset)).flat()

const initialMessages = (): Message[] => [
  ...(headBatch ? [call('head-0'), call('head-1')] : []),
  ...range(0, initialTurns),
]

function App() {
  const [messages, setMessages] = useState<Message[]>(initialMessages)
  const [focusMessageId, setFocusMessageId] = useState<string | null>(null)
  const [working, setWorking] = useState(false)

  useEffect(() => {
    let nextTail = initialTurns
    let nextHead = -1
    const patchCalls = (
      match: (message: FunctionCallMessage) => boolean,
      patch: Partial<FunctionCallMessage>,
    ) =>
      setMessages((current) =>
        current.map((message) =>
          message.role === 'function-call' && match(message)
            ? { ...message, ...patch }
            : message,
        ),
      )
    window.__transcript = {
      append(turns) {
        const added = range(nextTail, turns)
        nextTail += turns
        setMessages((current) => [...current, ...added])
      },
      prepend(turns) {
        const added = range(nextHead - turns + 1, turns)
        nextHead -= turns
        setMessages((current) => [...added, ...current])
      },
      prependCalls(id, calls) {
        const added = Array.from({ length: calls }, (_, position) =>
          call(`${id}-${position}`),
        )
        setMessages((current) => [...added, ...current])
      },
      startBatch(id, calls) {
        setMessages((current) => [
          ...current,
          ...Array.from({ length: calls }, (_, position) =>
            call(`${id}-${position}`, { running: true, output: undefined }),
          ),
        ])
      },
      settleBatch(id) {
        patchCalls((message) => message.id.startsWith(`${id}-`), {
          running: false,
          pendingApproval: false,
          output: { ok: true },
        })
      },
      stream(text) {
        setMessages((current) => {
          const last = current[current.length - 1]
          return last?.role === 'assistant' && last.streaming
            ? [
                ...current.slice(0, -1),
                { ...last, content: last.content + text },
              ]
            : [
                ...current,
                {
                  id: `stream-${nextTail++}`,
                  role: 'assistant',
                  content: text,
                  stopReason: 'end',
                  streaming: true,
                  createdAt: 0,
                },
              ]
        })
      },
      finishStream() {
        setMessages((current) =>
          current.map((message) =>
            message.role === 'assistant' && message.streaming
              ? { ...message, streaming: false }
              : message,
          ),
        )
      },
      focus: setFocusMessageId,
      setWorking,
      holdCall(id) {
        patchCalls((message) => message.id === id, {
          running: false,
          pendingApproval: true,
          sessionId: 'stress-session',
          output: undefined,
        })
      },
      holdForApproval(index) {
        patchCalls((message) => message.id === `call-${index}-0`, {
          pendingApproval: true,
          output: undefined,
        })
      },
    }
  }, [])

  return (
    <TooltipProvider>
      <div className="@container flex h-screen min-h-0 flex-col">
        <MessageList
          messages={messages}
          turnVisualPhase={working ? 'calling' : 'idle'}
          focusMessageId={focusMessageId}
          onFocusMessageHandled={() => setFocusMessageId(null)}
          history={
            hasMore
              ? { hasMore: true, loadingOlder: false, error: undefined }
              : undefined
          }
          onResolveApproval={async () => undefined}
        />
      </div>
    </TooltipProvider>
  )
}

const HOUR_MS = 3_600_000

/** Mostly plain chats, a parent with live and finished sub-agents every tenth. */
function sidebarChats(count: number): Conversation[] {
  const now = Date.now()
  return Array.from({ length: count }, (_, index) => {
    const base: Conversation = {
      id: `chat-${index}`,
      title: `Chat ${index} about something long enough to fade`,
      model: null,
      messages: [],
      hydrated: true,
      workingDir: `/work/project-${index % 7}`,
      createdAt: now - index * HOUR_MS,
      updatedAt: now - index * HOUR_MS,
    }
    if (index % 10 !== 9 || index === 0) return [base]
    return [
      base,
      ...['working', 'done', 'done'].map(
        (status, child): Conversation => ({
          ...base,
          id: `chat-${index}-sub-${child}`,
          title: `Sub-agent ${child} of chat ${index}`,
          parentId: base.id,
          status: status as Conversation['status'],
        }),
      ),
    ]
  }).flat()
}

function SidebarApp({ count }: { count: number }) {
  const [conversations, setConversations] = useState(() => sidebarChats(count))
  const [activeId, setActiveId] = useState<string | null>(null)
  useEffect(() => {
    window.__sidebar = {
      touch(id) {
        setConversations((current) =>
          current.map((c) =>
            c.id === id ? { ...c, updatedAt: Date.now() + 1000 } : c,
          ),
        )
      },
    }
  }, [])
  return (
    <TooltipProvider>
      <aside aria-label="Chats" className="flex h-screen w-[300px] flex-col">
        <ConversationSidebar
          conversations={conversations}
          activeId={activeId}
          onSelect={setActiveId}
          onRename={() => undefined}
          onRemove={() => undefined}
        />
      </aside>
    </TooltipProvider>
  )
}

const root = document.getElementById('root')
const sidebarCount = Number(params.get('sidebar')) || 0
if (root) {
  createRoot(root).render(
    <StrictMode>
      {sidebarCount ? <SidebarApp count={sidebarCount} /> : <App />}
    </StrictMode>,
  )
}
