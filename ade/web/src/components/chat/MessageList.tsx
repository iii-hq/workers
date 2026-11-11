import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowDown, ChevronDown } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import {
  type CSSProperties,
  forwardRef,
  type ReactNode,
  type Ref,
  type SyntheticEvent,
  type TouchEvent,
  type UIEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type WheelEvent,
} from 'react'
import {
  isErrorOutput,
  resultEnvelope,
} from '@/components/function-call/FunctionCallCard'
import {
  firstRendered,
  rawRedactor,
  useFunctionCallRenderers,
} from '@/components/function-call/renderer-registry'
import {
  collapsedTimelineActivities,
  groupTimelineActivities,
  observerActivityRows,
  type TimelineActivityGroupRow,
  timelineActivityPresentationKey,
} from '@/components/observer-activity/grouping'
import {
  type ObserverRegistration,
  registrationFromCall,
  registrationFromRow,
} from '@/components/observer-activity/model'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { Button } from '@/components/ui/Button'
import { Hint } from '@/components/ui/Hint'
import { ImageViewer } from '@/components/ui/ImageViewer'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import { useMediaQuery } from '@/hooks/use-media-query'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  assistantCopyText,
  functionCallsByAssistant,
} from '@/lib/function-call-copy'
import {
  entryIdOfMessage,
  notificationBindingId,
  observerFiredName,
} from '@/lib/sessions/entry-mapper'
import {
  assistantHeaderIds,
  changedFilesFromCalls,
  turnDurationMs,
} from '@/lib/turn-summary'
import { cn } from '@/lib/utils'
import type {
  Conversation,
  FunctionCallMessage,
  Message as MessageType,
} from '@/types/chat'
import { ConversationSkeleton } from './ConversationSkeleton'
import type { WorktreePickerOptions } from './DirectoryPicker'
import { EmptyState, type EmptyStateProps } from './EmptyState'
import { functionCallGroups } from './function-call-groups'
import { imageZoomTarget } from './image-zoom-target'
import { Message, type SpawnTaskContext } from './Message'
import { ModelWaitingIndicator } from './ModelWaitingIndicator'
import { THOUGHT_SETTLE_DURATION_MS } from './ThoughtMessage'
import {
  nextTailScrollTop,
  type PrependScrollMetrics,
  scrollTopAfterPrepend,
  TAIL_GLIDE_SETTLE_DISTANCE_PX,
  TAIL_REARM_DISTANCE_PX,
  type TailScrollState,
  tailDistanceFromBottom,
  tailScrollTarget,
  tailStateAfterScroll,
} from './tail-scroll'
import {
  agentMovedPast,
  estimateRowSlot,
  type FlatTimelineRow,
  flattenTimelineRows,
  isHistoryPrepend,
  isLiveRow,
  isObserverFireRow,
  isProseRow,
  measureRowSlot,
  observeNonEmptyRect,
  rowIndexByEntryId,
  rowSlot,
  rowsWithSpacers,
  topLevelRowKey,
  useSelectedRowKeys,
  WINDOW_MIN_ROWS,
  WINDOW_OVERSCAN_ROWS,
  windowedRowIndexes,
} from './transcript-window'
import type { TurnVisualPhase } from './turn-visual-state'
import './chat-motion.css'
import './tool-group.css'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface MessageListProps {
  messages: MessageType[]
  /**
   * False while an existing conversation is still loading its durable
   * transcript. Initializing content stays pinned instantly until this flips,
   * so opening a chat never glides through a partial history.
   */
  transcriptHydrated?: boolean
  /** Show the model-waiting indicator at the bottom while the agent is
      between visible outputs (after submit, or between fcall-end and the
      next turn's first token). */
  isThinking?: boolean
  /** Under-the-hood context shown in the waiting indicator (e.g. "dispatching
      zai::glm-5.2" or the session's status_reason). Falls back to "thinking…"
      when absent. */
  thinkingDetail?: string
  /** Current protocol-derived phase; presence animations never own this. */
  turnVisualPhase?: TurnVisualPhase
  /** User message that began the active turn; keeps elapsed time across gaps. */
  turnKey?: string
  density?: 'route' | 'dock'
  /** Rendered at the top of the transcript scroller, above the messages, so
      it scrolls away with them instead of unmounting. When set it replaces
      the `EmptyState` on an empty transcript (landing demo). */
  header?: ReactNode
  onResolveApproval?: (
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  onAlwaysAllow?: (
    sessionId: string,
    functionCallId: string,
    functionId: string,
  ) => Promise<void>
  onResolveFilesystemAccess?: (
    sessionId: string,
    functionCallId: string,
    action: FilesystemAccessAction,
  ) => Promise<void>
  onManageFilesystemAccess?: () => void
  /** Open the model/provider picker from the empty provider state. */
  onConfigureProvider?: () => void
  /** Current child-session identity shown by direct spawn seed messages. */
  spawnContext?: SpawnTaskContext
  /** Session agent profile name shown on assistant message headers. */
  agentName?: string
  /**
   * The conversation's session id, handed to user-message chips so a stored
   * attachment can be downloaded. Optional: showcase and fixture surfaces
   * have no session store behind them.
   */
  sessionId?: string
  workingDir?: string | null
  onWorkingDirChange?: (next: string) => void
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
  /**
   * Render every function-call card (and group) already expanded. Off in the
   * product, where a turn's calls collapse to one line each; on for showcase
   * surfaces whose whole point is the result renderers.
   */
  defaultOpenCalls?: boolean
  /** Observer rows by observer id, for observer-fired card detail. */
  observersById?: ReadonlyMap<string, SessionObserverInfo>
  /**
   * External landing request (trace → "go to message"): once this row's node
   * exists, the list centers it, flashes it, and calls
   * `onFocusMessageHandled` so the owner can consume the request. A target
   * hidden behind a collapsed activity group is revealed first.
   */
  focusMessageId?: string | null
  onFocusMessageHandled?: () => void
  /**
   * Where the loaded window ends at the top (`Conversation.history`). Drives
   * the status row above the first message and the upward infinite scroll:
   * while `hasMore`, a sentinel entering the viewport asks `onLoadOlder` for
   * the page above. Absent on surfaces that hold the whole transcript.
   */
  history?: Conversation['history']
  onLoadOlder?: () => void
  /**
   * Fetch whole entries for placeholder calls a paged read left behind, so a
   * group can show them when expanded, a single card can show them when
   * opened, or a renderer that wants the call visible while collapsed can
   * draw it. May resolve to whether the read succeeded, so an open card can
   * show a failure instead of a skeleton.
   */
  onLoadActivityEntries?: (entryIds: string[]) => Promise<boolean> | undefined
  /**
   * Open the side panel's Observers section. Given, a run of observer events
   * shows as an eye and a count that opens it, not an inline group.
   */
  onShowObservers?: () => void
}

const ACTIVITY_RESULT_DWELL_MS = 250

/**
 * How far above the viewport the top sentinel counts as "reached". A page is
 * requested before the reader hits the edge, so the row they are heading for
 * is usually there by the time they arrive.
 */
const OLDER_PAGE_ROOT_MARGIN = '600px 0px 0px 0px'
/** No content resize for this long after hydration = the open has settled. */
const OPEN_SETTLE_QUIET_MS = 120
/** Reveal no later than this after hydration, settled or not. */
const OPEN_SETTLE_MAX_MS = 800
/** A fast open shows nothing rather than a flash of the loading line. */
const OPEN_INDICATOR_DELAY_MS = 150
const HISTORY_EDGE_BUTTON_CLASS =
  'cursor-pointer underline decoration-dotted underline-offset-2 hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent'

/**
 * Transcript entry ids a placeholder row needs fetched: its own entry and,
 * when the read named it, the result entry. A fallback row whose id IS the
 * result entry asks for that one entry.
 */
export function unloadedEntryIds(
  rows: readonly FunctionCallMessage[],
): string[] {
  const ids = new Set<string>()
  for (const row of rows) {
    if (!row.unloaded) continue
    ids.add(entryIdOfMessage(row.id))
    if (row.resultEntryId) ids.add(row.resultEntryId)
  }
  return [...ids]
}

interface ThoughtPresenceState {
  signature: string
  streamingIds: ReadonlySet<string>
  settlingIds: ReadonlySet<string>
}

/** Preserve only thoughts this mounted list observed finishing. */
function useSettlingThoughtIds(
  messages: readonly MessageType[],
  reducedMotion: boolean,
): ReadonlySet<string> {
  const thoughts = messages.filter((message) => message.role === 'thought')
  const signature = `${reducedMotion ? 'reduced' : 'motion'}\u0000${thoughts
    .map((message) => `${message.id}:${message.streaming ? 'live' : 'done'}`)
    .join('\u0000')}`
  const currentStreamingIds = new Set(
    thoughts
      .filter((message) => message.streaming)
      .map((message) => message.id),
  )
  const currentThoughtIds = new Set(thoughts.map((message) => message.id))
  const [presenceState, setPresenceState] = useState<ThoughtPresenceState>(
    () => ({
      signature,
      streamingIds: currentStreamingIds,
      settlingIds: new Set(),
    }),
  )
  const timersRef = useRef(new Map<string, number>())
  let renderedPresenceState = presenceState

  if (presenceState.signature !== signature) {
    const settlingIds = reducedMotion
      ? new Set<string>()
      : new Set(
          [...presenceState.settlingIds].filter(
            (id) => currentThoughtIds.has(id) && !currentStreamingIds.has(id),
          ),
        )
    if (!reducedMotion) {
      for (const id of presenceState.streamingIds) {
        if (currentThoughtIds.has(id) && !currentStreamingIds.has(id)) {
          settlingIds.add(id)
        }
      }
    }
    renderedPresenceState = {
      signature,
      streamingIds: currentStreamingIds,
      settlingIds,
    }
    // State, unlike a render-mutated ref, rolls back with an abandoned
    // concurrent render. React restarts this component before committing.
    setPresenceState(renderedPresenceState)
  }

  useEffect(() => {
    if (reducedMotion) {
      for (const timer of timersRef.current.values()) window.clearTimeout(timer)
      timersRef.current.clear()
      return
    }

    for (const id of renderedPresenceState.settlingIds) {
      if (timersRef.current.has(id)) continue
      const timer = window.setTimeout(() => {
        timersRef.current.delete(id)
        setPresenceState((current) => {
          if (!current.settlingIds.has(id)) return current
          const next = new Set(current.settlingIds)
          next.delete(id)
          return { ...current, settlingIds: next }
        })
      }, THOUGHT_SETTLE_DURATION_MS)
      timersRef.current.set(id, timer)
    }
  }, [reducedMotion, renderedPresenceState.settlingIds])

  useEffect(
    () => () => {
      for (const timer of timersRef.current.values()) window.clearTimeout(timer)
      timersRef.current.clear()
    },
    [],
  )

  return renderedPresenceState.settlingIds
}

/**
 * Keys whose rows were appended live since the last commit: only those get
 * the appear animation. A key is live when it is new AND sits after the
 * newest previously known key that still exists. Anything new BEFORE that
 * anchor was inserted into history: an older page prepended by the reader
 * scrolling up, or a placeholder swapped for its whole entry. Animating
 * those made every page-up look like a burst of fresh messages.
 */
export function liveAppendedKeys(
  previousKeys: readonly string[],
  keys: readonly string[],
): Set<string> {
  const known = new Set(previousKeys)
  let anchor = -1
  for (let i = previousKeys.length - 1; i >= 0 && anchor === -1; i--) {
    anchor = keys.indexOf(previousKeys[i])
  }
  return new Set(keys.filter((key, index) => index > anchor && !known.has(key)))
}

function useLiveEntryKeys(
  keys: readonly string[],
  transcriptHydrated: boolean,
): ReadonlySet<string> {
  const signature = keys.join('\u0000')
  // biome-ignore lint/correctness/useExhaustiveDependencies: the primitive signature is the list's semantic identity.
  const stableKeys = useMemo(() => keys, [signature])
  const previousKeysRef = useRef<readonly string[]>(stableKeys)
  const wasHydratedRef = useRef(transcriptHydrated)
  const animate = transcriptHydrated && wasHydratedRef.current
  const liveKeys: ReadonlySet<string> = animate
    ? liveAppendedKeys(previousKeysRef.current, stableKeys)
    : new Set()

  useLayoutEffect(() => {
    previousKeysRef.current = stableKeys
    wasHydratedRef.current = transcriptHydrated
  }, [stableKeys, transcriptHydrated])

  return liveKeys
}

interface LiveTimelineRowProps {
  children: ReactNode
  messageRow: string
  animate: boolean
  staggerIndex?: number
  className?: string
  /** A transcript-level row: how the window measures it and finds its index. */
  rowRef?: Ref<HTMLDivElement>
  rowIndex?: number
  /** Sibling spacing, from the row model: the neighbor may not be mounted. */
  beforeGroup?: boolean
  afterProse?: boolean
  afterObserverFire?: boolean
}

function LiveTimelineRow({
  children,
  messageRow,
  animate,
  staggerIndex = 0,
  className,
  rowRef,
  rowIndex,
  beforeGroup,
  afterProse,
  afterObserverFire,
}: LiveTimelineRowProps) {
  const [present, setPresent] = useState(!animate)

  useLayoutEffect(() => {
    if (!animate) {
      setPresent(true)
      return
    }
    const frame = requestAnimationFrame(() => setPresent(true))
    return () => cancelAnimationFrame(frame)
  }, [animate])

  return (
    <div
      ref={rowRef}
      data-index={rowIndex}
      data-before-group={beforeGroup ? '' : undefined}
      data-after-prose={afterProse ? '' : undefined}
      data-after-observer-fire={afterObserverFire ? '' : undefined}
      data-message-row={messageRow}
      data-presence={present ? 'present' : 'entering'}
      className={cn('chat-live-row', className)}
      style={
        {
          '--chat-entry-delay': `${Math.min(staggerIndex, 4) * 40}ms`,
        } as CSSProperties
      }
    >
      {children}
    </div>
  )
}

/**
 * Pull `observer_id` out of a `harness::observe` output. Outputs arrive as
 * the `{ content: [{text}], details }` result envelope (see
 * functionResultOutput); the id lives in a JSON text block — bare
 * objects/strings are handled too.
 */
export function observerIdOf(output: unknown): string | null {
  return observeResultOf(output).observerId ?? null
}

function observeResultOf(output: unknown): {
  observerId?: string
  once?: boolean
  note?: string
} {
  const parseObserveResult = (
    v: unknown,
  ): { observerId?: string; once?: boolean; note?: string } => {
    if (typeof v === 'string') {
      try {
        return parseObserveResult(JSON.parse(v))
      } catch {
        return {}
      }
    }
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {}
    const record = v as Record<string, unknown>
    return {
      ...(typeof record.observer_id === 'string'
        ? { observerId: record.observer_id }
        : {}),
      ...(typeof record.once === 'boolean' ? { once: record.once } : {}),
      ...(typeof record.note === 'string' ? { note: record.note } : {}),
    }
  }
  const envelope = resultEnvelope(output)
  if (envelope) {
    for (const text of envelope.texts) {
      const result = parseObserveResult(text)
      if (result.observerId) return result
    }
    return parseObserveResult(envelope.details)
  }
  return parseObserveResult(output)
}

/**
 * Registration detail per message id, for observer-fired notices AND the
 * notification deliveries their notify-wakes produce. Two-tier per
 * observer: the harness's live/seen observer row when available, else
 * recovered from the transcript — the `harness::observe` call whose result
 * carries the same observer id (its input IS the registration).
 * Notifications correlate by the observer id carried by their trusted origin
 * or recovered from the current/legacy deterministic entry id, with a
 * name-match fallback only for older records. Resolution is order-independent:
 * an idle-session wake appends the notification before its fire record.
 */
export function resolveRegistrations(
  messages: MessageType[],
  observersById?: ReadonlyMap<string, SessionObserverInfo>,
): Map<string, ObserverRegistration> {
  const fromCalls = new Map<string, ObserverRegistration>()
  const notifyFireByName = new Map<string, string>()
  for (const m of messages) {
    if (m.role === 'function-call' && m.functionId === 'harness::observe') {
      const result = observeResultOf(m.output)
      const observerId = result.observerId
      if (observerId && m.input !== undefined) {
        fromCalls.set(
          observerId,
          registrationFromCall({
            id: m.id,
            input: m.input,
            observerId,
            effectiveOnce: result.once,
            note: result.note,
          }),
        )
      }
    } else if (
      m.role === 'system' &&
      m.kind === 'observer-fired' &&
      m.observer
    ) {
      const t = m.observer
      if (!t.target || t.target === 'notify' || t.target === 'harness::send')
        notifyFireByName.set(observerFiredName(t), t.observer_id)
    }
  }
  const regFor = (observerId: string): ObserverRegistration | undefined => {
    const row = observersById?.get(observerId)
    const fromCall = fromCalls.get(observerId)
    // A post-reload ghost row is reconstructed from the activity record. Even
    // when that record carries args and filters, it does not retain
    // registration-only conditions or lifecycle limits; the transcript call
    // is richer. The activity record still overlays its current source/state
    // in the card.
    if (row?.fired && fromCall) return fromCall
    if (row && (row.args !== undefined || row.filters !== undefined)) {
      return registrationFromRow(row)
    }
    return fromCall ?? (row ? registrationFromRow(row) : undefined)
  }
  const out = new Map<string, ObserverRegistration>()
  for (const m of messages) {
    if (m.role === 'system' && m.kind === 'observer-fired' && m.observer) {
      const t = m.observer
      const reg =
        regFor(t.observer_id) ??
        (t.function_id
          ? registrationFromCall({
              id: `observer-registration:${t.observer_id}`,
              observerId: t.observer_id,
              effectiveOnce: t.once,
              input: {
                function_id: t.function_id,
                args: t.args,
                filters: t.filters,
                label: t.label,
                action: t.action,
                target:
                  t.target &&
                  t.target !== 'notify' &&
                  t.target !== 'harness::send'
                    ? { function_id: t.target }
                    : undefined,
              },
            })
          : undefined)
      if (reg) out.set(m.id, reg)
    } else if (m.role === 'user' && m.notification) {
      const fromId = m.observerId ?? notificationBindingId(m.id)
      const name = /^\[notification\]\s*([^:]+):/.exec(m.content)?.[1]?.trim()
      const fromName = name ? notifyFireByName.get(name) : undefined
      // Some legacy `e_notify_*` ids append an ordinal that is impossible to
      // distinguish from an underscore inside the observer id. Trust an id
      // only when it resolves; otherwise retain the historical name match.
      const reg =
        (fromId ? regFor(fromId) : undefined) ??
        (fromName ? regFor(fromName) : undefined)
      if (reg) out.set(m.id, reg)
    }
  }
  return out
}

export function MessageList({
  messages,
  transcriptHydrated = true,
  isThinking,
  thinkingDetail,
  turnVisualPhase = 'idle',
  turnKey,
  density = 'route',
  header,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  onConfigureProvider,
  spawnContext,
  agentName,
  sessionId,
  workingDir,
  onWorkingDirChange,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  defaultOpenCalls,
  observersById,
  focusMessageId,
  onFocusMessageHandled,
  history,
  onLoadOlder,
  onLoadActivityEntries,
  onShowObservers,
}: MessageListProps) {
  const containerRef = useRef<HTMLElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const topSentinelRef = useRef<HTMLDivElement>(null)
  const didInitialScrollRef = useRef(false)
  const animationFrameRef = useRef<number | null>(null)
  const lastAnimationTimeRef = useRef<number | null>(null)
  const lastScrollTopRef = useRef(0)
  const lastTouchYRef = useRef<number | null>(null)
  // A mouse button held on the list: a scrollbar drag or a selection
  // auto-scroll is the one upward scroll that reaches handleScroll with no
  // wheel, touch or key event of its own.
  const mouseHeldRef = useRef(false)
  const tailStateRef = useRef<TailScrollState>('initializing')
  const [tailState, setTailState] = useState<TailScrollState>('initializing')
  // Any content image in the transcript opens in the viewer on click,
  // whichever renderer produced it; controls keep their own behaviour.
  const [zoomedImage, setZoomedImage] = useState<{
    src: string
    alt: string
  } | null>(null)

  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const reducedMotionRef = useRef(reducedMotion)
  reducedMotionRef.current = reducedMotion

  const settlingThoughtIds = useSettlingThoughtIds(messages, reducedMotion)

  const fcallsByAssistant = useMemo(
    () => functionCallsByAssistant(messages),
    [messages],
  )
  // A turn names its agent and model once, on its first reply.
  const headerIds = useMemo(
    () => assistantHeaderIds(messages, agentName),
    [messages, agentName],
  )
  const rows = useMemo(
    () =>
      groupTimelineActivities(
        observerActivityRows(functionCallGroups(messages, settlingThoughtIds)),
      ),
    [messages, settlingThoughtIds],
  )
  const presentationKeys = rows.flatMap((row) => {
    if (row.kind === 'activity-group') {
      return [
        ...row.items.map(timelineActivityPresentationKey),
        ...(row.summary ? [row.summary.id] : []),
      ]
    }
    if (
      row.message.role === 'thought' &&
      !row.message.streaming &&
      !settlingThoughtIds.has(row.message.id)
    ) {
      return []
    }
    return [row.message.id]
  })
  // One identity ledger spans top-level prose, grouped summaries, calls, and
  // observer activities. Reclassification never makes an existing surface look
  // newly inserted.
  const livePresentationKeys = useLiveEntryKeys(
    presentationKeys,
    transcriptHydrated,
  )
  const [waitingMounted, setWaitingMounted] = useState(Boolean(isThinking))
  if (isThinking && !waitingMounted) setWaitingMounted(true)
  const registrations = useMemo(
    () => resolveRegistrations(messages, observersById),
    [messages, observersById],
  )
  const newestPendingApprovalId = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index]
      if (
        message.role === 'function-call' &&
        message.pendingApproval === true
      ) {
        return message.id
      }
    }
    return null
  }, [messages])
  const hasPendingApproval = newestPendingApprovalId !== null
  // Same registry `FunctionCallCard` uses for its own raw pane: an
  // assistant-turn copy serializes each call's arguments the same way the
  // call's own card does, so a worker's `redactRaw` (e.g.
  // sandbox-code-runner's runtime_id) has to cover this exit too — see
  // function-call-copy.ts.
  const renderers = useFunctionCallRenderers()
  const redactFor = (functionId: string) => rawRedactor(renderers, functionId)

  // Read optionally so isolated renders (Storybook) still work without the
  // ConversationsProvider; the empty state falls back to `ready` there.
  const ctx = useConversationsCtxOptional()

  /* Windowing. Only the rows around the viewport are mounted, in normal flow,
     with a spacer standing in for each run of the others, so tail following
     and prepend restoration keep reading real DOM geometry. A row that leaves
     the window loses its local state, so a row the reader has clicked or
     focused stays mounted for good: that keeps every card's own state,
     including worker UI the host cannot lift, without the card knowing. Live
     rows, a landing target and the selected rows stay while they last. No
     ResizeObserver means no layout to window against (SSR, jsdom), so every
     row renders. */
  const windowed = typeof ResizeObserver !== 'undefined'
  const knownRowKeysRef = useRef<ReadonlySet<string>>(new Set())
  const flatRows = useMemo(
    () =>
      flattenTimelineRows(rows, settlingThoughtIds, knownRowKeysRef.current),
    [rows, settlingThoughtIds],
  )
  useLayoutEffect(() => {
    knownRowKeysRef.current = new Set(flatRows.map((flat) => flat.key))
  }, [flatRows])
  const rowIndexById = useMemo(() => rowIndexByEntryId(flatRows), [flatRows])
  // Placeholders a display renderer keeps visible in a folded batch (a
  // sub-agent card) are fetched when their page arrives, not when their row
  // mounts: a windowed row mounts as it scrolls in, and a card landing then
  // pushes the transcript down under the reader. Once per id; the store
  // dedupes reads still in flight, and opening a card is the retry.
  const requestedDisplayIdsRef = useRef(new Set<string>())
  useEffect(() => {
    if (!onLoadActivityEntries) return
    const display = new Map<string, boolean>()
    const isDisplay = (functionId: string) => {
      let claimed = display.get(functionId)
      if (claimed === undefined) {
        claimed = renderers.some(
          (renderer) =>
            renderer.isMatch(functionId) && renderer.metadata?.display === true,
        )
        display.set(functionId, claimed)
      }
      return claimed
    }
    const calls = flatRows.flatMap((flat) =>
      flat.kind === 'activity-group'
        ? flat.row.items.flatMap((item) =>
            item.kind === 'function-call' &&
            item.message.unloaded &&
            isDisplay(item.message.functionId)
              ? [item.message]
              : [],
          )
        : [],
    )
    const ids = unloadedEntryIds(calls).filter(
      (id) => !requestedDisplayIdsRef.current.has(id),
    )
    if (ids.length === 0) return
    for (const id of ids) requestedDisplayIdsRef.current.add(id)
    void onLoadActivityEntries(ids)
  }, [flatRows, renderers, onLoadActivityEntries])
  const [stickyRowKeys, setStickyRowKeys] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const pinRow = useCallback((key: string | undefined) => {
    if (!key) return
    setStickyRowKeys((current) =>
      current.has(key) ? current : new Set(current).add(key),
    )
  }, [])
  const pinInteractedRow = useCallback(
    (event: SyntheticEvent<HTMLElement>) => {
      pinRow(topLevelRowKey(event.target as Element))
    },
    [pinRow],
  )
  const selectedRows = useSelectedRowKeys(contentRef)
  const pinnedRowIndexes = useMemo(() => {
    const pinned = new Set<number>()
    flatRows.forEach((flat, index) => {
      if (isLiveRow(flat)) pinned.add(index)
    })
    for (const key of stickyRowKeys) {
      const index = rowIndexById.get(key)
      if (index !== undefined) pinned.add(index)
    }
    // Pinned in the commit the request arrives, so the landing effect below
    // finds its row in the DOM and a collapsed batch reveals it in that render.
    const focusIndex = focusMessageId
      ? rowIndexById.get(focusMessageId)
      : undefined
    if (focusIndex !== undefined) pinned.add(focusIndex)
    if (selectedRows) {
      const anchor = rowIndexById.get(selectedRows.anchor)
      const focus = rowIndexById.get(selectedRows.focus)
      if (anchor !== undefined && focus !== undefined) {
        const last = Math.max(anchor, focus)
        for (let index = Math.min(anchor, focus); index <= last; index++) {
          pinned.add(index)
        }
      }
    }
    return pinned
  }, [flatRows, focusMessageId, rowIndexById, selectedRows, stickyRowKeys])
  const prevFirstIdRef = useRef<string | undefined>(undefined)
  const renderedRowKeysRef = useRef<readonly string[]>([])
  // Only rows already mounted: one first mounted in this commit would add its
  // real height to the delta the prepend restoration reads, and then be
  // corrected a second time when it is measured. A short chat mounts everything
  // anyway, so it only needs this for a paused reader, the one who gets that
  // correction.
  const frozenRowIndexes =
    isHistoryPrepend(prevFirstIdRef.current, messages) &&
    (tailStateRef.current === 'paused' || flatRows.length > WINDOW_MIN_ROWS)
      ? flatRows.flatMap((flat, index) =>
          renderedRowKeysRef.current.includes(flat.key) ? [index] : [],
        )
      : null
  const getRowKey = useCallback(
    (index: number) => flatRows[index]?.key ?? index,
    [flatRows],
  )
  const virtualizer = useVirtualizer<HTMLElement, HTMLDivElement>({
    enabled: windowed,
    count: flatRows.length,
    getScrollElement: () => containerRef.current,
    getItemKey: getRowKey,
    estimateSize: estimateRowSlot,
    overscan: WINDOW_OVERSCAN_ROWS,
    initialRect: { width: 0, height: windowed ? window.innerHeight : 0 },
    observeElementRect: observeNonEmptyRect,
    measureElement: measureRowSlot,
    // Recorded so a correction is not read as the reader scrolling up.
    scrollToFn: (offset, { adjustments = 0 }) => {
      const container = containerRef.current
      if (!container) return
      container.scrollTop = offset + adjustments
      lastScrollTopRef.current = container.scrollTop
    },
    rangeExtractor: (range) =>
      windowedRowIndexes(range, {
        opening: tailState === 'initializing',
        frozen: frozenRowIndexes,
        pinned: pinnedRowIndexes,
      }),
  })
  // Keep what a paused reader sees still when a row above them changes height.
  // While following, the tail loop owns the offset. Live rects, because
  // `item.start` is stale until the next render while each correction in this
  // pass has already moved the offset. A row straddling the top edge only counts
  // on its first measurement: once known, its growth happens below the part in
  // view.
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (
    item,
    delta,
    instance,
  ) => {
    if (tailStateRef.current !== 'paused') return false
    const rect = instance.elementsCache.get(item.key)?.getBoundingClientRect()
    const top = instance.scrollElement?.getBoundingClientRect().top
    if (!rect || top === undefined) return false
    return instance.itemSizeCache.has(item.key)
      ? rect.bottom - delta <= top
      : rect.top < top
  }
  const virtualItems = windowed ? virtualizer.getVirtualItems() : null
  useLayoutEffect(() => {
    renderedRowKeysRef.current =
      virtualItems?.map((item) => String(item.key)) ?? []
  })
  // A hidden tab runs no ResizeObserver: a row growing there would leave the
  // window with the height it had when it mounted.
  useLayoutEffect(() => {
    if (!windowed || !document.hidden) return
    for (const node of virtualizer.elementsCache.values()) {
      if (!node.isConnected) continue
      virtualizer.resizeItem(
        virtualizer.indexFromElement(node),
        rowSlot(node, node.offsetHeight),
      )
    }
  })
  // What the reader chose for a batch, kept here to outlive its time in the
  // window. Nothing else is remembered: a batch left alone shows what a reload
  // would show.
  const expandedGroupsRef = useRef(new Map<string, boolean>())
  /** Whether the reader is in this element, so it must not change under them. */
  const readerIsIn = useCallback((element: HTMLElement | null) => {
    const container = containerRef.current
    if (!element || !container) return false
    if (element.contains(document.activeElement)) return true
    const selection = document.getSelection()
    if (selection && !selection.isCollapsed) {
      if (selection.getRangeAt(0).intersectsNode(element)) return true
    }
    if (tailStateRef.current !== 'paused') return false
    const box = container.getBoundingClientRect()
    const rect = element.getBoundingClientRect()
    return rect.bottom > box.top && rect.top < box.bottom
  }, [])

  const transitionTailState = useCallback((next: TailScrollState) => {
    if (tailStateRef.current === next) return
    tailStateRef.current = next
    setTailState(next)
  }, [])

  const cancelTailAnimation = useCallback(() => {
    if (animationFrameRef.current !== null) {
      cancelAnimationFrame(animationFrameRef.current)
      animationFrameRef.current = null
    }
    lastAnimationTimeRef.current = null
  }, [])

  /** Whether the write moved the window, so a commit with new rows is coming. */
  const writeScrollTop = useCallback(
    (container: HTMLElement, scrollTop: number) => {
      container.scrollTop = scrollTop
      // Programmatic scroll events arrive later. Recording the clamped value
      // now keeps them from looking like manual upward movement.
      lastScrollTopRef.current = container.scrollTop
      // The virtualizer would only hear of this from that later event, and
      // until then a row resize would write back from the stale offset. Told
      // now, it reads that event as a no-op, so the window moves from here.
      const before = virtualizer.range
      virtualizer.scrollOffset = container.scrollTop
      const after = virtualizer.calculateRange()
      const moved =
        after?.startIndex !== before?.startIndex ||
        after?.endIndex !== before?.endIndex
      if (moved) virtualizer.options.onChange?.(virtualizer, false)
      return moved
    },
    [virtualizer],
  )

  /**
   * Center a mounted row. A far or upward landing jumps: the smooth path would
   * cross rows mounting above it, each one correcting the offset and cancelling
   * the glide.
   */
  const landingRowRef = useRef<HTMLElement | null>(null)
  const scrollRowToCenter = useCallback(
    (container: HTMLElement, node: HTMLElement, settling = false) => {
      const containerRect = container.getBoundingClientRect()
      const nodeRect = node.getBoundingClientRect()
      const centeredTop =
        container.scrollTop +
        nodeRect.top -
        containerRect.top -
        (container.clientHeight - nodeRect.height) / 2
      const target = Math.max(
        0,
        Math.min(tailScrollTarget(container), centeredTop),
      )
      const jump =
        settling ||
        reducedMotionRef.current ||
        target < container.scrollTop ||
        target - container.scrollTop > container.clientHeight * 2
      if (!jump) {
        container.scrollTo({ top: target, behavior: 'smooth' })
      } else if (writeScrollTop(container, target) && !settling) {
        landingRowRef.current = node
      }
    },
    [writeScrollTop],
  )
  // The rows around a jump mount in the next commit, and a tall one above the
  // target pushes it down without counting as above the viewport.
  useLayoutEffect(() => {
    const node = landingRowRef.current
    const container = containerRef.current
    landingRowRef.current = null
    if (node && container) scrollRowToCenter(container, node, true)
  })

  /**
   * One retargetable animation follows the live tail. ResizeObserver callbacks
   * only ask for this loop; they never create competing smooth-scroll jobs.
   */
  const scheduleTailFollow = useCallback(() => {
    const container = containerRef.current
    if (!container || tailStateRef.current !== 'following') return

    const target = tailScrollTarget(container)
    if (reducedMotionRef.current) {
      cancelTailAnimation()
      writeScrollTop(container, target)
      return
    }
    if (animationFrameRef.current !== null) return

    const step = (time: number) => {
      animationFrameRef.current = null
      const currentContainer = containerRef.current
      if (!currentContainer || tailStateRef.current !== 'following') {
        lastAnimationTimeRef.current = null
        return
      }

      const nextTarget = tailScrollTarget(currentContainer)
      const current = currentContainer.scrollTop
      const remaining = nextTarget - current
      if (Math.abs(remaining) <= TAIL_GLIDE_SETTLE_DISTANCE_PX) {
        writeScrollTop(currentContainer, nextTarget)
        lastAnimationTimeRef.current = null
        return
      }

      const previousTime = lastAnimationTimeRef.current
      const elapsed = previousTime === null ? 1000 / 60 : time - previousTime
      lastAnimationTimeRef.current = time
      writeScrollTop(
        currentContainer,
        nextTailScrollTop(current, nextTarget, elapsed),
      )
      // The browser rounds the offset, so a step under a pixel lands nowhere:
      // the glide would spin short of the tail, every frame, until new content.
      if (currentContainer.scrollTop === current) {
        writeScrollTop(currentContainer, nextTarget)
        lastAnimationTimeRef.current = null
        return
      }
      animationFrameRef.current = requestAnimationFrame(step)
    }

    animationFrameRef.current = requestAnimationFrame(step)
  }, [cancelTailAnimation, writeScrollTop])

  /* Opening veil. From mount until the first page has landed AND its layout
     has gone quiet, the list is laid out and scrolled to the tail behind a
     `visibility: hidden` veil with a loading line in front. The loaded state
     a reader first sees must be final: no rows popping in, no glide chasing
     late layout (images, rich cards, fonts, lazy chips). Quiet means no
     content resize for OPEN_SETTLE_QUIET_MS; OPEN_SETTLE_MAX_MS caps the
     wait so a page that keeps growing (a live turn, slow images) still
     reveals. ChatView is keyed by conversation id, so every open starts
     veiled, including a cached transcript re-opened. */
  const [opening, setOpening] = useState(true)
  const openingRef = useRef(true)
  const [openingIndicator, setOpeningIndicator] = useState(false)
  const openQuietTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const transcriptHydratedRef = useRef(transcriptHydrated)
  transcriptHydratedRef.current = transcriptHydrated
  const finishOpening = useCallback(() => {
    if (!openingRef.current) return
    openingRef.current = false
    if (openQuietTimerRef.current !== null) {
      clearTimeout(openQuietTimerRef.current)
      openQuietTimerRef.current = null
    }
    setOpening(false)
  }, [])
  /* Every layout change while opening re-arms the quiet timer; the open ends
     when it fires. Before hydration there is nothing to settle yet. */
  const noteOpeningLayout = useCallback(() => {
    if (!openingRef.current || !transcriptHydratedRef.current) return
    if (openQuietTimerRef.current !== null) {
      clearTimeout(openQuietTimerRef.current)
    }
    openQuietTimerRef.current = setTimeout(finishOpening, OPEN_SETTLE_QUIET_MS)
  }, [finishOpening])
  useEffect(() => {
    if (!transcriptHydrated || !openingRef.current) return
    noteOpeningLayout()
    const cap = setTimeout(finishOpening, OPEN_SETTLE_MAX_MS)
    return () => clearTimeout(cap)
  }, [transcriptHydrated, noteOpeningLayout, finishOpening])
  useEffect(() => {
    if (!opening) return
    const timer = setTimeout(
      () => setOpeningIndicator(true),
      OPEN_INDICATOR_DELAY_MS,
    )
    return () => clearTimeout(timer)
  }, [opening])
  useEffect(
    () => () => {
      if (openQuietTimerRef.current !== null) {
        clearTimeout(openQuietTimerRef.current)
      }
    },
    [],
  )

  const hasScrollableContent =
    messages.length > 0 || Boolean(header) || waitingMounted

  /* Opening a session lands on the latest exchange before paint. ChatView is
     keyed by conversation id, so every open gets a fresh initializing state;
     the first hydrated content is always an instant jump, never a glide. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: message identity and thinking are deliberate pre-paint layout triggers while a transcript hydrates.
  useLayoutEffect(() => {
    if (didInitialScrollRef.current || !hasScrollableContent) return
    if (tailStateRef.current === 'paused') return
    const container = containerRef.current
    if (!container) return
    cancelTailAnimation()
    writeScrollTop(container, tailScrollTarget(container))
    if (!transcriptHydrated) return
    didInitialScrollRef.current = true
    transitionTailState('following')
  }, [
    cancelTailAnimation,
    hasScrollableContent,
    isThinking,
    messages,
    transcriptHydrated,
    transitionTailState,
    writeScrollTop,
  ])

  /* Content height can change without a messages identity change: markdown
     wraps, images load, results expand, the waiting indicator mounts, or the pane
     resizes. Observe both the content and viewport, coalescing all of it into
     the single follow loop. A paused reader is never moved. */
  useEffect(() => {
    if (!hasScrollableContent || typeof ResizeObserver === 'undefined') return
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return

    let previousTarget = tailScrollTarget(container)
    const observer = new ResizeObserver(() => {
      const target = tailScrollTarget(container)
      const growth = Math.max(0, target - previousTarget)
      previousTarget = target
      noteOpeningLayout()
      if (tailStateRef.current === 'paused') return
      if (!didInitialScrollRef.current || openingRef.current) {
        // Hydration and the veiled settle include late layout (images, rich
        // cards, fonts). Keep the tail pinned with direct writes: nothing
        // glides while nothing is visible, and the reveal shows the settled
        // bottom, never a scroll in progress.
        writeScrollTop(container, target)
        return
      }
      // Moving focus or find-in-page scrolls up with no gesture to pause on, and
      // then rows mounting there change the height. Far from the tail by more
      // than this growth, with no glide of ours running, the reader went there.
      if (
        animationFrameRef.current === null &&
        tailDistanceFromBottom(container) - growth > container.clientHeight
      ) {
        transitionTailState('paused')
        return
      }
      scheduleTailFollow()
    })
    observer.observe(container)
    observer.observe(content)
    return () => observer.disconnect()
  }, [
    hasScrollableContent,
    noteOpeningLayout,
    scheduleTailFollow,
    transitionTailState,
    writeScrollTop,
  ])

  useEffect(() => {
    if (reducedMotion && tailStateRef.current === 'following') {
      scheduleTailFollow()
    }
  }, [reducedMotion, scheduleTailFollow])

  /* Upward infinite scroll. The sentinel sits above the first message; while
     older history exists and none is loading, its entering the (margin-
     extended) viewport asks for the page above. Armed ONLY for a reader who
     has scrolled away from the tail (`paused`): opening a session lands at
     the bottom in `following`, and a short first page has the sentinel in
     view right there, so arming in that state pulled in page after page the
     reader never asked for. A page that is too short to scroll cannot be
     scrolled up either; the edge row offers the load as a button instead.
     Observed once per arming: the effect re-arms when `loadingOlder` clears,
     so a reader still heading up keeps getting pages ahead of them. A failed
     page does NOT re-arm (the row offers the retry) or a dead connection
     would loop request after request. */
  const hasOlderHistory = history?.hasMore === true
  const loadingOlder = history?.loadingOlder === true
  const olderHistoryError = history?.error
  const onLoadOlderRef = useRef(onLoadOlder)
  onLoadOlderRef.current = onLoadOlder
  useEffect(() => {
    if (
      !hasOlderHistory ||
      loadingOlder ||
      olderHistoryError ||
      !onLoadOlder ||
      !transcriptHydrated ||
      tailState !== 'paused' ||
      typeof IntersectionObserver === 'undefined'
    ) {
      return
    }
    const container = containerRef.current
    const sentinel = topSentinelRef.current
    if (!container || !sentinel) return
    let requested = false
    const observer = new IntersectionObserver(
      (entries) => {
        if (requested || !entries.some((entry) => entry.isIntersecting)) return
        requested = true
        onLoadOlderRef.current?.()
      },
      { root: container, rootMargin: OLDER_PAGE_ROOT_MARGIN },
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [
    hasOlderHistory,
    loadingOlder,
    olderHistoryError,
    onLoadOlder,
    tailState,
    transcriptHydrated,
  ])

  /* Prepend scroll restoration. A page landing above the fold pushes
     everything down by its height; the reader must not notice. The height
     recorded at the last commit (and refreshed by the resize observer, since
     images and cards grow without a re-render) is what the container
     measured before this commit; the difference in scrollHeight is exactly
     how far the content moved. The BASE is the container's live scrollTop,
     not the recorded one: a paused reader keeps scrolling while the page is
     in flight and no re-render records that, and with scroll anchoring off
     the live value is still where the reader is at this instant. Written
     through `writeScrollTop` so the resulting scroll event is not mistaken
     for a manual gesture. Applied in EVERY tail state, not just `paused`:
     for a reader at the bottom the shifted offset IS the new bottom (the
     browser clamps it there), so the follow loop finds nothing to glide. It
     cannot be left to the follow loop: the observer re-arms in this same
     commit, and with the offset still at its old value the viewport sits on
     the TOP of the page that just landed, sentinel in view, so the next page
     is requested before the glide has moved a pixel, and the whole history
     cascades in on a short first page. `didInitialScrollRef` is left alone,
     this is not the open. */
  const prevMetricsRef = useRef<PrependScrollMetrics | null>(null)
  const recordScrollMetrics = useCallback((container: HTMLElement) => {
    prevMetricsRef.current = {
      scrollTop: container.scrollTop,
      scrollHeight: container.scrollHeight,
    }
  }, [])
  useLayoutEffect(() => {
    const container = containerRef.current
    const firstId = messages[0]?.id
    const prevFirstId = prevFirstIdRef.current
    prevFirstIdRef.current = firstId
    if (!container) return
    const previous = prevMetricsRef.current
    if (previous !== null && isHistoryPrepend(prevFirstId, messages)) {
      writeScrollTop(
        container,
        scrollTopAfterPrepend(
          {
            scrollTop: container.scrollTop,
            scrollHeight: previous.scrollHeight,
          },
          container.scrollHeight,
        ),
      )
    }
    recordScrollMetrics(container)
  }, [messages, recordScrollMetrics, writeScrollTop])
  useEffect(() => {
    if (typeof ResizeObserver === 'undefined') return
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return
    const observer = new ResizeObserver(() => recordScrollMetrics(container))
    observer.observe(content)
    return () => observer.disconnect()
  }, [recordScrollMetrics])

  useEffect(() => cancelTailAnimation, [cancelTailAnimation])

  const pauseTailFollow = useCallback(() => {
    const container = containerRef.current
    if (
      !container ||
      container.scrollHeight <= container.clientHeight + TAIL_REARM_DISTANCE_PX
    ) {
      return
    }
    cancelTailAnimation()
    transitionTailState('paused')
  }, [cancelTailAnimation, transitionTailState])

  const handleScroll = useCallback(
    (event: UIEvent<HTMLElement>) => {
      const container = event.currentTarget
      const currentScrollTop = container.scrollTop
      const nextState = tailStateAfterScroll(
        tailStateRef.current,
        lastScrollTopRef.current,
        container,
        mouseHeldRef.current,
      )
      if (nextState === 'paused') cancelTailAnimation()
      transitionTailState(nextState)
      lastScrollTopRef.current = currentScrollTop
      // A prepend restores against the offset the reader was at, not the
      // one at the last commit.
      recordScrollMetrics(container)
    },
    [cancelTailAnimation, recordScrollMetrics, transitionTailState],
  )

  const handleWheel = useCallback(
    (event: WheelEvent<HTMLElement>) => {
      if (event.deltaY < 0) pauseTailFollow()
    },
    [pauseTailFollow],
  )

  const handleMouseDown = useCallback(() => {
    mouseHeldRef.current = true
    window.addEventListener(
      'mouseup',
      () => {
        mouseHeldRef.current = false
      },
      { once: true },
    )
  }, [])

  const handleTouchStart = useCallback((event: TouchEvent<HTMLElement>) => {
    lastTouchYRef.current = event.touches[0]?.clientY ?? null
  }, [])

  const handleTouchMove = useCallback(
    (event: TouchEvent<HTMLElement>) => {
      const nextY = event.touches[0]?.clientY
      const previousY = lastTouchYRef.current
      if (nextY !== undefined && previousY !== null && nextY > previousY) {
        pauseTailFollow()
      }
      lastTouchYRef.current = nextY ?? null
    },
    [pauseTailFollow],
  )

  const clearTouchPosition = useCallback(() => {
    lastTouchYRef.current = null
  }, [])

  const handleJumpToLatest = useCallback(() => {
    didInitialScrollRef.current = true
    transitionTailState('following')
    const container = containerRef.current
    container?.focus({ preventScroll: true })
    // A far jump skips the rows in between instead of mounting each on the way.
    if (
      container &&
      tailDistanceFromBottom(container) > container.clientHeight * 2
    ) {
      writeScrollTop(container, tailScrollTarget(container))
    }
    scheduleTailFollow()
  }, [scheduleTailFollow, transitionTailState, writeScrollTop])

  const handleJumpToAction = useCallback(() => {
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content || !newestPendingApprovalId) {
      handleJumpToLatest()
      return
    }

    const approval = Array.from(
      content.querySelectorAll<HTMLElement>('[data-message-id]'),
    ).find((node) => node.dataset.messageId === newestPendingApprovalId)
    if (!approval) {
      handleJumpToLatest()
      return
    }

    cancelTailAnimation()
    // The scroll up to the card is this button's doing; the scroll handler
    // no longer reads direction, so pause here. Landing on the tail (a card
    // at the very end) re-arms following from the scroll event itself.
    transitionTailState('paused')
    scrollRowToCenter(container, approval)

    const action = approval.querySelector<HTMLElement>(
      '[data-approval-actions] button:not([disabled])',
    )
    const focusTarget = action ?? approval
    focusTarget.focus({ preventScroll: true })
  }, [
    cancelTailAnimation,
    handleJumpToLatest,
    newestPendingApprovalId,
    scrollRowToCenter,
    transitionTailState,
  ])

  /* External landing (trace → "go to message"): center the requested row
     once it exists, then hand the request back to the owner. Tail following
     is paused first so a live tail can't yank the view back down; the flash
     gives the jump a visible landmark. Gated on hydration so it never
     centers against a partial history. */
  const focusAppliedRef = useRef<string | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: message arrival is the retry trigger while the target row hasn't rendered yet.
  useEffect(() => {
    if (!focusMessageId) {
      focusAppliedRef.current = null
      return
    }
    if (!transcriptHydrated || focusAppliedRef.current === focusMessageId) {
      return
    }
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return
    // `data-message-row` is the transcript-row identity (top-level rows,
    // group items, group summaries) — not `data-message-id`, which is the
    // card-level attribute the approval jump uses. A row that absorbed a
    // wake notification carries both entry ids, space-separated.
    const node = Array.from(
      content.querySelectorAll<HTMLElement>('[data-message-row]'),
    ).find((el) => el.dataset.messageRow?.split(' ').includes(focusMessageId))
    if (!node) return
    focusAppliedRef.current = focusMessageId
    cancelTailAnimation()
    didInitialScrollRef.current = true
    transitionTailState('paused')
    // The request is handed back below; the row must outlive it to be landed on.
    pinRow(topLevelRowKey(node))
    scrollRowToCenter(container, node)
    if (typeof node.animate === 'function') {
      node.animate(
        [
          {
            backgroundColor: 'color-mix(in srgb, currentColor 5%, transparent)',
          },
          { backgroundColor: 'transparent' },
        ],
        { duration: 900, easing: 'ease-out' },
      )
    }
    onFocusMessageHandled?.()
  }, [
    focusMessageId,
    transcriptHydrated,
    messages,
    onFocusMessageHandled,
    cancelTailAnimation,
    pinRow,
    scrollRowToCenter,
    transitionTailState,
  ])

  if (messages.length === 0 && !header && !isThinking && transcriptHydrated) {
    return (
      <EmptyState
        {...resolveEmptyState(ctx, density, onConfigureProvider, {
          workingDir,
          onWorkingDirChange,
          workingDirError,
          defaultWorkingDir,
          worktreePicker,
        })}
      />
    )
  }

  const listPad =
    density === 'dock'
      ? 'px-3 py-5 @2xl:px-4 @2xl:py-6'
      : 'px-3 py-5 @2xl:px-6 @2xl:py-7 @5xl:px-9 @5xl:py-8'

  const measureRow = windowed ? virtualizer.measureElement : undefined
  const renderFlatRow = (flat: FlatTimelineRow, index: number) => {
    const beforeGroup =
      isProseRow(flat) && flatRows[index + 1]?.kind === 'activity-group'
    if (flat.kind === 'activity-group') {
      return (
        <LiveTimelineRow
          key={flat.key}
          rowRef={measureRow}
          rowIndex={index}
          afterProse={isProseRow(flatRows[index - 1])}
          messageRow={flat.key}
          // Activity children own their stagger; animating the whole
          // group too would double-transform rich worker surfaces.
          animate={false}
          staggerIndex={flat.staggerIndex}
        >
          <FunctionCallGroup
            row={flat.row}
            renderers={renderers}
            registrations={registrations}
            reducedMotion={reducedMotion}
            livePhaseChildKeys={livePresentationKeys}
            groupKey={flat.key}
            expandedGroups={expandedGroupsRef.current}
            agentMovedOn={agentMovedPast(
              flatRows,
              index,
              turnVisualPhase !== 'idle',
            )}
            readerIsIn={readerIsIn}
            defaultOpenCalls={defaultOpenCalls}
            focusMessageId={focusMessageId}
            onLoadActivityEntries={onLoadActivityEntries}
            onShowObservers={onShowObservers}
            onResolveApproval={onResolveApproval}
            onAlwaysAllow={onAlwaysAllow}
            onResolveFilesystemAccess={onResolveFilesystemAccess}
            onManageFilesystemAccess={onManageFilesystemAccess}
            workingDir={workingDir}
            agentName={agentName}
          />
        </LiveTimelineRow>
      )
    }

    if (flat.kind === 'summary') {
      const summary = flat.message
      const calls = fcallsByAssistant.get(summary.id)
      const copyText =
        summary.content || calls?.length
          ? () => assistantCopyText(summary.content, calls ?? [], redactFor)
          : undefined
      return (
        <LiveTimelineRow
          key={flat.key}
          rowRef={measureRow}
          rowIndex={index}
          beforeGroup={beforeGroup}
          messageRow={flat.key}
          animate={!reducedMotion && livePresentationKeys.has(summary.id)}
          staggerIndex={flat.staggerIndex}
        >
          <Message
            message={summary}
            copyText={copyText}
            agentName={agentName}
            showHeader={headerIds.has(summary.id)}
            turnDurationMs={turnDurationMs(messages, summary.id)}
            changedFiles={changedFilesFromCalls(calls ?? [])}
            revealOnMount={
              !reducedMotion && livePresentationKeys.has(summary.id)
            }
          />
        </LiveTimelineRow>
      )
    }

    const m = flat.message
    // Assistant turns copy their prose plus the calls that follow them;
    // the thunk defers building that string until the copy click. Left
    // undefined when the turn has nothing to copy (no prose, no calls)
    // so the header shows no copy affordance.
    const calls =
      m.role === 'assistant' ? fcallsByAssistant.get(m.id) : undefined
    const copyText =
      m.role === 'assistant' && (m.content || calls?.length)
        ? () => assistantCopyText(m.content, calls ?? [], redactFor)
        : undefined
    return (
      <LiveTimelineRow
        key={flat.key}
        rowRef={measureRow}
        rowIndex={index}
        beforeGroup={beforeGroup}
        afterObserverFire={
          isObserverFireRow(flat) && isObserverFireRow(flatRows[index - 1])
        }
        messageRow={flat.key}
        animate={
          !reducedMotion &&
          livePresentationKeys.has(m.id) &&
          // These live surfaces own their internal reveal; wrapping
          // them in a second translate/fade would compound motion.
          m.role !== 'thought' &&
          !(m.role === 'assistant' && m.streaming)
        }
        staggerIndex={flat.staggerIndex}
        className={cn(
          m.role === 'thought' && 'chat-thought-row',
          m.role === 'thought' && !m.streaming && 'chat-thought-row-settling',
        )}
      >
        <Message
          message={m}
          spawnContext={spawnContext}
          agentName={agentName}
          showHeader={m.role === 'assistant' ? headerIds.has(m.id) : undefined}
          sessionId={sessionId}
          copyText={copyText}
          turnDurationMs={
            m.role === 'assistant' ? turnDurationMs(messages, m.id) : null
          }
          changedFiles={
            m.role === 'assistant'
              ? changedFilesFromCalls(calls ?? [])
              : undefined
          }
          revealOnMount={
            !reducedMotion &&
            m.role === 'assistant' &&
            livePresentationKeys.has(m.id)
          }
          defaultOpenCalls={defaultOpenCalls}
          onResolveApproval={onResolveApproval}
          onAlwaysAllow={onAlwaysAllow}
          onResolveFilesystemAccess={onResolveFilesystemAccess}
          onManageFilesystemAccess={onManageFilesystemAccess}
          workingDir={workingDir}
          registration={registrations.get(m.id)}
        />
      </LiveTimelineRow>
    )
  }

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1">
      <ImageViewer
        open={zoomedImage !== null}
        onOpenChange={(next) => {
          if (!next) setZoomedImage(null)
        }}
        src={zoomedImage?.src}
        alt={zoomedImage?.alt ?? 'image'}
      />
      <section
        ref={containerRef}
        data-message-list=""
        data-tail-scroll={tailState}
        data-turn-phase={turnVisualPhase}
        aria-label="conversation messages"
        tabIndex={-1}
        data-transcript-opening={opening ? '' : undefined}
        className={cn(
          // Scroll anchoring is off: the prepend restoration above is the one
          // adjustment, and it must not add to the browser's own.
          'conversation-transcript min-h-0 min-w-0 flex-1 overflow-y-auto [overflow-anchor:none] focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-accent',
          // Opacity, not display or visibility: the veiled list keeps its
          // layout so the tail jump and the settle happen for real behind it,
          // and opacity composites the whole subtree, whereas `visibility:
          // hidden` is only inherited: the status glyph/copy layers switch
          // themselves to `visibility: visible` and showed through.
          opening && 'pointer-events-none opacity-0',
          listPad,
        )}
        onScroll={handleScroll}
        onClickCapture={pinInteractedRow}
        onFocusCapture={pinInteractedRow}
        onClick={(event) => {
          if (event.defaultPrevented) return
          const zoom = imageZoomTarget(event.target)
          if (zoom) setZoomedImage(zoom)
        }}
        onKeyDown={(event) => {
          if (event.defaultPrevented) return
          // The keys the browser scrolls a focused list up with.
          if (
            event.key === 'ArrowUp' ||
            event.key === 'PageUp' ||
            event.key === 'Home' ||
            (event.key === ' ' && event.shiftKey)
          ) {
            pauseTailFollow()
            return
          }
          if (event.key !== 'Enter' && event.key !== ' ') return
          const zoom = imageZoomTarget(event.target)
          if (!zoom) return
          event.preventDefault()
          setZoomedImage(zoom)
        }}
        onWheel={handleWheel}
        onMouseDown={handleMouseDown}
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        onTouchEnd={clearTouchPosition}
        onTouchCancel={clearTouchPosition}
      >
        <div
          ref={contentRef}
          className="chat-message-stack mx-auto flex max-w-[720px] flex-col gap-y-6 @2xl:gap-y-8"
        >
          {header}
          {history && messages.length > 0 ? (
            <HistoryEdgeRow
              ref={topSentinelRef}
              history={history}
              onLoadOlder={onLoadOlder}
            />
          ) : null}
          {virtualItems
            ? rowsWithSpacers(
                virtualItems,
                virtualizer.getTotalSize(),
                (index) => renderFlatRow(flatRows[index], index),
                (key, slot) => <TranscriptSpacer key={key} slot={slot} />,
              )
            : flatRows.map(renderFlatRow)}
          {waitingMounted ? (
            <div
              className="chat-waiting-slot"
              data-active={Boolean(isThinking)}
              aria-hidden={!isThinking}
              inert={!isThinking ? true : undefined}
            >
              <div className="chat-waiting-slot-inner">
                <ModelWaitingIndicator
                  label={thinkingDetail}
                  active={Boolean(isThinking)}
                  turnKey={turnKey}
                />
              </div>
            </div>
          ) : null}
        </div>
      </section>
      {opening ? (
        // The same skeleton as ChatPanel's "conversation not yet loaded"
        // state, so the two waits read as one to the reader.
        <div
          data-transcript-loading=""
          aria-live="polite"
          className={cn(
            'pointer-events-none absolute inset-0 overflow-hidden transition-opacity duration-[var(--motion-duration-control)]',
            openingIndicator ? 'opacity-100' : 'opacity-0',
          )}
        >
          <ConversationSkeleton />
        </div>
      ) : null}
      {tailState === 'paused' ? (
        <Button
          type="button"
          variant="pill"
          size="sm"
          className={cn(
            'iii-ui-motion-overlay absolute bottom-3 left-1/2 z-10 h-9 -translate-x-1/2 rounded-full bg-panel-raised/80 pr-3 pl-2.5 font-medium text-base shadow-raised backdrop-blur-md sm:h-8 sm:text-[0.8125rem] dark:bg-black/80 dark:ring-1 dark:ring-neutral-900',
            hasPendingApproval && 'text-warn',
          )}
          onClick={hasPendingApproval ? handleJumpToAction : handleJumpToLatest}
          aria-label={
            hasPendingApproval
              ? 'jump to action required'
              : 'jump to latest message'
          }
        >
          <span
            className="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <ArrowDown aria-hidden="true" className="stroke-current" />
          <span>{hasPendingApproval ? 'action required' : 'latest'}</span>
        </Button>
      ) : null}
    </div>
  )
}

