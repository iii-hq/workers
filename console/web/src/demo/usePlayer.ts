/**
 * Drives the landing demo: walks `runScenario()` and folds its events into
 * the three pieces of state the surface renders.
 *
 * The `StreamEvent` half of the switch below is the reducer from
 * `ChatView`'s stream loop (`components/chat/ChatView.tsx`), trimmed to the
 * cases a scripted turn can produce. Keeping the same reducer is the point:
 * the transcript is built the way the real console builds it, so
 * `MessageList` renders exactly what it renders in the product.
 *
 * Lifecycle: idle → typing the prompt → streaming → done → (hold) → reset.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { StoredSpan } from '@/pages/TracesV2/api/traces'
import {
  toWaterfallData,
  type WaterfallData,
} from '@/pages/TracesV2/lib/traceTransform'
import type {
  AssistantMessage,
  Conversation,
  FunctionTriggerMessage,
  Message,
  MessagePatch,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  type Callout,
  type ChildEntry,
  type DemoEvent,
  MODEL_ID,
  PROMPT,
  runScenario,
  SESSION_ID,
  TRACE_ID,
} from './scenario'

export type Phase = 'idle' | 'typing' | 'streaming' | 'done'

/** How long the finished turn stays up before the loop restarts. */
const HOLD_MS = 14000
const TYPE_MS_PER_CHAR = 42
/** Repaint cadence while a span is still open, so its bar grows. */
const PENDING_TICK_MS = 120
/** The gate releases itself if nobody clicks approve. */
const GATE_TIMEOUT_MS = 5200
/** A callout clears itself so a stale one never annotates the wrong beat. */
const CALLOUT_MS = 11000

let seq = 0
function uid(): string {
  seq += 1
  return `demo-${seq}`
}

/** Root chat title, shown in the sidebar. */
const ROOT_TITLE = 'payments ledger'

/** One replayed child entry, as the `Message` the transcript renders. */
function childMessage(
  sessionId: string,
  index: number,
  entry: ChildEntry,
  now: number,
): Message {
  const id = `${sessionId}-${index}`
  switch (entry.role) {
    case 'thought':
      return {
        id,
        role: 'thought',
        content: entry.content,
        durationMs: entry.durationMs,
        createdAt: now,
      }
    case 'assistant':
      return {
        id,
        role: 'assistant',
        content: entry.content,
        model: MODEL_ID,
        mode: 'agent',
        createdAt: now,
      }
    case 'function-trigger':
      return {
        id,
        role: 'function-trigger',
        functionId: entry.functionId,
        input: entry.input,
        output: entry.output,
        durationMs: entry.durationMs,
        running: false,
        sessionId,
        createdAt: now,
      }
  }
}

function rootConversation(status: Conversation['status']): Conversation {
  const now = Date.now()
  return {
    id: SESSION_ID,
    title: ROOT_TITLE,
    model: MODEL_ID,
    mode: 'agent',
    messages: [],
    depth: 0,
    status,
    createdAt: now,
    updatedAt: now,
  }
}

export interface PlayerState {
  phase: Phase
  /** The selected session's transcript: the root turn, or a child's. */
  messages: Message[]
  /** Characters of the prompt typed so far, for the composer. */
  typed: string
  waterfall: WaterfallData | null
  /** The raw feed, for the live TimelineStrip masthead. */
  spans: readonly StoredSpan[]
  spanCount: number
  callout: Callout | null
  /** The turn is between visible outputs — drives the thinking shimmer. */
  isThinking: boolean
  thinkingDetail?: string
  /** Set while a call sits in the approval gate. */
  resolveApproval: (
    sessionId: string,
    functionTriggerId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  /** Root chat plus every session `harness::spawn` created, for the sidebar. */
  conversations: Conversation[]
  activeId: string
  select: (id: string) => void
  /** The selected session, when it is one of the children. */
  activeChild: Conversation | null
  /** Reader-controlled freeze: the run holds between events until resumed. */
  paused: boolean
  togglePause: () => void
  /** Restart the turn from the top, whatever state it is in. */
  replay: () => void
}

export function usePlayer(active: boolean, loop = true): PlayerState {
  const [phase, setPhase] = useState<Phase>('idle')
  const [messages, setMessages] = useState<Message[]>([])
  const [typed, setTyped] = useState('')
  const [callout, setCallout] = useState<Callout | null>(null)
  const [turnPhase, setTurnPhase] = useState<string | null>(null)
  const [isStreaming, setIsStreaming] = useState(false)
  /* Child sessions, in spawn order, each carrying its own little transcript. */
  const [children, setChildren] = useState<Conversation[]>([])
  const [activeId, setActiveId] = useState<string>(SESSION_ID)

  // Spans live in a ref: the scenario mutates them far more often than the
  // waterfall needs to repaint, and the pending tick owns the cadence.
  const spansRef = useRef<StoredSpan[]>([])
  const [waterfall, setWaterfall] = useState<WaterfallData | null>(null)
  // The strip masthead reads spans directly (it does its own layout), so the
  // repaint publishes the array alongside the derived waterfall.
  const [spans, setSpans] = useState<readonly StoredSpan[]>([])
  const [spanCount, setSpanCount] = useState(0)

  /* Pause is a ref the running loop polls (state alone would be stale inside
     the closure) plus state for the button label. */
  const [paused, setPaused] = useState(false)
  const pausedRef = useRef(false)
  const togglePause = useCallback(() => {
    pausedRef.current = !pausedRef.current
    setPaused(pausedRef.current)
  }, [])
  const [runKey, setRunKey] = useState(0)
  const replay = useCallback(() => {
    pausedRef.current = false
    setPaused(false)
    setRunKey((k) => k + 1)
  }, [])

  const gateResolveRef = useRef<(() => void) | null>(null)
  const calloutTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  )
  const runIdRef = useRef(0)

