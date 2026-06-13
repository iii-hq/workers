import { useMemo } from 'react'
import { cn } from '@/lib/utils'

const HANDLERS = [
  { id: 'chat::render', sub: 'console ui — deltas in place', y: 30 },
  { id: 'tg::on-message-updated', sub: 'telegram — edit the message', y: 108 },
  { id: 'metrics::on_update', sub: 'dashboard — usage tiles', y: 186 },
] as const

const EDGES = [
  { d: 'M 612 132 C 666 132, 676 52, 728 52', dur: 1.8 },
  { d: 'M 612 132 L 728 130', dur: 1.5 },
  { d: 'M 612 132 C 666 132, 676 208, 728 208', dur: 2.1 },
] as const

/**
 * one write fans out to every bound handler — the ambient, always-on
 * visual for the reactive surface.
 */
export function EventFanOut({ className }: { className?: string }) {
  const reducedMotion = useMemo(
    () =>
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    [],
  )

  return (
    <div className={cn('border border-rule bg-bg', className)}>
      <div className="flex items-center justify-between bg-panel px-3.5 py-2 border-b border-rule">
        <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          one write, every surface
        </span>
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
          ambient — always on
        </span>
      </div>
      <div className="overflow-x-auto">
        <svg
          viewBox="0 0 980 262"
          className="w-full h-auto min-w-[680px] font-mono select-none"
          role="img"
          aria-label="an event fanning out from one write to three bound functions"
        >
          <defs>
            <marker id="fan-arr" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
              <path d="M 0 0 L 8 4 L 0 8 z" className="fill-accent" />
            </marker>
          </defs>

          {/* the write */}
          <rect x={24} y={100} width={230} height={64} strokeWidth={1.25} className="fill-bg stroke-ink" />
          <text x={139} y={127} textAnchor="middle" fontSize="12.5" fontWeight={600} className="fill-ink">
            session::update_message
          </text>
          <text x={139} y={146} textAnchor="middle" fontSize="9.5" letterSpacing="0.05em" className="fill-ink-ghost">
            revision 7 — one write, no publish step
          </text>

          {/* write → trigger type */}
          <path d="M 254 132 L 364 132" fill="none" strokeWidth={1} className="stroke-ink-ghost" markerEnd="url(#fan-arr)" />
          <text x={309} y={122} textAnchor="middle" fontSize="9" letterSpacing="0.04em" className="fill-ink-ghost" style={{ paintOrder: 'stroke', stroke: 'var(--color-bg)', strokeWidth: 3.5 }}>
            emits
          </text>

          {/* the trigger type */}
          <rect x={368} y={106} width={244} height={52} strokeWidth={1.4} className="fill-panel stroke-accent" />
          <text x={490} y={129} textAnchor="middle" fontSize="12" fontWeight={600} className="fill-ink">
            session::message-updated
          </text>
          <text x={490} y={146} textAnchor="middle" fontSize="9" letterSpacing="0.06em" className="fill-ink-faint uppercase">
            trigger type
          </text>
          {!reducedMotion ? (
            <>
              <circle cx={612} cy={132} className="fill-none stroke-accent ripple-ring" strokeWidth={1} />
              <circle cx={612} cy={132} className="fill-none stroke-accent ripple-ring" strokeWidth={1} style={{ animationDelay: '0.8s' }} />
            </>
          ) : null}

          {/* fan-out edges */}
          {EDGES.map((edge, i) => (
            <g key={edge.d}>
              <path d={edge.d} fill="none" strokeWidth={1.1} className="stroke-accent" markerEnd="url(#fan-arr)" />
              {!reducedMotion ? (
                <circle r="2.6" className="fill-accent">
                  <animateMotion dur={`${edge.dur}s`} begin={`${i * 0.25}s`} repeatCount="indefinite" path={edge.d} />
                </circle>
              ) : null}
            </g>
          ))}

          {/* bound handlers */}
          {HANDLERS.map((handler) => (
            <g key={handler.id}>
              <rect x={732} y={handler.y} width={228} height={46} strokeWidth={1} className="fill-bg stroke-rule" />
              <text x={846} y={handler.y + 20} textAnchor="middle" fontSize="11.5" fontWeight={600} className="fill-ink">
                {handler.id}
              </text>
              <text x={846} y={handler.y + 36} textAnchor="middle" fontSize="9" letterSpacing="0.04em" className="fill-ink-faint">
                {handler.sub}
              </text>
            </g>
          ))}

          <text x={846} y={250} textAnchor="middle" fontSize="9" letterSpacing="0.06em" className="fill-ink-ghost uppercase">
            bound functions — subscribe once, render live
          </text>
        </svg>
      </div>
    </div>
  )
}
