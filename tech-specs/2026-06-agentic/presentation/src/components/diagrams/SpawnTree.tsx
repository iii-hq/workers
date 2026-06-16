import { useMemo } from 'react'
import { PlayerControls } from '@/components/PlayerControls'
import { FnChip } from '@/components/schematic/FnChip'
import { useStepper } from '@/hooks/useStepper'
import { cn } from '@/lib/utils'

type ChildState = 'hidden' | 'queued' | 'running' | 'done'

interface TreeState {
  title: string
  desc: string
  parent: 'running' | 'parked' | 'resumed' | 'completed'
  children: [ChildState, ChildState, ChildState]
  joins: [boolean, boolean, boolean] // result edge delivered
}

const CHILDREN = [
  { id: 'a', session: 's_9c1', task: 'survey the provider landscape', x: 80 },
  { id: 'b', session: 's_9c2', task: 'compare pricing + limits', x: 380 },
  { id: 'c', session: 's_9c3', task: 'draft the recommendation', x: 680 },
] as const

const STATES: TreeState[] = [
  {
    title: 'the model asks for three experts',
    desc: 'harness::spawn is just another function call on the one invocation surface — no special protocol. each call names a task, an optional model, a policy request, and a typed output contract: "give me json matching this schema."',
    parent: 'running',
    children: ['hidden', 'hidden', 'hidden'],
    joins: [false, false, false],
  },
  {
    title: 'children spawn — the parent parks',
    desc: 'each child is an ordinary session with its own turn, linked to the parent by metadata. the spawn dispatch reports pending and the parent parks — durable execution means it holds no queue step, costs nothing, and cannot be lost while it waits.',
    parent: 'parked',
    children: ['queued', 'queued', 'queued'],
    joins: [false, false, false],
  },
  {
    title: 'three agents, truly parallel',
    desc: 'the turn queue orders steps per session and runs sessions concurrently — spawn three children, get three loops working at once. every child transcript is live: bind the session events filtered by parent_session_id and watch them think.',
    parent: 'parked',
    children: ['running', 'running', 'running'],
    joins: [false, false, false],
  },
  {
    title: 'the first result joins',
    desc: 'a child that completes resolves the parent\u2019s call with its typed deliverable — schema-validated json when the contract asks for it. the result lands under a deterministic entry id, so a replayed completion can never double-deliver.',
    parent: 'parked',
    children: ['running', 'done', 'running'],
    joins: [false, true, false],
  },
  {
    title: 'all results in — the parent wakes',
    desc: 'the last resolve re-enqueues the parent turn exactly where it parked. the model now sees three function_results in its transcript and reasons over all of them at once.',
    parent: 'resumed',
    children: ['done', 'done', 'done'],
    joins: [true, true, true],
  },
  {
    title: 'one answer, a tree of work',
    desc: 'depth, fan-out, and turn budgets bound the whole tree; a child\u2019s policy is the parent\u2019s intersected with its request — narrow, never escalate. stop the parent and the cancellation cascades to every descendant.',
    parent: 'completed',
    children: ['done', 'done', 'done'],
    joins: [true, true, true],
  },
]

const PARENT = { x: 360, y: 36, w: 260, h: 78 }
const CHILD_Y = 196
const CHILD_W = 240
const CHILD_H = 92

