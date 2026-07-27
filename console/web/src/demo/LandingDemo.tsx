/**
 * The landing-page demo surface: the console's real chat transcript and real
 * trace waterfall, side by side, replaying a scripted turn.
 *
 * Everything below the chrome is the shipped product code — `MessageList`
 * (and through it every per-worker result renderer) and `WaterfallChart` over
 * `toWaterfallData`. Only three things are demo-local: the composer is a
 * lookalike that types instead of a Lexical editor, the events come from
 * `scenario.ts` instead of the engine, and the callout chips are new.
 */

import { ArrowUp, Paperclip } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ContextUsage } from '@/components/chat/ContextUsage'
import { MessageList } from '@/components/chat/MessageList'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import { WaterfallChart } from '@/pages/TracesV2/components/WaterfallChart'
import type { VisualizationSpan } from '@/pages/TracesV2/lib/traceTransform'
import { MODEL_ID, SESSION_ID, TRACE_ID } from './scenario'
import { usePlayer } from './usePlayer'

const MODEL_LABEL = 'claude opus 4.7'
const CONTEXT_WINDOW = 1_000_000

export interface LandingDemoProps {
  /** Pause the whole thing when the overlay is closed. */
  active?: boolean
  loop?: boolean
}

export function LandingDemo({ active = true, loop = true }: LandingDemoProps) {
  const player = usePlayer(active, loop)
  const [selectedSpanId, setSelectedSpanId] = useState<string | null>(null)
  const listWrapRef = useRef<HTMLDivElement>(null)
  const traceWrapRef = useRef<HTMLDivElement>(null)

  const handleSpanClick = useCallback((span: VisualizationSpan) => {
    setSelectedSpanId((prev) => (prev === span.span_id ? null : span.span_id))
  }, [])

  /**
   * Nobody is driving the scrollbar here. `MessageList` only follows the tail
   * when the reader is already near it, and its scroll-the-approval-into-view
   * pass parks the viewport mid-transcript for the rest of the turn; the
   * waterfall never follows at all (a reader inspecting a trace would hate
   * that). Both panes are pinned to the tail so the demo keeps showing the
   * thing that just happened.
   */
  // biome-ignore lint/correctness/useExhaustiveDependencies: the transcript is the trigger, not a value read here.
  useEffect(() => {
    const el = listWrapRef.current?.firstElementChild as HTMLElement | null
    if (!el) return
    const id = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight
    })
    return () => cancelAnimationFrame(id)
  }, [player.messages, player.callout])

  // biome-ignore lint/correctness/useExhaustiveDependencies: the span count is the trigger, not a value read here.
  useEffect(() => {
    const el =
      traceWrapRef.current?.querySelector<HTMLElement>('.overflow-y-auto')
    if (!el) return
    const id = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight
    })
    return () => cancelAnimationFrame(id)
  }, [player.spanCount, player.callout])

  const working = player.phase === 'streaming'

  return (
    <div className="flex h-full min-h-0 flex-col bg-bg text-ink">
      <DemoChrome working={working} />

      {/*
        Two panes side by side once there is room for both. Narrower than
        that they stack, each with a height of its own and the pair
        scrolling: a 58/42 split of a phone screen leaves the transcript
        with no room to be a transcript.
      */}
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden">
        {/* ── transcript ─────────────────────────────────────────────── */}
        <section className="relative flex h-[72vh] shrink-0 flex-col lg:h-auto lg:min-h-0 lg:w-[58%]">
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
              <span className="hidden flex-shrink-0 text-ink-ghost md:inline">
                ·
              </span>
              <span className="hidden min-w-0 truncate font-mono text-[11px] normal-case tracking-normal tabular-nums text-ink-faint md:inline">
                {SESSION_ID}
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
                <StatusDot tone={working ? 'accent' : 'ink'} pulse={working} />
                <span className="text-ink-faint">
                  {working ? 'working' : 'ready'}
                </span>
              </div>
            </div>
          </header>

          <div ref={listWrapRef} className="flex min-h-0 flex-1 flex-col">
            <MessageList
              messages={player.messages}
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
              <FakeComposer typed={player.typed} streaming={working} />
            </div>
          </footer>
        </section>

        {/* ── traces ─────────────────────────────────────────────────── */}
        <aside className="relative flex h-[68vh] shrink-0 flex-col border-t border-rule bg-panel lg:h-auto lg:min-h-0 lg:flex-1 lg:border-l lg:border-t-0">
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

          <div ref={traceWrapRef} className="flex min-h-0 flex-1 flex-col">
            {player.waterfall ? (
              <WaterfallChart
                data={player.waterfall}
                onSpanClick={handleSpanClick}
                selectedSpanId={selectedSpanId}
              />
            ) : (
              <div className="flex h-full items-center justify-center px-6 text-center font-mono text-[12px] text-ink-ghost">
                waiting for the first span…
              </div>
            )}
          </div>

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
function DemoChrome({ working }: { working: boolean }) {
  return (
    <div className="flex items-center gap-3 border-b border-rule bg-panel px-5 py-2">
      <span className="font-mono text-[12px] lowercase tracking-[0.06em] text-ink">
        iii console
      </span>
      <span className="text-ink-ghost">·</span>
      <span className="min-w-0 truncate font-mono text-[12px] text-ink-faint">
        payments ledger
      </span>
      <span className="ml-auto flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-ghost">
        <span className="hidden sm:inline">recorded session</span>
        <StatusDot tone={working ? 'accent' : 'ink'} pulse={working} />
      </span>
    </div>
  )
}

/** Composer lookalike: same frame as `Composer`, types instead of editing. */
function FakeComposer({
  typed,
  streaming,
}: {
  typed: string
  streaming: boolean
}) {
  const showPlaceholder = typed.length === 0
  return (
    <div className="border border-rule bg-panel">
      <div className="px-1 pt-1">
        <div className="composer-editor min-h-[2.5rem] px-3 py-2 text-[14px] leading-[1.5]">
          {showPlaceholder ? (
            <span className="text-ink-ghost">
              {streaming ? 'streaming response…' : 'send a message…'}
            </span>
          ) : (
            <span className="whitespace-pre-wrap break-words">
              {typed}
              <span className="ml-px inline-block h-[1.05em] w-[1px] translate-y-[3px] animate-pulse bg-ink" />
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
