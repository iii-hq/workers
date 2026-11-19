import { Badge, Eyebrow } from '@iii-dev/console-ui'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import type { ReactNode } from 'react'
import { inputSources, isJoinNode, type NodeState, type RunStatus, type WorkflowDef } from '../lib/parsers'

/* ---------------- run + node status presentation ---------------- */

/** Badge copy + variant per terminal/in-flight run status. `awaiting_nodes`
 *  and `running` are in-flight (neutral); `completed` is the only reassuring
 *  accent; failures escalate alert, an explicit stop is warn. */
const RUN_STATUS_PRESENTATION: Record<RunStatus, { label: string; variant: 'default' | 'accent' | 'warn' | 'alert' }> =
  {
    completed: { label: 'completed', variant: 'accent' },
    running: { label: 'running', variant: 'default' },
    awaiting_nodes: { label: 'awaiting nodes', variant: 'default' },
    failed: { label: 'failed', variant: 'alert' },
    cancelled: { label: 'cancelled', variant: 'warn' },
  }

export function RunStatusPill({ status }: { status: RunStatus }) {
  const p = RUN_STATUS_PRESENTATION[status]
  return (
    <Badge className="wf-pill" variant={p.variant}>
      {p.label}
    </Badge>
  )
}

const NODE_STATE_GLYPH: Record<NodeState, string> = {
  done: '●',
  running: '◐',
  pending: '○',
  failed: '✕',
  cancelled: '◌',
}

/** A single-glyph state marker for a node row. Color (by `data-node-state`) +
 *  glyph carry the state so a long DAG scans at a glance; running pulses. */
export function NodeStateDot({ state }: { state: NodeState }) {
  return (
    <span
      role="img"
      aria-label={state}
      title={state}
      data-node-state={state}
      className={state === 'running' ? `wf-dot ${uiClasses.pulse}` : 'wf-dot'}
    >
      {NODE_STATE_GLYPH[state]}
    </span>
  )
}

/** The cards' outlined chip (the console's `Chip` is filled). */
export function WfChip({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={className ? `wf-chip ${className}` : 'wf-chip'}>{children}</span>
}

/* ---------------- run id ---------------- */

/** The run handle — the copy-target an operator pastes into
 *  `workflow::status`. Rendered as a labeled mono row, never buried. */
export function RunIdRow({ runId, label }: { runId: string; label?: ReactNode }) {
  return (
    <div className="wf-run-id">
      <span className="wf-run-id-label">{label ?? 'Run'}</span>
      <span className="wf-run-id-value">{runId}</span>
    </div>
  )
}

/* ---------------- DAG summary ---------------- */

/** Compact, readable rendering of the DAG: one row per node with its model,
 *  dependency edges, fan-out source and join badge. Structure instead of a
 *  raw JSON blob. */
export function DagSummary({ def }: { def: WorkflowDef }) {
  const entries = Object.entries(def.nodes)
  const outputIds = inputSources(def.output?.from).map((s) => s.replace(/^node:/, '').split('.')[0])

  return (
    <div className="wf-dag">
      <div className="wf-dag-head">
        <span className={uiClasses.eyebrow}>dag</span>
        <span className="wf-meta">
          {entries.length} {entries.length === 1 ? 'node' : 'nodes'}
        </span>
        {outputIds.length > 0 ? (
          <span className="wf-meta wf-ghost">
            → output <span className="wf-accent">{outputIds.join(', ')}</span>
          </span>
        ) : null}
      </div>
      <ul className="wf-rows">
        {entries.map(([id, node]) => {
          const deps = node.depends_on ?? []
          const items = node.fanout?.items?.length
          return (
            <li key={id} className="wf-row">
              <span className="wf-dot">·</span>
              <div className="wf-row-body">
                <div className="wf-row-head">
                  <span className="wf-node-id">{id}</span>
                  {outputIds.includes(id) ? <WfChip className="wf-chip-output">Output</WfChip> : null}
                  {isJoinNode(node) ? <WfChip className="wf-accent">Join</WfChip> : null}
                  {node.fanout ? <WfChip>Fan-out</WfChip> : null}
                  {node.agent.model ? <WfChip>{node.agent.model}</WfChip> : null}
                </div>
                <DagEdges
                  deps={deps}
                  fanoutOver={
                    items != null
                      ? `${items} ${items === 1 ? 'item' : 'items'}`
                      : node.fanout?.over?.replace(/^node:/, '')
                  }
                />
              </div>
            </li>
          )
        })}
      </ul>
    </div>
  )
}

/** The wiring line under a node: which deps it waits on and its fan-out
 *  source. Kept terse + mono. */
function DagEdges({ deps, fanoutOver }: { deps: string[]; fanoutOver?: string }) {
  if (deps.length === 0 && !fanoutOver) return null
  return (
    <div className="wf-edges">
      {deps.length > 0 ? <span>← {deps.join(', ')}</span> : null}
      {fanoutOver ? <span>⋔ over {fanoutOver}</span> : null}
    </div>
  )
}

/* ---------------- progress bar (status view) ---------------- */

/** Thin segmented bar: done (accent) / running (accent dim) / failed (alert) /
 *  remaining (surface). A glanceable completion read for a long run. */
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
    <div className="wf-progress">
      <div data-part="done" style={{ width: pct(done) }} />
      <div data-part="running" style={{ width: pct(running) }} />
      <div data-part="failed" style={{ width: pct(failed) }} />
    </div>
  )
}

/* ---------------- small rows ---------------- */

export function GhostRow({ label, pulse }: { label: string; pulse?: boolean }) {
  return <div className={pulse ? `wf-ghost-row ${uiClasses.pulse}` : 'wf-ghost-row'}>{`· ${label}`}</div>
}

/** Renders a node result / run result that may be a string (markdown/text) or
 *  a JSON object. Strings render verbatim in a scroll box; objects pretty-print. */
export function ResultPane({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="wf-pane">
      <div className="wf-pane-head">
        <Eyebrow>{label}</Eyebrow>
      </div>
      <pre className="wf-pre">{typeof value === 'string' ? value : safeStringify(value)}</pre>
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
