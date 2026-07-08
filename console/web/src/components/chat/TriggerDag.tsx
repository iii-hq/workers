import { GitMerge } from 'lucide-react'
import { useMemo } from 'react'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { cn } from '@/lib/utils'
import {
  buildTriggerDag,
  type DagNodeBox,
  layoutTriggerDag,
} from './trigger-graph'

/**
 * The reactive pipeline as a layered node-link diagram: state keys and watched
 * sessions on the left, the sub-agents they spawn flowing rightward, joins as
 * fan-in gates. Same surface pattern as WorktreeGraph — absolutely-positioned
 * HTML nodes over one SVG of orthogonal elbow edges, sized by the pure layout
 * in trigger-graph.ts. Static (no motion); the strip's list is the accessible
 * equivalent, so the SVG is decorative and the nodes carry the real text.
 */
interface TriggerDagProps {
  triggers: SessionTriggerInfo[]
  /** trigger.id → whether its watched state key exists (colors state roots). */
  keyPresence?: Record<string, boolean>
}

export function TriggerDag({ triggers, keyPresence }: TriggerDagProps) {
  const layout = useMemo(
    () => layoutTriggerDag(buildTriggerDag(triggers, { keyPresence })),
    [triggers, keyPresence],
  )

  if (layout.boxes.length === 0) {
    return (
      <div className="px-3 py-6 text-center font-mono text-[12px] text-ink-ghost">
        · no connectable bindings to graph
      </div>
    )
  }

  return (
    <div className="overflow-auto border border-rule-2 bg-paper-2/40">
      <div
        className="relative mx-auto"
        style={{ width: layout.width, height: layout.height }}
      >
        <svg
          aria-hidden="true"
          role="presentation"
          className="absolute inset-0"
          width={layout.width}
          height={layout.height}
        >
          <defs>
            <marker
              id="trigger-dag-arrow"
              viewBox="0 0 8 8"
              refX="7"
              refY="4"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M0 0 L8 4 L0 8 z" className="fill-ink-ghost" />
            </marker>
          </defs>
          {layout.edges.map((edge) => (
            <g key={`${edge.from}->${edge.to}:${edge.label ?? ''}`}>
              <path
                d={`M ${edge.fx} ${edge.fy} H ${edge.midX} V ${edge.ty} H ${edge.tx}`}
                markerEnd="url(#trigger-dag-arrow)"
                strokeWidth={1}
                className={cn(
                  'fill-none',
                  edge.kind === 'join'
                    ? 'stroke-ink-ghost [stroke-dasharray:3_2]'
                    : 'stroke-rule',
                )}
              />
              {edge.label ? (
                <text
                  x={edge.midX}
                  y={edge.ty - 4}
                  textAnchor="middle"
                  className="fill-ink-ghost font-mono text-[9px] lowercase"
                >
                  {edge.label}
                </text>
              ) : null}
            </g>
          ))}
        </svg>

        {layout.boxes.map((box) => (
          <DagNodeCard key={box.id} box={box} />
        ))}
      </div>
    </div>
  )
}

function DagNodeCard({ box }: { box: DagNodeBox }) {
  const stalled = box.kind === 'state' && box.present === false
  const written = box.kind === 'state' && box.present === true
  return (
    <div
      className={cn(
        'absolute flex flex-col justify-center gap-0.5 border px-2.5 font-mono',
        box.kind === 'owner'
          ? 'border-accent bg-bg'
          : box.kind === 'join'
            ? 'border-rule bg-paper-2'
            : stalled
              ? 'border-warn/50 bg-bg'
              : 'border-rule bg-bg',
      )}
      style={{ left: box.x, top: box.y, width: box.w, height: box.h }}
      title={box.kind === 'session' ? box.label : undefined}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        {box.kind === 'join' ? (
          <GitMerge size={11} className="shrink-0 text-ink-faint" aria-hidden />
        ) : (
          <span
            className={cn(
              'shrink-0 text-[9px] uppercase tracking-[0.06em]',
              box.kind === 'state'
                ? stalled
                  ? 'text-warn'
                  : written
                    ? 'text-ok'
                    : 'text-ink-ghost'
                : box.kind === 'owner'
                  ? 'text-accent'
                  : 'text-ink-ghost',
            )}
          >
            {box.kind === 'state'
              ? 'state'
              : box.kind === 'owner'
                ? 'chat'
                : 'agent'}
          </span>
        )}
        <span
          className={cn(
            'min-w-0 truncate text-[11.5px]',
            box.kind === 'owner' ? 'text-accent' : 'text-ink',
          )}
        >
          {box.kind === 'join' ? `join ${box.label}` : box.label}
        </span>
      </span>
      {box.sub ? (
        <span className="min-w-0 truncate text-[10px] text-ink-ghost">
          {box.sub}
        </span>
      ) : box.kind === 'state' ? (
        <span
          className={cn(
            'min-w-0 truncate text-[10px]',
            stalled
              ? 'text-warn'
              : written
                ? 'text-ink-ghost'
                : 'text-ink-ghost',
          )}
        >
          {stalled ? 'not written yet' : written ? 'written' : 'state key'}
        </span>
      ) : null}
    </div>
  )
}