export function SpawnTree({ className }: { className?: string }) {
  const stepper = useStepper(STATES.length, 3200)
  const state = STATES[stepper.step]
  const reducedMotion = useMemo(
    () =>
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    [],
  )

  const parentLabel: Record<TreeState['parent'], string> = {
    running: 'running — generating',
    parked: 'parked — awaiting_functions · zero queue steps held',
    resumed: 'running — reasoning over results',
    completed: 'completed — result delivered',
  }

  return (
    <div className={cn('border border-rule bg-bg', className)}>
      <div className="flex flex-wrap items-center justify-between gap-2 bg-panel px-3.5 py-2 border-b border-rule">
        <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          harness::spawn — fan out, join back
        </span>
        <span className="flex gap-x-2">
          <FnChip tone="ghost">depth ≤ 3</FnChip>
          <FnChip tone="ghost">fan-out ≤ 5</FnChip>
          <FnChip tone="ghost">turns ≤ parent budget</FnChip>
        </span>
      </div>

      <div className="overflow-x-auto">
        <svg
          viewBox="0 0 980 320"
          className="w-full h-auto min-w-[760px] font-mono select-none"
          role="img"
          aria-label="a parent agent spawning three parallel sub-agents and joining their results"
        >
          <defs>
            <marker id="tree-arr" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
              <path d="M 0 0 L 8 4 L 0 8 z" className="fill-ink-ghost" />
            </marker>
            <marker id="tree-arr-accent" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
              <path d="M 0 0 L 8 4 L 0 8 z" className="fill-accent" />
            </marker>
          </defs>

          {/* parent node */}
          <rect
            x={PARENT.x}
            y={PARENT.y}
            width={PARENT.w}
            height={PARENT.h}
            strokeWidth={1.4}
            className={cn(
              'transition-all duration-300',
              state.parent === 'parked' ? 'fill-paper-2 stroke-warn' : 'fill-panel stroke-accent',
            )}
          />
          <text x={PARENT.x + PARENT.w / 2} y={PARENT.y + 26} textAnchor="middle" fontSize="13" fontWeight={600} className="fill-ink">
            parent turn — s_7a1 / t_004
          </text>
          <text x={PARENT.x + PARENT.w / 2} y={PARENT.y + 45} textAnchor="middle" fontSize="9.5" letterSpacing="0.03em" className={cn(state.parent === 'parked' ? 'fill-warn' : 'fill-ink-faint')}>
            {parentLabel[state.parent]}
          </text>
          <text x={PARENT.x + PARENT.w / 2} y={PARENT.y + 62} textAnchor="middle" fontSize="9" className="fill-ink-ghost">
            {stepper.step === 0 ? 'model emits: harness::spawn ×3' : 'calls: 3 × harness::spawn'}
          </text>

          {/* spawn + join edges, per child */}
          {CHILDREN.map((child, i) => {
            const cs = state.children[i]
            if (cs === 'hidden') return null
            const cx = child.x + CHILD_W / 2
            const px = PARENT.x + PARENT.w / 2 + (i - 1) * 70
            const spawnD = `M ${px} ${PARENT.y + PARENT.h} C ${px} ${PARENT.y + PARENT.h + 40}, ${cx} ${CHILD_Y - 40}, ${cx} ${CHILD_Y}`
            const joinD = `M ${cx + 24} ${CHILD_Y} C ${cx + 24} ${CHILD_Y - 44}, ${px + 24} ${PARENT.y + PARENT.h + 36}, ${px + 24} ${PARENT.y + PARENT.h}`
            const joined = state.joins[i]
            return (
              <g key={child.id}>
                <path
                  d={spawnD}
                  fill="none"
                  strokeWidth={1}
                  className={cn('transition-[stroke]', cs === 'queued' ? 'stroke-rule' : 'stroke-ink-ghost')}
                  markerEnd="url(#tree-arr)"
                />
                {joined ? (
                  <>
                    <path d={joinD} fill="none" strokeWidth={1.2} strokeDasharray="5 4" className="stroke-accent" markerEnd="url(#tree-arr-accent)" />
                    {!reducedMotion && stepper.step < STATES.length - 1 ? (
                      <circle r="2.4" className="fill-accent">
                        <animateMotion dur="1.6s" repeatCount="indefinite" path={joinD} />
                      </circle>
                    ) : null}
                  </>
                ) : null}
              </g>
            )
          })}

          {/* children */}
          {CHILDREN.map((child, i) => {
            const cs = state.children[i]
            if (cs === 'hidden') return null
            return (
              <g key={child.id} className="transition-opacity duration-300">
                <rect
                  x={child.x}
                  y={CHILD_Y}
                  width={CHILD_W}
                  height={CHILD_H}
                  strokeWidth={cs === 'running' ? 1.4 : 1}
                  className={cn(
                    'transition-all duration-300',
                    cs === 'queued' && 'fill-bg stroke-rule',
                    cs === 'running' && 'fill-bg stroke-ink',
                    cs === 'done' && 'fill-paper-2 stroke-ink-faint',
                  )}
                />
                <circle
                  cx={child.x + 16}
                  cy={CHILD_Y + 19}
                  r={3}
                  className={cn(
                    cs === 'queued' && 'fill-ink-ghost',
                    cs === 'running' && 'fill-accent',
                    cs === 'done' && 'fill-ink-faint',
                  )}
                >
                  {cs === 'running' && !reducedMotion ? (
                    <animate attributeName="opacity" values="1;0.25;1" dur="1.2s" repeatCount="indefinite" />
                  ) : null}
                </circle>
                <text x={child.x + 28} y={CHILD_Y + 23} fontSize="11.5" fontWeight={600} className="fill-ink">
                  sub-agent {child.id} — {child.session}
                </text>
                <text x={child.x + 16} y={CHILD_Y + 44} fontSize="9.5" className="fill-ink-faint">
                  task: {child.task}
                </text>
                <text x={child.x + 16} y={CHILD_Y + 62} fontSize="9" className="fill-ink-ghost">
                  policy: parent ∩ request · output: json + schema
                </text>
                <text x={child.x + 16} y={CHILD_Y + 78} fontSize="9" letterSpacing="0.05em" className={cn('uppercase', cs === 'running' ? 'fill-accent' : 'fill-ink-ghost')}>
                  {cs === 'queued' ? 'queued' : cs === 'running' ? 'turn running' : 'completed ✓'}
                </text>
              </g>
            )
          })}
        </svg>
      </div>

      <div className="border-t border-rule px-4 py-3.5 min-h-[96px]">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <span className="font-mono text-[11px] text-ink-ghost tabular-nums">
            {String(stepper.step + 1).padStart(2, '0')}
          </span>
          <span className="font-mono text-[14px] font-semibold lowercase text-ink">
            {state.title}
          </span>
        </div>
        <p className="mt-1.5 font-mono text-[12.5px] leading-[1.65] text-ink-faint lowercase max-w-[88ch]">
          {state.desc}
        </p>
      </div>

      <PlayerControls stepper={stepper} total={STATES.length} />
    </div>
  )
}
