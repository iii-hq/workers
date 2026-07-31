/**
 * The landing-page demo surface: the console's real chat transcript and real
 * traces surface, side by side, replaying a scripted turn.
 *
 * Everything below the chrome is the shipped product code — `MessageList`
 * (and through it every per-worker result renderer), and on the traces side
 * the page's own stack: the live `TimelineStrip` masthead, `TraceFilters`,
 * the `timeline`/`waterfall` switcher over `TraceTimeline` / `WaterfallChart`,
 * and the `WorkerBreakdown` footer: same components, same defaults (the
 * hierarchical timeline, not the waterfall). Only three things are demo-local:
 * the composer is a lookalike that types instead of a Lexical editor, the
 * events come from `scenario.ts` instead of the engine, and the callout chips
 * are new.
 */

import { ArrowUp, Paperclip } from 'lucide-react'
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react'
import { ContextUsage } from '@/components/chat/ContextUsage'
import { MessageList } from '@/components/chat/MessageList'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import { TraceFilters } from '@/pages/TracesV2/components/TraceFilters'
import { TimelineStrip } from '@/pages/TracesV2/components/timeline/TimelineStrip'
import { TraceTimeline } from '@/pages/TracesV2/components/timeline/TraceTimeline'
import {
  ViewSwitcher,
  type ViewType,
} from '@/pages/TracesV2/components/ViewSwitcher'
import { WaterfallChart } from '@/pages/TracesV2/components/WaterfallChart'
import { WorkerBreakdown } from '@/pages/TracesV2/components/WorkerBreakdown'
import { useTraceFilters } from '@/pages/TracesV2/hooks/useTraceFilters'
import type { VisualizationSpan } from '@/pages/TracesV2/lib/traceTransform'
import { DemoEmptyState } from './EmptyState'
import { MODEL_ID, SESSION_ID, TRACE_ID } from './scenario'
import { usePlayer } from './usePlayer'

const MODEL_LABEL = 'claude sonnet 5'
const CTA_TEXT = 'click here to try the harness for yourself'
const CTA_HREF = 'https://workers.iii.dev/workers/harness#quickstart'
const CONTEXT_WINDOW = 1_000_000

/** The sidebar's write actions are wired to nothing: this is a recording. */
const noop = () => {}

/** Back within this many px of the bottom counts as "following again". */
const FOLLOW_SLACK_PX = 120

/**
 * Keep a pane's scroll on the newest row while the turn plays, and stop the
 * moment the reader takes the scrollbar.
 *
 * The demo has to pin: `MessageList` only follows when the viewport is
 * already near the bottom, and its scroll-the-approval-into-view pass parks
 * it mid-transcript for the rest of the turn, while the waterfall never
 * follows at all. But pinning must lose to a person — scrolling up to reread
 * a card should not be yanked back by the next token.
 *
 * Following is released on real input (wheel, touch) rather than on any
 * scroll, because the programmatic scrolls are indistinguishable from a
 * user's by position alone. It re-arms when a scroll lands back at the
 * bottom, so scrolling down to catch up resumes the follow.
 */
function useTailFollow(wrapRef: RefObject<HTMLElement | null>) {
  const followRef = useRef(true)

  useEffect(() => {
    const wrap = wrapRef.current
    if (!wrap) return
    const release = () => {
      followRef.current = false
    }
    const onScroll = (event: Event) => {
      const el = event.target as HTMLElement | null
      if (!el || typeof el.scrollHeight !== 'number') return
      const fromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
      if (fromBottom <= FOLLOW_SLACK_PX) followRef.current = true
    }
    wrap.addEventListener('wheel', release, { passive: true })
    wrap.addEventListener('touchmove', release, { passive: true })
    // Scroll does not bubble; capture catches whichever child is scrolling.
    wrap.addEventListener('scroll', onScroll, { capture: true, passive: true })
    return () => {
      wrap.removeEventListener('wheel', release)
      wrap.removeEventListener('touchmove', release)
      wrap.removeEventListener('scroll', onScroll, { capture: true })
    }
  }, [wrapRef])

  return useCallback(() => {
    if (!followRef.current) return
    const el = wrapRef.current?.querySelector<HTMLElement>('.overflow-y-auto')
    if (!el) return
    const id = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight
    })
    return () => cancelAnimationFrame(id)
  }, [wrapRef])
}