/** Stands in for unmounted rows; a flex child, so it brings one gap itself. */
function TranscriptSpacer({ slot }: { slot: number }) {
  return (
    <div
      aria-hidden="true"
      data-transcript-spacer=""
      style={{ height: `max(0px, calc(${slot}px - var(--chat-stack-gap)))` }}
    />
  )
}

/**
 * The row above the first message: the top edge of the loaded window. It
 * keeps one fixed height whatever it says, so switching from "loading" to a
 * loaded page does not itself move the content the prepend restoration is
 * measuring. It also carries the infinite-scroll sentinel (the forwarded
 * ref), which therefore sits exactly at the edge of what is loaded.
 */
const HistoryEdgeRow = forwardRef<
  HTMLDivElement,
  {
    history: NonNullable<Conversation['history']>
    onLoadOlder?: () => void
  }
>(function HistoryEdgeRow({ history, onLoadOlder }, ref) {
  const state = history.loadingOlder
    ? 'loading'
    : history.error
      ? 'error'
      : history.hasMore
        ? 'more'
        : 'beginning'
  return (
    <div
      ref={ref}
      data-history-edge={state}
      className="flex h-6 items-center justify-center font-mono text-[11px] text-ink-ghost"
      aria-live="polite"
    >
      {state === 'loading' ? (
        <span className="animate-pulse">loading earlier messages…</span>
      ) : state === 'error' ? (
        <span className="flex items-center gap-2">
          <span className="text-warn">earlier messages failed to load</span>
          {onLoadOlder ? (
            <button
              type="button"
              onClick={onLoadOlder}
              className={HISTORY_EDGE_BUTTON_CLASS}
            >
              retry
            </button>
          ) : null}
        </span>
      ) : state === 'beginning' ? (
        <span>beginning of conversation</span>
      ) : onLoadOlder ? (
        // The explicit path to older history: the only one when the page is
        // too short to scroll, and a visible promise that nothing loads on
        // its own while the reader sits at the tail.
        <button
          type="button"
          onClick={onLoadOlder}
          className={HISTORY_EDGE_BUTTON_CLASS}
        >
          load earlier messages
        </button>
      ) : null}
    </div>
  )
})