  /** Show a callout, and retire it on its own so it never outlives its beat. */
  const showCallout = useCallback((next: Callout | null) => {
    clearTimeout(calloutTimerRef.current)
    setCallout(next)
    if (next) {
      calloutTimerRef.current = setTimeout(() => setCallout(null), CALLOUT_MS)
    }
  }, [])

  const openChild = useCallback((id: string, title: string, task: string) => {
    const now = Date.now()
    setChildren((prev) =>
      prev.some((c) => c.id === id)
        ? prev
        : [
            ...prev,
            {
              id,
              title,
              model: MODEL_ID,
              mode: 'agent',
              parentId: SESSION_ID,
              depth: 1,
              spawnedBy: 'agent',
              status: 'working',
              messages: [
                {
                  id: `${id}-task`,
                  role: 'user',
                  content: task,
                  spawn: true,
                  createdAt: now,
                },
              ],
              createdAt: now,
              updatedAt: now,
            },
          ],
    )
  }, [])

  const appendChild = useCallback((id: string, entry: ChildEntry) => {
    const now = Date.now()
    setChildren((prev) =>
      prev.map((c) =>
        c.id === id
          ? {
              ...c,
              updatedAt: now,
              messages: [
                ...c.messages,
                childMessage(id, c.messages.length, entry, now),
              ],
            }
          : c,
      ),
    )
  }, [])

  const finishChild = useCallback((id: string, result: string) => {
    const now = Date.now()
    setChildren((prev) =>
      prev.map((c) =>
        c.id === id
          ? {
              ...c,
              status: 'done',
              updatedAt: now,
              messages: [
                ...c.messages,
                {
                  id: `${id}-result`,
                  role: 'assistant',
                  content: result,
                  model: MODEL_ID,
                  mode: 'agent',
                  createdAt: now,
                },
              ],
            }
          : c,
      ),
    )
  }, [])

  const append = useCallback((message: Message) => {
    setMessages((prev) => [...prev, message])
  }, [])

  const patch = useCallback((id: string, p: MessagePatch) => {
    setMessages((prev) =>
      prev.map((m) => (m.id === id ? ({ ...m, ...p } as Message) : m)),
    )
  }, [])

  const repaintSpans = useCallback(() => {
    const spans = spansRef.current
    setSpans(spans)
    setSpanCount(spans.length)
    setWaterfall(spans.length ? toWaterfallData(spans, TRACE_ID) : null)
  }, [])

  const resolveApproval = useCallback(async () => {
    gateResolveRef.current?.()
    gateResolveRef.current = null
  }, [])

  /* Repaint while anything is still open so pending bars grow. */
  useEffect(() => {
    if (phase !== 'streaming') return
    const t = setInterval(() => {
      if (!pausedRef.current && spansRef.current.some((s) => s.pending))
        repaintSpans()
    }, PENDING_TICK_MS)
    return () => clearInterval(t)
  }, [phase, repaintSpans])