export interface LandingDemoProps {
  /** Pause the whole thing when the overlay is closed. */
  active?: boolean
  /**
   * Replay forever. Off by default: the turn ends holding a finished
   * transcript, a full trace and three child sessions, and that is the state
   * worth clicking around in. Restarting on top of someone reading it is not.
   */
  loop?: boolean
}

export function LandingDemo({ active = true, loop = false }: LandingDemoProps) {
  const player = usePlayer(active, loop)
  const [selectedSpanId, setSelectedSpanId] = useState<string | null>(null)
  // The page's default view, and the page's own filter state. The filters
  // control a single recorded trace, so narrowing has nothing to narrow;
  // they are here because the surface is not the surface without them.
  const [view, setView] = useState<ViewType>('timeline')
  const traceFilters = useTraceFilters()
  const listWrapRef = useRef<HTMLDivElement>(null)
  const traceWrapRef = useRef<HTMLDivElement>(null)

  const handleSpanClick = useCallback((span: VisualizationSpan) => {
    setSelectedSpanId((prev) => (prev === span.span_id ? null : span.span_id))
  }, [])

  /* ~8s into the replay the composer types out an invitation to run the
     harness yourself — the type-out is the attention cue, and the typed text
     is a live link. Hidden whenever the player owns the composer (the loop
     retyping the scenario prompt). Keyed on the run, so a replay types it out
     again instead of revealing the finished line the moment the composer
     frees up. */
  const started = player.phase !== 'idle'
  const [ctaChars, setCtaChars] = useState(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: runKey is the restart trigger, not a value read here.
  useEffect(() => {
    if (!started) return
    setCtaChars(0)
    let interval: ReturnType<typeof setInterval> | undefined
    const delay = setTimeout(() => {
      interval = setInterval(() => {
        setCtaChars((c) => {
          if (c >= CTA_TEXT.length) {
            clearInterval(interval)
            return c
          }
          return c + 1
        })
      }, 55)
    }, 8000)
    return () => {
      clearTimeout(delay)
      clearInterval(interval)
    }
  }, [started, player.runKey])
  const cta = player.typed ? '' : CTA_TEXT.slice(0, ctaChars)

  const pinTranscript = useTailFollow(listWrapRef)
  const pinTrace = useTailFollow(traceWrapRef)

  // biome-ignore lint/correctness/useExhaustiveDependencies: the transcript is the trigger, not a value read here.
  useEffect(pinTranscript, [pinTranscript, player.messages, player.callout])
  // biome-ignore lint/correctness/useExhaustiveDependencies: the span count is the trigger, not a value read here.
  useEffect(pinTrace, [pinTrace, player.spanCount, player.callout])

  const working = player.phase === 'streaming'
  /* The header follows whichever session the sidebar has selected. */
  const paneWorking = player.activeChild
    ? player.activeChild.status === 'working'
    : working

  return (
    <div className="flex h-full min-h-0 flex-col bg-bg text-ink">
      <DemoChrome
        working={working}
        paused={player.paused}
        onTogglePause={player.togglePause}
        onReplay={player.replay}
      />

      {/*
        Sidebar, transcript, traces — the console's own three columns, once
        there is room for them. Narrower than that only the chat survives:
        the sidebar and the traces pane hide, and the transcript takes the
        whole frame.
      */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden lg:flex-row">
        <div className="hidden lg:flex">
          <ConversationSidebar
            conversations={player.conversations}
            activeId={player.activeId}
            width={260}
            onSelect={player.select}
            onCreate={noop}
            onRename={noop}
            onRemove={noop}
          />
        </div>

        {/* ── transcript ─────────────────────────────────────────────── */}
        <section className="relative flex min-h-0 flex-1 flex-col lg:flex-none lg:w-[46%]">
          <div className="pointer-events-none absolute inset-x-0 top-0 z-10 flex flex-col items-center">
            <MobileNotice />
          </div>
          <header className="flex items-center justify-between gap-3 whitespace-nowrap border-b border-rule px-5 py-3 lg:px-9">
            <div className="flex min-w-0 flex-1 items-center gap-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              <span className="flex-shrink-0 text-accent" aria-hidden>
                $
              </span>
              <span className="hidden min-w-0 truncate text-ink sm:inline">
                {MODEL_LABEL}
              </span>
              <span className="hidden flex-shrink-0 text-ink-ghost sm:inline">
                ·
              </span>
              <span className="flex-shrink-0 text-ink-faint">agent</span>
              {player.activeChild ? (
                <>
                  <span className="flex-shrink-0 text-ink-ghost">·</span>
                  <span className="flex-shrink-0 text-accent">
                    subagent · depth 1
                  </span>
                </>
              ) : null}
              <span className="hidden flex-shrink-0 text-ink-ghost md:inline">
                ·
              </span>
              <span className="hidden min-w-0 truncate font-mono text-[11px] normal-case tracking-normal tabular-nums text-ink-faint md:inline">
                {player.activeChild ? player.activeChild.id : SESSION_ID}
              </span>
            </div>
            <div className="flex flex-shrink-0 items-center gap-3 font-mono text-[11px] uppercase tracking-[0.06em]">
              <span className="hidden sm:inline-flex">
                <ContextUsage
                  messages={player.messages}
                  contextWindow={CONTEXT_WINDOW}
                />
              </span>
              <div className="flex items-center gap-2">
                <StatusDot
                  tone={paneWorking ? 'accent' : 'ink'}
                  pulse={paneWorking}
                />
                <span className="text-ink-faint">
                  {paneWorking ? 'working' : 'ready'}
                </span>
              </div>
            </div>
          </header>

          <div ref={listWrapRef} className="flex min-h-0 flex-1 flex-col">
            <MessageList
              messages={player.messages}
              header={<DemoEmptyState />}
              isThinking={player.isThinking}
              thinkingDetail={
                player.thinkingDetail ?? `dispatching ${MODEL_LABEL}`
              }
              defaultOpenCalls
              onResolveApproval={player.resolveApproval}
            />
          </div>

          <CalloutStrip
            callout={
              player.callout?.anchor === 'transcript' ? player.callout : null
            }
          />

          <footer className="px-5 pb-6 pt-2 lg:px-9">
            <div className="mx-auto max-w-[760px]">
              <FakeComposer
                typed={player.typed}
                streaming={working}
                cta={cta}
              />
            </div>
          </footer>
        </section>

        {/* ── traces ─────────────────────────────────────────────────── */}
        <aside className="relative hidden min-h-0 flex-1 flex-col border-l border-rule bg-panel lg:flex">
          <header className="flex items-center justify-between gap-3 border-b border-rule px-5 py-3">
            <div className="flex min-w-0 items-center gap-2 font-mono text-[11px] uppercase tracking-[0.06em]">
              <span className="text-ink">traces</span>
              <span className="text-ink-ghost">·</span>
              <span className="min-w-0 truncate normal-case tracking-normal text-ink-faint">
                {TRACE_ID}
              </span>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              <StatusDot tone={working ? 'accent' : 'ink'} pulse={working} />
              <span className="tabular-nums">{player.spanCount} spans</span>
            </div>
          </header>

          {/* The masthead and filter bar the real page carries above its
              trace list; the strip runs live off the same span feed the
              detail below is built from. */}
          <TimelineStrip spans={player.spans} isPaused={!active} />
          <div className="border-b border-rule px-4 py-2.5">
            <TraceFilters
              filters={traceFilters.filters}
              onFilterChange={traceFilters.updateFilter}
              onClear={traceFilters.resetFilters}
              validationWarnings={traceFilters.validationWarnings}
              onClearWarnings={traceFilters.clearValidationWarnings}
              stats={{
                totalTraces: 1,
                errorCount: player.waterfall
                  ? player.waterfall.spans.filter((s) => s.status === 'error')
                      .length
                  : 0,
                avgDuration: player.waterfall?.total_duration_ms ?? 0,
              }}
            />
          </div>

          <div className="border-b border-rule px-4 py-2.5">
            <ViewSwitcher currentView={view} onViewChange={setView} />
          </div>

          <div
            ref={traceWrapRef}
            className="flex min-h-0 flex-1 flex-col overflow-y-auto"
          >
            {player.waterfall ? (
              view === 'timeline' ? (
                <TraceTimeline
                  data={player.waterfall}
                  onSpanClick={handleSpanClick}
                  selectedSpanId={selectedSpanId ?? undefined}
                  fitContent
                />
              ) : (
                <div className="min-h-0 flex-1">
                  {/* The real console boxes the chart at 420px; the demo pane is the whole column, so it takes all of it. */}
                  <WaterfallChart
                    data={player.waterfall}
                    onSpanClick={handleSpanClick}
                    selectedSpanId={selectedSpanId}
                    showExpandControls={false}
                  />
                </div>
              )
            ) : (
              <div className="flex h-full items-center justify-center px-6 text-center font-mono text-[12px] text-ink-ghost">
                waiting for the first span…
              </div>
            )}
          </div>

          {player.waterfall ? (
            <div className="shrink-0 border-t border-rule">
              <WorkerBreakdown data={player.waterfall} />
            </div>
          ) : null}

          <CalloutStrip
            callout={
              player.callout?.anchor === 'waterfall' ? player.callout : null
            }
          />
        </aside>
      </div>
    </div>
  )
}

/**
 * The window frame. The real console is a full app with a sidebar and route
 * tabs; the demo keeps just enough of it to read as the same product.
 */
function DemoChrome({
  working,
  paused,
  onTogglePause,
  onReplay,
}: {
  working: boolean
  paused: boolean
  onTogglePause: () => void
  onReplay: () => void
}) {
  const controlClass =
    'flex items-center gap-1.5 border border-rule px-2.5 py-1 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-faint transition-colors hover:text-ink'
  return (
    <div className="relative flex items-center gap-3 border-b border-rule bg-panel px-5 py-2">
      <span className="font-mono text-[12px] lowercase tracking-[0.06em] text-ink">
        iii console
      </span>
      <TryItCta />
      <span className="text-ink-ghost">·</span>
      <span className="min-w-0 truncate font-mono text-[12px] text-ink-faint">
        payments ledger
      </span>
      <span className="ml-auto flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-ghost">
        <span className="hidden sm:inline">recorded session</span>
        <StatusDot
          tone={working ? 'accent' : 'ink'}
          pulse={working && !paused}
        />
      </span>
      <button type="button" onClick={onTogglePause} className={controlClass}>
        {paused ? 'play' : 'pause'}
      </button>
      <button type="button" onClick={onReplay} className={controlClass}>
        replay
      </button>
      {/* Embedded only: the host page listens for iii-demo-close and scrolls
          the console away. Alert red on black, the loudest thing in the frame,
          because a reader who wants out should not have to look for it. Black
          rather than cream on the red: it is a 10px label and needs the
          contrast. */}
      {window.self !== window.top && (
        <button
          type="button"
          onClick={() =>
            window.parent?.postMessage({ type: 'iii-demo-close' }, '*')
          }
          className="flex items-center gap-1.5 bg-alert px-2.5 py-1 font-mono text-[10px] font-semibold uppercase tracking-[0.14em] text-black transition-opacity hover:opacity-80"
        >
          close <span className="opacity-70">esc</span>
        </button>
      )}
    </div>
  )
}

/**
 * Centered in the window chrome: the one thing in the frame that leads out of
 * the recording and into the reader's own terminal. A shine sweep and a slow
 * 3D tilt carry the attention; the colors are the page's accent tokens, so it
 * reads as part of whichever theme the host is in. Centered absolutely so it
 * stays mid-bar regardless of what the controls on either side measure, and
 * dropped below `md` where there is no room between them.
 */
function TryItCta() {
  return (
    <span className="demo-3d absolute left-1/2 hidden -translate-x-1/2 md:block">
      <a
        href={CTA_HREF}
        target="_blank"
        rel="noreferrer"
        className={cn(
          'demo-shine demo-tilt flex items-center gap-1.5 overflow-hidden',
          'border border-accent bg-accent/10 px-3 py-1 font-mono text-[10px]',
          'uppercase tracking-[0.14em] text-accent transition-colors',
          'hover:bg-accent hover:text-accent-fg',
        )}
      >
        try it for yourself
        <span aria-hidden className="text-[11px]">
          👩‍💻
        </span>
      </a>
    </span>
  )
}

/** Phone-width only: drops in from the top, over the transcript header. */
function MobileNotice() {
  const [shown, setShown] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setShown(true), 400)
    return () => clearTimeout(t)
  }, [])
  return (
    <div
      className={cn(
        'border-x border-b border-rule bg-panel px-3 py-2 text-center font-mono text-[10px] uppercase tracking-[0.08em] text-ink-faint transition-all duration-500 lg:hidden',
        shown ? 'translate-y-0 opacity-100' : '-translate-y-3 opacity-0',
      )}
    >
      this is a simplified view. the iii console is best viewed on larger
      screens.
    </div>
  )
}