interface FunctionCallGroupProps {
  row: TimelineActivityGroupRow
  renderers: ReturnType<typeof useFunctionCallRenderers>
  registrations: ReadonlyMap<string, ObserverRegistration>
  reducedMotion: boolean
  livePhaseChildKeys: ReadonlySet<string>
  /** This row's key in the window, stable as older calls merge into it. */
  groupKey: string
  /** The reader's choices by group key, outliving this group's mount. */
  expandedGroups: Map<string, boolean>
  /** The turn ended, or something other than a thought came after this batch. */
  agentMovedOn: boolean
  readerIsIn: (element: HTMLElement | null) => boolean
  defaultOpenCalls?: boolean
  onResolveApproval?: MessageListProps['onResolveApproval']
  onAlwaysAllow?: MessageListProps['onAlwaysAllow']
  onResolveFilesystemAccess?: MessageListProps['onResolveFilesystemAccess']
  onManageFilesystemAccess?: MessageListProps['onManageFilesystemAccess']
  workingDir?: string | null
  agentName?: string
  /** External landing target — a hidden matching item expands the group. */
  focusMessageId?: string | null
  onLoadActivityEntries?: MessageListProps['onLoadActivityEntries']
  onShowObservers?: () => void
}

function useDwelledActivityKeys(
  desiredKeys: ReadonlySet<string>,
  reducedMotion: boolean,
): ReadonlySet<string> {
  const signature = `${reducedMotion ? 'reduced' : 'motion'}\u0000${[
    ...desiredKeys,
  ].join('\u0000')}`
  const [presenceState, setPresenceState] = useState(() => ({
    desiredKeys: new Set(desiredKeys) as ReadonlySet<string>,
    retainedKeys: new Set(desiredKeys) as ReadonlySet<string>,
    signature,
  }))
  const timersRef = useRef(new Map<string, number>())
  let renderedPresenceState = presenceState

  if (presenceState.signature !== signature) {
    renderedPresenceState = {
      desiredKeys: new Set(desiredKeys),
      retainedKeys: reducedMotion
        ? new Set(desiredKeys)
        : new Set([...presenceState.retainedKeys, ...desiredKeys]),
      signature,
    }
    setPresenceState(renderedPresenceState)
  }

  useEffect(() => {
    if (reducedMotion) {
      for (const timer of timersRef.current.values()) window.clearTimeout(timer)
      timersRef.current.clear()
      return
    }

    for (const key of renderedPresenceState.desiredKeys) {
      const timer = timersRef.current.get(key)
      if (timer === undefined) continue
      window.clearTimeout(timer)
      timersRef.current.delete(key)
    }
    for (const key of renderedPresenceState.retainedKeys) {
      if (
        renderedPresenceState.desiredKeys.has(key) ||
        timersRef.current.has(key)
      ) {
        continue
      }
      const timer = window.setTimeout(() => {
        timersRef.current.delete(key)
        setPresenceState((current) => {
          if (current.desiredKeys.has(key) || !current.retainedKeys.has(key)) {
            return current
          }
          const next = new Set(current.retainedKeys)
          next.delete(key)
          return { ...current, retainedKeys: next }
        })
      }, ACTIVITY_RESULT_DWELL_MS)
      timersRef.current.set(key, timer)
    }
  }, [reducedMotion, renderedPresenceState])

  useEffect(
    () => () => {
      for (const timer of timersRef.current.values()) window.clearTimeout(timer)
      timersRef.current.clear()
    },
    [],
  )

  return renderedPresenceState.retainedKeys
}