  useEffect(() => {
    if (!active) return
    runIdRef.current += 1
    const runId = runIdRef.current
    const controller = new AbortController()
    const { signal } = controller
    let holdTimer: ReturnType<typeof setTimeout> | undefined

    const stale = () => signal.aborted || runIdRef.current !== runId

    const wait = (ms: number) =>
      new Promise<void>((resolve) => {
        const t = setTimeout(resolve, ms)
        signal.addEventListener(
          'abort',
          () => {
            clearTimeout(t)
            resolve()
          },
          { once: true },
        )
      })

    const holdWhilePaused = async () => {
      while (pausedRef.current && !stale()) await wait(150)
    }

    const gate = (_functionTriggerId: string) =>
      new Promise<void>((resolve) => {
        let done = false
        const finish = () => {
          if (done) return
          done = true
          gateResolveRef.current = null
          clearTimeout(timer)
          resolve()
        }
        const timer = setTimeout(finish, GATE_TIMEOUT_MS)
        gateResolveRef.current = finish
        signal.addEventListener('abort', finish, { once: true })
      })

    async function play() {
      /* reset */
      spansRef.current = []
      setMessages([])
      setChildren([])
      setActiveId(SESSION_ID)
      setTyped('')
      showCallout(null)
      setWaterfall(null)
      setSpans([])
      setSpanCount(0)
      setIsStreaming(false)
      setTurnPhase(null)
      setPhase('typing')

      /* type the prompt into the composer */
      await wait(700)
      for (let i = 1; i <= PROMPT.length; i++) {
        await holdWhilePaused()
        if (stale()) return
        setTyped(PROMPT.slice(0, i))
        await wait(TYPE_MS_PER_CHAR * (0.55 + Math.random() * 0.9))
      }
      if (stale()) return
      await wait(520)

      /* submit */
      const userMsg: UserMessage = {
        id: uid(),
        role: 'user',
        content: PROMPT,
        createdAt: Date.now(),
      }
      setTyped('')
      append(userMsg)
      setPhase('streaming')
      setIsStreaming(true)

      /* ── the ChatView stream reducer ─────────────────────────────── */
      let thoughtId: string | null = null
      let thoughtBuffer = ''
      let fcallId: string | null = null
      const fcallMap = new Map<string, string>()
      let assistantId: string | null = null
      let assistantBuffer = ''

      for await (const event of runScenario({ signal, gate })) {
        /* The generator is pull-based: holding here suspends the scenario. */
        await holdWhilePaused()
        if (stale()) return
        const ev = event as DemoEvent
        switch (ev.kind) {
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
            append(msg)
            break
          }
          case 'thought-token': {
            if (!thoughtId) break
            thoughtBuffer += ev.token
            patch(thoughtId, { content: thoughtBuffer })
            break
          }
          case 'thought-end': {
            if (!thoughtId) break
            patch(thoughtId, { streaming: false, durationMs: ev.durationMs })
            thoughtId = null
            break
          }
          case 'fcall-start': {
            if (assistantId) {
              patch(assistantId, { streaming: false })
              assistantId = null
              assistantBuffer = ''
            }
            const msg: FunctionTriggerMessage = {
              id: uid(),
              role: 'function-trigger',
              functionId: ev.functionId,
              input: ev.input,
              running: !ev.pendingApproval,
              pendingApproval: ev.pendingApproval,
              functionTriggerId: ev.functionTriggerId,
              sessionId: ev.sessionId,
              createdAt: Date.now(),
            }
            fcallId = msg.id
            if (ev.functionTriggerId) fcallMap.set(msg.id, ev.functionTriggerId)
            append(msg)
            break
          }
          case 'fcall-approval-cleared': {
            const clearedId = [...fcallMap.entries()].find(
              ([, fcid]) => fcid === ev.functionTriggerId,
            )?.[0]
            if (clearedId) {
              patch(clearedId, {
                pendingApproval: false,
                ...(ev.running ? { running: true } : {}),
              })
            }
            break
          }
          case 'fcall-end': {
            const targetId: string | null = ev.functionTriggerId
              ? ([...fcallMap.entries()].find(
                  ([, fcid]) => fcid === ev.functionTriggerId,
                )?.[0] ?? fcallId)
              : fcallId
            if (!targetId) break
            patch(targetId, {
              output: ev.output,
              durationMs: ev.durationMs,
              running: false,
              pendingApproval: false,
            })
            fcallMap.delete(targetId)
            if (targetId === fcallId) fcallId = null
            break
          }
          case 'assistant-token': {
            if (!assistantId) {
              const msg: AssistantMessage = {
                id: uid(),
                role: 'assistant',
                content: '',
                model: MODEL_ID,
                mode: 'agent',
                streaming: true,
                createdAt: Date.now(),
              }
              assistantId = msg.id
              assistantBuffer = ''
              append(msg)
            }
            assistantBuffer += ev.token
            patch(assistantId, { content: assistantBuffer })
            break
          }
          case 'assistant-end': {
            if (assistantId) patch(assistantId, { streaming: false })
            assistantId = null
            assistantBuffer = ''
            break
          }
          case 'turn-status': {
            setTurnPhase(ev.phase)
            break
          }

          /* ── demo-only markers ─────────────────────────────────── */
          case 'demo-span-open': {
            const now = Date.now()
            spansRef.current = [
              ...spansRef.current,
              {
                trace_id: TRACE_ID,
                span_id: ev.span.id,
                parent_span_id: ev.span.parent,
                name: ev.span.name,
                kind: ev.span.kind,
                service_name: ev.span.service,
                start_time_unix_nano: now,
                end_time_unix_nano: 0,
                status: 'UNSET',
                attributes: ev.span.attributes ?? [],
                events: [],
                links: [],
                pending: true,
              },
            ]
            repaintSpans()
            break
          }
          case 'demo-span-close': {
            const now = Date.now()
            spansRef.current = spansRef.current.map((s) =>
              s.span_id === ev.id
                ? {
                    ...s,
                    /* An explicit duration is what the call really costs;
                       the demo dwelt longer only so it could be seen. */
                    end_time_unix_nano:
                      ev.durationMs === undefined
                        ? now
                        : s.start_time_unix_nano + ev.durationMs,
                    status: ev.status ?? 'OK',
                    pending: false,
                  }
                : s,
            )
            repaintSpans()
            break
          }
          case 'demo-callout': {
            showCallout(ev.callout)
            break
          }
          case 'demo-session-open': {
            openChild(ev.session.id, ev.session.title, ev.session.task)
            break
          }
          case 'demo-session-msg': {
            appendChild(ev.id, ev.entry)
            break
          }
          case 'demo-session-done': {
            finishChild(ev.id, ev.result)
            break
          }
        }

        if (
          ev.kind === 'fcall-start' ||
          ev.kind === 'assistant-token' ||
          ev.kind === 'thought-start'
        ) {
          setTurnPhase(null)
        }
      }

      if (stale()) return
      setIsStreaming(false)
      setPhase('done')

      if (loop) {
        holdTimer = setTimeout(() => {
          if (!stale()) play()
        }, HOLD_MS)
      }
    }

    play()

    return () => {
      controller.abort()
      clearTimeout(holdTimer)
      clearTimeout(calloutTimerRef.current)
      gateResolveRef.current = null
    }
  }, [
    active,
    loop,
    runKey,
    append,
    patch,
    repaintSpans,
    showCallout,
    openChild,
    appendChild,
    finishChild,
  ])

  const lastRole = messages.length
    ? messages[messages.length - 1].role
    : undefined
  const rootThinking =
    isStreaming &&
    (lastRole === 'user' ||
      (lastRole === 'function-trigger' &&
        !(messages[messages.length - 1] as FunctionTriggerMessage).running &&
        !(messages[messages.length - 1] as FunctionTriggerMessage)
          .pendingApproval))

  /* Rebuilt only when a child or the root's status changes — not per token,
     which is what the transcript re-renders on. */
  const conversations = useMemo(
    () => [
      rootConversation(
        phase === 'streaming' ? 'working' : phase === 'done' ? 'done' : 'idle',
      ),
      ...children,
    ],
    [children, phase],
  )

  const activeChild =
    activeId === SESSION_ID
      ? null
      : (children.find((c) => c.id === activeId) ?? null)

  return {
    phase,
    messages: activeChild ? activeChild.messages : messages,
    typed,
    waterfall,
    spans,
    spanCount,
    callout,
    isThinking: activeChild ? activeChild.status === 'working' : rootThinking,
    thinkingDetail: activeChild
      ? 'sub-agent working…'
      : turnPhase === 'accepted'
        ? 'turn accepted, step queued…'
        : turnPhase === 'started'
          ? 'harness::turn started…'
          : undefined,
    resolveApproval,
    conversations,
    activeId,
    select: setActiveId,
    activeChild,
    paused,
    togglePause,
    replay,
  }
}

export { SESSION_ID }