/** Composer lookalike: same frame as `Composer`, types instead of editing.
    `cta` types out an invitation in the editor when the player isn't using
    it; the typed text is a live link and the frame glows accent while it
    has something to say. */
function FakeComposer({
  typed,
  streaming,
  cta = '',
}: {
  typed: string
  streaming: boolean
  cta?: string
}) {
  return (
    <div
      className={cn(
        'border bg-panel transition-colors duration-700',
        cta ? 'border-accent' : 'border-rule',
      )}
    >
      <div className="px-1 pt-1">
        <div className="composer-editor min-h-[2.5rem] px-3 py-2 text-[14px] leading-[1.5]">
          {typed.length > 0 ? (
            <span className="whitespace-pre-wrap break-words">
              {typed}
              <span className="ml-px inline-block h-[1.05em] w-[1px] translate-y-[3px] animate-pulse bg-ink" />
            </span>
          ) : cta ? (
            <a
              href={CTA_HREF}
              target="_blank"
              rel="noreferrer"
              className="whitespace-pre-wrap break-words text-accent underline decoration-accent/50 underline-offset-4 hover:decoration-accent"
            >
              {cta}
              <span className="ml-px inline-block h-[1.05em] w-[1px] translate-y-[3px] animate-pulse bg-accent" />
            </a>
          ) : (
            <span className="text-ink-ghost">
              {streaming ? 'streaming response…' : 'send a message…'}
            </span>
          )}
        </div>
      </div>

      <div className="flex min-w-0 items-center gap-2 border-t border-rule-2 px-3 py-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <span className="inline-flex h-9 items-center gap-2 border border-rule bg-bg px-3 font-mono text-[13px] lowercase text-ink">
            agent
          </span>
          <span className="inline-flex h-9 min-w-0 items-center gap-2 border border-rule bg-bg px-3 font-mono text-[13px] lowercase text-ink">
            <span className="truncate">{MODEL_LABEL}</span>
          </span>
          <span className="hidden font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost sm:inline">
            {MODEL_ID.split('::')[0]}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <span className="inline-flex size-9 items-center justify-center text-ink-ghost">
            <Paperclip size={16} aria-hidden />
          </span>
          <span
            className={cn(
              'inline-flex size-9 items-center justify-center rounded-full bg-bg text-ink',
              '[html[data-theme=dark]_&]:bg-white [html[data-theme=dark]_&]:text-[#0a0a0a]',
              typed.length === 0 && 'opacity-40',
            )}
          >
            <ArrowUp size={20} aria-hidden />
          </span>
        </div>
      </div>
    </div>
  )
}

/**
 * The annotation strip. One at a time, replaced as the turn moves on. In
 * flow rather than floating: both panes pin to their newest row, and a
 * floating chip would sit exactly on top of it.
 */
function CalloutStrip({
  callout,
}: {
  callout: { title: string; text: string } | null
}) {
  if (!callout) return null
  return (
    <div
      className="shrink-0 border-t border-accent bg-accent/[0.06] px-5 py-3 lg:px-9"
      aria-live="polite"
    >
      {callout ? (
        <div className="mx-auto max-w-[760px]">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-accent">
            {callout.title}
          </div>
          <p className="mt-1.5 text-[13px] leading-[1.5] text-ink">
            {callout.text}
          </p>
        </div>
      ) : null}
    </div>
  )
}