/**
 * A sequence of agent calls reads as one phase: one card whose header counts
 * the tools and whose rows are the calls. A batch seen live opens in full
 * until the agent moves past it; a settled batch keeps the latest call plus any
 * rich-display, approval, or live call visible, and the header chevron restores
 * every call in chronological order. The assistant's intermediate prose stays normal prose
 * after the card and doubles as the batch summary.
 */
function FunctionCallGroup({
  row,
  renderers,
  registrations,
  reducedMotion,
  livePhaseChildKeys,
  groupKey,
  expandedGroups,
  agentMovedOn,
  readerIsIn,
  defaultOpenCalls,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  agentName,
  focusMessageId,
  onLoadActivityEntries,
  onShowObservers,
}: FunctionCallGroupProps) {
  // A batch observed live opens in full: its rows arrive one by one and the
  // reader watches them settle. Once the agent moves past it, and for a settled
  // batch from the transcript, it folds to its latest call (plus anything a
  // renderer keeps on display) behind the header, where "Called 11 functions"
  // and a chevron say what is there.
  const live = row.items.some(
    (item) =>
      item.kind === 'function-call' &&
      (item.message.running === true || item.message.pendingApproval === true),
  )
  // A phase of nothing but observer activity counts what it observed; one
  // with calls in it says what it called.
  const observedOnly = row.items.every(
    (item) => item.kind === 'observer-activity',
  )
  const noun = observedOnly
    ? row.items.length === 1
      ? 'event'
      : 'events'
    : row.items.length === 1
      ? 'function'
      : 'functions'
  const failedCount = row.items.filter(
    (item) =>
      item.kind === 'function-call' &&
      !item.message.running &&
      !item.message.pendingApproval &&
      isErrorOutput(item.message.output),
  ).length
  // Mounting settled means the window brought it back or history landed:
  // nothing in it is arriving, so nothing in it animates in.
  const [mountedSettled] = useState(!live)
  const [expanded, setExpanded] = useState(
    () => expandedGroups.get(groupKey) ?? (!!defaultOpenCalls || live),
  )
  // A batch that ran live folds when it is over, as a reload would show it,
  // unless the reader chose or is in it. Read once, when it finishes. One that
  // mounted settled has nothing to fold: if open, a landing just revealed it.
  const sectionRef = useRef<HTMLElement>(null)
  const over = !live && agentMovedOn
  // biome-ignore lint/correctness/useExhaustiveDependencies: fires when the batch finishes; the rest is read at that moment.
  useLayoutEffect(() => {
    if (!over || mountedSettled || defaultOpenCalls) return
    if (expandedGroups.has(groupKey)) return
    if (!readerIsIn(sectionRef.current)) setExpanded(false)
  }, [over])
  const contentId = useId()
  // A renderer that keeps its calls visible while the phase is collapsed
  // (`metadata.display`) is decided by function id alone for a placeholder:
  // there is nothing to `tryRender` yet, and the call must be fetched anyway
  // so the display can draw it.
  const hasDisplayRenderer = (functionId: string) =>
    renderers.some(
      (renderer) =>
        renderer.isMatch(functionId) && renderer.metadata?.display === true,
    )
  const collapsedItems = collapsedTimelineActivities(row.items, (call) => {
    // Observe receipts stay visible as durable activity receipts without
    // opting the engine renderer into the richer `display` presentation.
    if (call.functionId === 'harness::observe') return true
    if (call.unloaded) return hasDisplayRenderer(call.functionId)

    const rendered = firstRendered(renderers, (renderer) => {
      if (!renderer.isMatch(call.functionId) || call.pendingApproval)
        return null
      return call.running
        ? (renderer.tryRenderRunning ?? renderer.tryRender)(call)
        : renderer.tryRender(call)
    })
    return rendered?.renderer.metadata?.display === true
  })
  const collapsedKeyValues = collapsedItems.map(timelineActivityPresentationKey)
  const collapsedKeySignature = collapsedKeyValues.join('\u0000')
  // biome-ignore lint/correctness/useExhaustiveDependencies: the primitive signature is the set's semantic identity.
  const collapsedKeys = useMemo(
    () => new Set(collapsedKeyValues),
    [collapsedKeySignature],
  )
  // External landing (trace → "go to message"): a target hidden behind the
  // collapse must have a DOM row in the same render the request resolves, so
  // the reveal happens here, via the render-phase setState pattern. Latched
  // through `expanded` — not derived — so consuming the request doesn't
  // re-collapse the revealed row. An absorbed wake notification is this
  // row's transcript entry too, so it counts as a match.
  const ownsFocusTarget = (item: (typeof row.items)[number]) =>
    item.message.id === focusMessageId ||
    (item.kind === 'observer-activity' &&
      item.notification?.id === focusMessageId)
  // A run of observer events shows as an eye and a count that opens the side
  // panel. One event shows as itself: its text is the news.
  const rollsUp = observedOnly && !!onShowObservers && row.items.length > 1
  if (
    !expanded &&
    focusMessageId != null &&
    (rollsUp || !collapsedItems.some(ownsFocusTarget)) &&
    row.items.some(ownsFocusTarget)
  ) {
    // A landing needs the target's row on screen: the roll-up has none.
    setExpanded(true)
  }
  const hiddenCount = row.items.length - collapsedItems.length
  const canCollapse = hiddenCount > 0
  // Placeholders the collapsed view shows anyway are fetched by MessageList
  // when their page arrives. The rest wait for their card to open or "show
  // all"; the store dedupes ids still in flight.
  const unloadedCalls = row.items.flatMap((item) =>
    item.kind === 'function-call' && item.message.unloaded
      ? [item.message]
      : [],
  )
  const requestedEntryIdsRef = useRef(new Set<string>())
  // `retry` bypasses this group's once-per-id set: "show all" and a card
  // opening are explicit asks, and a placeholder still standing after an
  // earlier request means that fetch failed, so asking again is the retry.
  // The store dedupes ids still in flight, so a double click costs nothing.
  const requestEntries = (
    calls: readonly FunctionCallMessage[],
    retry = false,
  ) => {
    if (!onLoadActivityEntries) return
    const ids = unloadedEntryIds(calls).filter(
      (id) => retry || !requestedEntryIdsRef.current.has(id),
    )
    if (ids.length === 0) return
    for (const id of ids) requestedEntryIdsRef.current.add(id)
    return onLoadActivityEntries(ids)
  }
  const toggleExpanded = () => {
    if (!expanded) requestEntries(unloadedCalls, true)
    expandedGroups.set(groupKey, !expanded)
    setExpanded(!expanded)
  }
  const presentedKeyValues = expanded
    ? row.items.map(timelineActivityPresentationKey)
    : collapsedKeyValues
  const presentedKeySignature = presentedKeyValues.join('\u0000')
  // Track expanded rows as desired presence too. When “show latest” is
  // clicked, they then remain mounted for the closing track instead of
  // disappearing before their 1fr → 0fr transition can paint.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the primitive signature is the set's semantic identity.
  const presentedKeys = useMemo(
    () => new Set(presentedKeyValues),
    [presentedKeySignature],
  )
  const retainedKeys = useDwelledActivityKeys(presentedKeys, reducedMotion)
  let visibleOrdinal = 0
  const presentedItems = row.items.map((item, index) => {
    const key = timelineActivityPresentationKey(item)
    const semanticVisible = expanded || collapsedKeys.has(key)
    const mounted = semanticVisible || retainedKeys.has(key)
    const ordinal = semanticVisible ? visibleOrdinal++ : -1
    return { item, index, key, semanticVisible, mounted, ordinal }
  })

  if (rollsUp && !expanded) {
    const label = `${row.items.length} observer ${noun}`
    // Which observers the run came from, most recent first: two names, then
    // a count, so a run from several observers says so.
    const names: string[] = []
    for (let i = row.items.length - 1; i >= 0; i--) {
      const fired = row.items[i].message
      const name =
        fired.role === 'system'
          ? (fired.observer?.label ?? fired.observer?.action)
          : undefined
      if (name && !names.includes(name)) names.push(name)
    }
    const sources =
      names.length > 2
        ? `${names.slice(0, 2).join(', ')} +${names.length - 2}`
        : names.join(', ')
    return (
      <section ref={sectionRef} data-observer-rollup="">
        <Hint label="Show in Observers">
          <button
            type="button"
            onClick={onShowObservers}
            aria-label={`${label}${sources ? ` from ${sources}` : ''} · show in Observers`}
            className="-ml-2 inline-flex h-8 max-w-full items-center gap-2 rounded-md px-2 font-sans text-sm text-ink-faint transition-colors hover:bg-surface hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          >
            <ObserverIcon aria-hidden className="size-4 shrink-0 fill-warn" />
            <span className="font-medium text-ink tabular-nums">
              {row.items.length}
            </span>
            <span>{noun}</span>
            {sources ? (
              <span className="min-w-0 truncate" data-observer-sources="">
                · {sources}
              </span>
            ) : null}
          </button>
        </Hint>
      </section>
    )
  }

  return (
    <section
      ref={sectionRef}
      onClickCapture={(event) => {
        // Opening or using a call is a choice to keep this batch open, or the
        // fold would discard it. Answering an approval is not: that is the
        // batch moving along, and it can fold like any other.
        const target = event.target as Element
        if (
          target.closest('.chat-activity-item') &&
          !target.closest('[data-approval-actions]')
        ) {
          expandedGroups.set(groupKey, expanded)
        }
      }}
      className="chat-tool-group"
      data-function-call-group=""
      data-function-call-count={row.items.length}
      data-mounted-settled={mountedSettled ? '' : undefined}
      data-expanded={expanded}
      data-can-collapse={canCollapse}
      data-live={live}
    >
      <button
        type="button"
        aria-expanded={expanded}
        aria-disabled={!canCollapse || undefined}
        aria-label={`${observedOnly ? 'Observed' : 'Agent called'} ${row.items.length} ${noun} · ${expanded ? 'show latest' : 'show all'}`}
        aria-controls={contentId}
        onClick={canCollapse ? toggleExpanded : undefined}
        tabIndex={canCollapse ? undefined : -1}
        className="chat-tool-group__head"
      >
        <ToolGroupIconStack items={row.items} />
        <span className="chat-tool-group__label">
          <span className="chat-call-disclosure-label">
            <span data-active={live} aria-hidden={!live}>
              {observedOnly ? 'Observing' : 'Calling'}
            </span>
            <span data-active={!live} aria-hidden={live}>
              {observedOnly ? 'Observed' : 'Called'}
            </span>
          </span>
          <AnimatedCount
            value={row.items.length}
            reducedMotion={reducedMotion}
          />
          {/* iii has functions, not "tools": the batch says what it called. */}
          <span>{noun}</span>
        </span>
        {failedCount > 0 ? (
          <span
            className="chat-tool-group__failed"
            data-failed-count={failedCount}
          >
            {failedCount} failed
          </span>
        ) : null}
        <ChevronDown
          aria-hidden="true"
          className="chat-tool-group__chevron size-4"
        />
      </button>
      <div
        id={contentId}
        className="chat-tool-group__list"
        data-function-call-group-calls=""
      >
        {presentedItems.map(
          ({ item, index, key, semanticVisible, mounted, ordinal }) => {
            const message = item.message
            const notification =
              item.kind === 'observer-activity' ? item.notification : undefined
            return (
              <div
                key={key}
                className="chat-activity-item"
                data-visible={semanticVisible}
                data-mounted={mounted}
                data-stacked={ordinal > 0}
                data-archiving={mounted && !semanticVisible}
                aria-hidden={!semanticVisible}
                inert={!semanticVisible ? true : undefined}
              >
                <div className="chat-activity-item-inner">
                  {mounted ? (
                    <LiveTimelineRow
                      // An item that absorbed its wake notification represents
                      // two transcript entries; the row carries both ids.
                      messageRow={
                        notification
                          ? `${message.id} ${notification.id}`
                          : message.id
                      }
                      animate={!reducedMotion && livePhaseChildKeys.has(key)}
                      staggerIndex={index}
                    >
                      <Message
                        message={message}
                        agentName={agentName}
                        observerNotification={notification}
                        registration={
                          registrations.get(message.id) ??
                          (notification
                            ? registrations.get(notification.id)
                            : undefined)
                        }
                        defaultOpenCalls={defaultOpenCalls}
                        onLoadDetails={
                          item.kind === 'function-call'
                            ? () => requestEntries([item.message], true)
                            : undefined
                        }
                        onResolveApproval={onResolveApproval}
                        onAlwaysAllow={onAlwaysAllow}
                        onResolveFilesystemAccess={onResolveFilesystemAccess}
                        onManageFilesystemAccess={onManageFilesystemAccess}
                        workingDir={workingDir}
                      />
                    </LiveTimelineRow>
                  ) : null}
                </div>
              </div>
            )
          },
        )}
      </div>
    </section>
  )
}

