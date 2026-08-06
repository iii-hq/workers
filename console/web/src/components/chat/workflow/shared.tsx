import { Chip, StatusPill } from '@/components/chat/sandbox/shared'
import { cn } from '@/lib/utils'
import {
  consumedDeps,
  isJoinNode,
  type NodeState,
  type RunStatus,
  type WorkflowDef,
} from './parsers'

/* ---------------- run + node status presentation ---------------- */

/** Pill copy + variant per terminal/in-flight run status. `awaiting_nodes`
 *  and `running` are in-flight (neutral); `completed` is the only reassuring
 *  accent; failures escalate alert, an explicit stop is warn. */
const RUN_STATUS_PRESENTATION: Record<
  RunStatus,
  { label: string; variant: 'default' | 'accent' | 'warn' | 'alert' }
> = {
  completed: { label: 'completed', variant: 'accent' },
  running: { label: 'running', variant: 'default' },
  awaiting_nodes: { label: 'awaiting nodes', variant: 'default' },
  failed: { label: 'failed', variant: 'alert' },
  cancelled: { label: 'cancelled', variant: 'warn' },
}

export function RunStatusPill({ status }: { status: RunStatus }) {
  const p = RUN_STATUS_PRESENTATION[status]
  return <StatusPill label={p.label} variant={p.variant} />
}

const NODE_STATE_PRESENTATION: Record<
  NodeState,
  { glyph: string; tone: string; pulse?: boolean }
> = {
  done: { glyph: '●', tone: 'text-accent' },
  running: { glyph: '◐', tone: 'text-accent', pulse: true },
  pending: { glyph: '○', tone: 'text-ink-ghost' },
  failed: { glyph: '✕', tone: 'text-alert' },
  cancelled: { glyph: '◌', tone: 'text-warn' },
}

/** A single-glyph state marker for a node row. Color + glyph carry the state
 *  so a long DAG scans at a glance; running pulses. */
export function NodeStateDot({ state }: { state: NodeState }) {
  const p = NODE_STATE_PRESENTATION[state]
  return (
    <span
      role="img"
      aria-label={state}
      title={state}
      className={cn('font-mono shrink-0', p.tone, p.pulse && 'animate-pulse')}
    >
      {p.glyph}
    </span>
  )
}

/* ---------------- run id ---------------- */

/** The run handle — the copy-target an operator pastes into
 *  `workflow::status`. Rendered as a labeled mono row, never buried. */
export function RunIdRow({ runId }: { runId: string }) {
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint break-all">
      <span className="uppercase tracking-[0.06em] text-[10px] mr-1">run</span>
      <span className="text-ink select-all">{runId}</span>
    </div>
  )
}

/* ---------------- DAG summary ---------------- */

interface DagSummaryProps {
  def: WorkflowDef
  /** uid → state, when a status payload is available (run/status views). The
   *  start view omits this; nodes then render structure-only. */
  states?: Record<string, NodeState>
}

/** Compact, readable rendering of the DAG: one row per node with its model,
 *  dependency edges, fan-out source, join badge, and (when known) live state.
 *  This is the core "better view" — structure instead of a raw JSON blob. */