/** The worker marks that open the header: one per worker, newest trailing. */
function ToolGroupIconStack({
  items,
}: {
  items: TimelineActivityGroupRow['items']
}) {
  const marks: Array<{
    key: string
    kind: 'worker' | 'observer'
    label: string
  }> = []
  for (const item of items) {
    const mark =
      item.kind === 'function-call'
        ? {
            key: `worker:${workerOf(item.message.functionId)}`,
            kind: 'worker' as const,
            label: workerOf(item.message.functionId),
          }
        : { key: 'observer', kind: 'observer' as const, label: 'observer' }
    if (!marks.some((m) => m.key === mark.key)) marks.push(mark)
  }
  const shown = marks.slice(-3)
  return (
    <span className="chat-tool-group__icons" aria-hidden="true">
      {shown.map((mark) => (
        <span
          key={mark.key}
          className="chat-tool-group__icon"
          data-kind={mark.kind}
          title={mark.label}
        >
          {mark.kind === 'observer' ? (
            <ObserverIcon className="size-4 fill-warn" />
          ) : (
            mark.label.slice(0, 1).toUpperCase()
          )}
        </span>
      ))}
    </span>
  )
}

/** `shell::exec` → `shell`; a bare `agent_call` placeholder stays itself. */
function workerOf(functionId: string): string {
  const split = functionId.indexOf('::')
  return split > 0 ? functionId.slice(0, split) : functionId
}