export function DagSummary({ def, states }: DagSummaryProps) {
  const entries = Object.entries(def.nodes)
  const outputId = def.output?.from
    ? def.output.from.replace(/^node:/, '').split('.')[0]
    : undefined

  return (
    <div className="border-b border-rule-2">
      <div className="flex items-center gap-2 px-3 py-1.5 bg-paper-2 border-b border-rule-2">
        <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
          dag
        </span>
        <span className="font-mono text-[11px] text-ink-faint">
          {entries.length} {entries.length === 1 ? 'node' : 'nodes'}
        </span>
        {outputId ? (
          <span className="font-mono text-[11px] text-ink-ghost">
            → output <span className="text-accent">{outputId}</span>
          </span>
        ) : null}
      </div>
      <ul className="divide-y divide-rule-2">
        {entries.map(([id, node]) => {
          const deps = node.depends_on ?? []
          const join = isJoinNode(node)
          const consumes = consumedDeps(node.input?.from)
          const state = states?.[id]
          return (
            <li key={id} className="flex items-start gap-2 px-3 py-1.5">
              {state ? (
                <NodeStateDot state={state} />
              ) : (
                <span className="font-mono text-ink-ghost shrink-0">·</span>
              )}
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-[12.5px] text-ink font-medium break-all">
                    {id}
                  </span>
                  {id === outputId ? (
                    <Chip className="text-accent border-accent/40">output</Chip>
                  ) : null}
                  {join ? <Chip className="text-accent">join</Chip> : null}
                  {node.fanout ? <Chip>fan-out</Chip> : null}
                  {node.agent.model ? <Chip>{node.agent.model}</Chip> : null}
                </div>
                <DagEdges
                  deps={deps}
                  consumes={consumes}
                  fanoutOver={node.fanout?.over}
                />
              </div>
            </li>
          )
        })}
      </ul>
    </div>
  )
}

/** The wiring line under a node: which deps it waits on, which outputs it
 *  actually reads (join clarity), and its fan-out source. Kept terse + mono. */
function DagEdges({
  deps,
  consumes,
  fanoutOver,
}: {
  deps: string[]
  consumes: string[]
  fanoutOver?: string
}) {
  if (deps.length === 0 && !fanoutOver) return null
  return (
    <div className="mt-0.5 font-mono text-[11px] text-ink-faint break-all">
      {deps.length > 0 ? (
        <span>
          ← {deps.join(', ')}
          {/* When a node depends on more than it reads, the gap is the exact
              footgun the worker now rejects; surface it so a stale def reads
              clearly. */}
          {consumes.length > 0 && consumes.length < deps.length ? (
            <span className="text-warn"> · reads {consumes.join(', ')}</span>
          ) : null}
        </span>
      ) : null}
      {fanoutOver ? (
        <span className={cn(deps.length > 0 && 'ml-2')}>
          ⋔ over {fanoutOver}
        </span>
      ) : null}
    </div>
  )
}

/* ---------------- progress bar (status view) ---------------- */

/** Thin segmented bar: done (accent) / running (accent dim) / failed (alert) /
 *  remaining (rule). A glanceable completion read for a long run. */
export function ProgressBar({
  done,
  running,
  failed,
  total,
}: {
  done: number
  running: number
  failed: number
  total: number
}) {
  if (total === 0) return null
  const pct = (n: number) => `${(n / total) * 100}%`
  return (
    <div className="flex h-1.5 w-full overflow-hidden bg-surface">
      <div className="bg-accent" style={{ width: pct(done) }} />
      <div className="bg-accent/40" style={{ width: pct(running) }} />
      <div className="bg-alert" style={{ width: pct(failed) }} />
    </div>
  )
}

/* ---------------- small rows ---------------- */

export function GhostRow({ label }: { label: string }) {
  return (
    <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
      {`· ${label}`}
    </div>
  )
}

/** Renders a node result / run result that may be a string (markdown/text) or
 *  a JSON object. Strings render verbatim in a scroll box; objects pretty-print. */
export function ResultPane({
  label,
  value,
  tone = 'ink',
}: {
  label: string
  value: unknown
  tone?: 'ink' | 'warn'
}) {
  const text = typeof value === 'string' ? value : safeStringify(value)
  const labelTone = tone === 'warn' ? 'text-warn' : 'text-ink-faint'
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 pt-2 pb-1 bg-paper-2 border-b border-rule-2">
        <span
          className={cn(
            'font-mono text-[10px] uppercase tracking-[0.06em]',
            labelTone,
          )}
        >
          {label}
        </span>
      </div>
      <pre className="max-h-80 overflow-auto px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
        {text}
      </pre>
    </div>
  )
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