/** The tool count rolls to its next value instead of being replaced. */
function AnimatedCount({
  value,
  reducedMotion,
}: {
  value: number
  reducedMotion: boolean
}) {
  return (
    <span className="chat-tool-group__count">
      <AnimatePresence initial={false} mode="popLayout">
        <motion.span
          key={value}
          initial={
            reducedMotion ? false : { y: 8, opacity: 0, filter: 'blur(2px)' }
          }
          animate={{ y: 0, opacity: 1, filter: 'blur(0px)' }}
          exit={
            reducedMotion
              ? undefined
              : { y: -8, opacity: 0, filter: 'blur(2px)' }
          }
          transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
        >
          {value}
        </motion.span>
      </AnimatePresence>
    </span>
  )
}

type ChatCtx = ReturnType<typeof useConversationsCtxOptional>

/**
 * Map harness presence + the model catalog (from ConversationsContext) onto an
 * `EmptyState` variant. Loading flags hold the `ready` hero so the first paint
 * never flashes an install/configure prompt before the probes resolve.
 */
function resolveEmptyState(
  ctx: ChatCtx,
  density: 'route' | 'dock',
  onConfigureProvider?: () => void,
  directory?: Pick<
    EmptyStateProps,
    | 'workingDir'
    | 'onWorkingDirChange'
    | 'workingDirError'
    | 'defaultWorkingDir'
    | 'worktreePicker'
  >,
): EmptyStateProps {
  if (!ctx) return { variant: 'ready', density, ...directory }

  const { harnessStatus, modelOptions, catalogLoading } = ctx
  const base: EmptyStateProps = {
    variant: 'ready',
    density,
    errorMessage: harnessStatus.error,
    action: harnessStatus.action ?? undefined,
    onInstallHarness: harnessStatus.install,
    onStartHarness: harnessStatus.start,
    onUpdateHarness: harnessStatus.updateAndStart,
    onRetryInstall: harnessStatus.retry,
    onConfigureProvider,
    ...directory,
  }

  if (harnessStatus.error) return { ...base, variant: 'install-failed' }
  if (harnessStatus.installing) return { ...base, variant: 'installing' }
  if (harnessStatus.loading) return base
  // Declared in compose but down: an add would change nothing, so offer a
  // start (or an update and start) instead.
  if (harnessStatus.stopped) return { ...base, variant: 'harness-stopped' }
  if (!harnessStatus.present) return { ...base, variant: 'no-harness' }
  if (catalogLoading) return base
  if (modelOptions.length === 0) return { ...base, variant: 'no-provider' }
  return base
}
