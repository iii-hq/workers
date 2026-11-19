/**
 * The Workflow section of the chat's Context panel: the runs the chat and
 * its sub-agents started, newest first, each folding its nodes under a
 * header per phase. The console draws the header from `setSummary`.
 */

import type { ContextSectionProps, ContextSectionSummary, Host } from '@iii-dev/console-ui'
import { StatusDot } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Bot, Check, ChevronRight, LoaderCircle, X } from 'lucide-react'
import { type CSSProperties, Fragment, useEffect, useState } from 'react'
import {
  groupsByPhase,
  nodeFacts,
  nodeLabel,
  nodeLine,
  nodeMark,
  phaseCurrent,
  phaseFacts,
  phaseMark,
  phaseNodeLine,
  phaseOpens,
  phaseState,
  runFacts,
  runMark,
  soleFacts,
  soleNode,
  solePhaseNode,
  underPhase,
  type WorkflowMark,
  type WorkflowNodeView,
  type WorkflowPhaseView,
  type WorkflowRunView,
} from '../lib/run-view'
import { capRows, Row, ShowMore } from './row'
import { useWorkflowRuns } from './use-runs'

const SECTION_CAP = 3
/** A run, or one phase of it, lists this many nodes before "Show N more". */
const WORKFLOW_NODE_CAP = 8
/** A row one level in: a run's nodes, or its phases. */
const NESTED_ROW = { '--iii-ui-tree-depth': 1 } as CSSProperties
/** A row two levels in: the nodes of a phase. */
const PHASE_NODE_ROW = { '--iii-ui-tree-depth': 2 } as CSSProperties

type SectionHost = Pick<Host, 'iii' | 'chat'>

/** Adds the section where the console has the slot; `undefined` on an older console. */
export function registerWorkflowSection(host: Host): (() => void) | undefined {
  function Render(props: ContextSectionProps) {
    return <WorkflowSection {...props} host={host} />
  }
  return host.chat?.registerContextSection?.({ id: 'workflow', title: 'Workflow', order: 450, render: Render })
}

/** `1 failed · 2 completed`, leaving out the parts that are zero. */
function plural(parts: [number, string, string][]): string | undefined {
  const text = parts
    .filter(([n]) => n > 0)
    .map(([n, one, many]) => `${n} ${n === 1 ? one : many}`)
    .join(' · ')
  return text || undefined
}

export function WorkflowSection({
  host,
  sessionId,
  active,
  open = true,
  isStreaming,
  setSummary,
}: ContextSectionProps & { host: SectionHost }) {
  const workflow = useWorkflowRuns({ iii: host.iii, sessionId, active, open, isStreaming })
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const setOpen = (key: string, value: boolean) => setExpanded((current) => ({ ...current, [key]: value }))
  const workflowRows = capRows(workflow.runs, SECTION_CAP, !!expanded.workflow)
  const runsWith = (status: string) => workflow.runs.filter((run) => run.status === status).length
  const liveRuns = workflow.runs.filter((run) => run.live).length
  const failedRuns = runsWith('failed')
  const count = workflow.runs.length
  const brief = plural([
    [failedRuns, 'failed', 'failed'],
    [runsWith('cancelled'), 'cancelled', 'cancelled'],
    [runsWith('completed'), 'completed', 'completed'],
  ])
  const live = liveRuns > 0 ? `${liveRuns} running` : undefined
  const tone: ContextSectionSummary['tone'] = failedRuns > 0 ? 'alert' : undefined
  useEffect(() => {
    setSummary({ count, empty: count === 0, brief, tone, live })
  }, [setSummary, count, brief, tone, live])

  const chat = host.chat
  const onOpenNode = (id: string) => (chat?.openConversation ?? chat?.selectConversation)?.(id)
  return (
    <>
      {workflowRows.shown.map((run, index) => {
        const runKey = `workflow-run:${run.runId}`
        const runOpen = expanded[runKey] ?? (run.live || run.status === 'failed' || index === 0)
        return (
          <WorkflowRunItem
            key={run.runId}
            run={run}
            open={runOpen}
            onToggle={() => setOpen(runKey, !runOpen)}
            expanded={expanded}
            onSetOpen={setOpen}
            onOpenNode={onOpenNode}
          />
        )
      })}
      <ShowMore
        hidden={workflowRows.hidden}
        expanded={!!expanded.workflow}
        total={workflow.runs.length}
        cap={SECTION_CAP}
        onToggle={() => setOpen('workflow', !expanded.workflow)}
      />
      {workflow.earlier > 0 ? (
        <p className="wf-ctx-note">
          {workflow.earlier} earlier {workflow.earlier === 1 ? 'run' : 'runs'} not listed
        </p>
      ) : null}
    </>
  )
}

/** The dot and word at a workflow row's end, as a sub-agent row has them; no dot where a leading mark already says it. */
function WorkflowStatus({ mark, dot = true }: { mark: WorkflowMark; dot?: boolean }) {
  const quiet = mark.tone === 'ok' || mark.tone === 'ghost'
  return (
    <>
      {dot ? <StatusDot tone={mark.tone} pulse={mark.pulse} /> : null}
      {mark.text ? <span className={quiet ? uiClasses.treeItemMeta : 'wf-ctx-mark'}>{mark.text}</span> : null}
    </>
  )
}

/** A row's second line: the error in alert, else its facts. */
function Line({ error, text }: { error?: string; text?: string }) {
  if (error) return <span className="wf-ctx-truncate wf-ctx-alert">{error}</span>
  return text ? <span className="wf-ctx-truncate">{text}</span> : null
}

/**
 * One run. A run of one node is that node's row and opens its session; a
 * larger run is a row that folds its nodes, in the order they ran, under a
 * header per phase when it has two or more; a phase of one node is that
 * node's row.
 */
function WorkflowRunItem({
  run,
  open,
  onToggle,
  expanded,
  onSetOpen,
  onOpenNode,
}: {
  run: WorkflowRunView
  open: boolean
  onToggle: () => void
  /** The section's open state, by key: phases and "Show more". */
  expanded: Record<string, boolean>
  onSetOpen: (key: string, open: boolean) => void
  onOpenNode: (sessionId: string) => void
}) {
  const now = Date.now()
  const sole = soleNode(run)
  if (sole) {
    const sessionId = sole.sessionId
    const error = sole.error ?? run.error
    return (
      <Row
        icon={<Bot aria-hidden />}
        label={run.title}
        title={[run.title, sole.profile, run.runId, error].filter(Boolean).join(' · ')}
        ariaLabel={sessionId ? `Open ${run.title} in a new panel (${run.status})` : `${run.title} (${run.status})`}
        disabled={!sessionId}
        onActivate={sessionId ? () => onOpenNode(sessionId) : undefined}
        description={<Line error={error} text={soleFacts(run, sole)} />}
        meta={<WorkflowStatus mark={run.live ? nodeMark(sole, now) : runMark(run, now)} />}
      />
    )
  }
  const grouped = groupsByPhase(run)
  // A run, or each phase of it, caps its rows on its own.
  const list = (nodes: WorkflowNodeView[], phase: string, style: CSSProperties) => {
    const key = `workflow-nodes:${run.runId}:${phase}`
    const rows = capRows(nodes, WORKFLOW_NODE_CAP, !!expanded[key])
    return (
      <>
        {rows.shown.map((node) => (
          <WorkflowNodeRow
            key={node.uid}
            run={run}
            node={node}
            grouped={grouped}
            style={style}
            now={now}
            onOpenNode={onOpenNode}
          />
        ))}
        <ShowMore
          style={style}
          hidden={rows.hidden}
          expanded={!!expanded[key]}
          total={nodes.length}
          cap={WORKFLOW_NODE_CAP}
          onToggle={() => onSetOpen(key, !expanded[key])}
        />
      </>
    )
  }
  return (
    <>
      <Row
        icon={<ChevronRight aria-hidden className={open ? 'wf-ctx-chevron wf-ctx-open' : 'wf-ctx-chevron'} />}
        label={run.title}
        title={[run.title, run.runId, run.error].filter(Boolean).join(' · ')}
        ariaLabel={`${run.title} (${run.status})`}
        aria-expanded={open}
        onActivate={onToggle}
        description={<Line error={run.error} text={runFacts(run)} />}
        meta={<WorkflowStatus mark={runMark(run, now)} />}
      />
      {open ? (
        <>
          {!grouped
            ? list(
                run.phases.flatMap((phase) => phase.nodes),
                '',
                NESTED_ROW,
              )
            : run.phases.map((phase) => {
                // Nodes without a phase sit beside the headers.
                if (!phase.phase) return <Fragment key="">{list(phase.nodes, '', NESTED_ROW)}</Fragment>
                const sole = solePhaseNode(phase)
                if (sole)
                  return (
                    <WorkflowNodeRow
                      key={phase.phase}
                      run={run}
                      node={sole}
                      grouped
                      phase={phase}
                      style={NESTED_ROW}
                      now={now}
                      onOpenNode={onOpenNode}
                    />
                  )
                const key = `workflow-phase:${run.runId}:${phase.phase}`
                // A phase whose nodes the status left out has nothing to fold.
                const phaseOpen = phase.nodes.length ? (expanded[key] ?? phaseOpens(run, phase)) : undefined
                return (
                  <Fragment key={phase.phase}>
                    <WorkflowPhaseRow
                      run={run}
                      phase={phase}
                      open={phaseOpen}
                      onToggle={() => onSetOpen(key, !phaseOpen)}
                    />
                    {phaseOpen ? list(phase.nodes, phase.phase, PHASE_NODE_ROW) : null}
                  </Fragment>
                )
              })}
          {run.phases.length > 0 && run.unlisted > 0 ? (
            <p className="wf-ctx-note wf-ctx-note-nested">
              {run.unlisted} more {run.unlisted === 1 ? 'node' : 'nodes'} not listed
            </p>
          ) : null}
        </>
      ) : null}
    </>
  )
}

/** A phase's leading mark: a glyph once settled or while the run is at it, the panel's dot while it waits. */
const PHASE_MARKS = {
  running: <LoaderCircle aria-hidden className={`${uiClasses.spin} wf-ctx-accent`} />,
  failed: <X aria-hidden className="wf-ctx-alert" />,
  cancelled: <X aria-hidden className="wf-ctx-warn" />,
  done: <Check aria-hidden className="wf-ctx-ok" />,
  pending: <StatusDot tone="ghost" />,
}

/** A phase of a run: its mark, its name and done/total (in ink while current); folds its nodes. */
function WorkflowPhaseRow({
  run,
  phase,
  open,
  onToggle,
}: {
  run: WorkflowRunView
  phase: WorkflowPhaseView
  /** Unset when the phase lists no nodes: the header folds nothing. */
  open?: boolean
  onToggle: () => void
}) {
  const state = phaseState(phase)
  const mark = phaseMark(run, phase)
  const current = phaseCurrent(run, phase)
  return (
    <Row
      style={NESTED_ROW}
      icon={PHASE_MARKS[mark]}
      label={<span className={current ? undefined : 'wf-ctx-faint'}>{phase.phase}</span>}
      ariaLabel={[
        `${phase.phase} phase`,
        phase.failed > 0 && `${phase.failed} failed`,
        phase.cancelled > 0 && `${phase.cancelled} cancelled`,
        `${phase.done} of ${phase.total} done (${state})`,
      ]
        .filter(Boolean)
        .join(', ')}
      aria-expanded={open}
      aria-current={mark === 'running' ? 'step' : undefined}
      data-workflow-phase={state}
      disabled={open === undefined}
      onActivate={open === undefined ? undefined : onToggle}
      meta={<span className={current ? 'wf-ctx-mark wf-ctx-num' : uiClasses.treeItemMeta}>{phaseFacts(phase)}</span>}
    />
  )
}

/**
 * A node of a run; under a phase header its label leaves the phase out. The
 * only node of a phase stands for both: the phase's mark and name, its own line.
 */
function WorkflowNodeRow({
  run,
  node,
  grouped,
  phase,
  style,
  now,
  onOpenNode,
}: {
  run: WorkflowRunView
  node: WorkflowNodeView
  grouped: boolean
  /** The phase this node is the only one of: the row stands for both. */
  phase?: WorkflowPhaseView
  style: CSSProperties
  now: number
  onOpenNode: (sessionId: string) => void
}) {
  const named = nodeLabel(run, node)
  const current = !!phase && phaseCurrent(run, phase)
  const label = phase ? (
    <span className={current ? undefined : 'wf-ctx-faint'}>{phase.phase}</span>
  ) : grouped ? (
    underPhase(named, node.phase)
  ) : (
    named
  )
  const name = node.suffix ? `${named} ${node.suffix}` : named
  const sessionId = node.sessionId
  const mark = nodeMark(node, now)
  const leading = phase && phaseMark(run, phase)
  const line = phase ? phaseNodeLine(run, node) : (grouped ? nodeFacts(node) : nodeLine(run, node)) || node.uid
  const action = sessionId ? `Open ${name} in a new panel (${node.state})` : `${name} (${node.state})`
  return (
    <Row
      style={style}
      icon={leading ? PHASE_MARKS[leading] : <Bot aria-hidden />}
      // Passes and items share the label, so their suffix stays out of its ellipsis.
      label={
        node.suffix ? (
          <span className="wf-ctx-suffixed">
            <span className="wf-ctx-truncate">{label}</span>
            <span className="wf-ctx-suffix">{node.suffix}</span>
          </span>
        ) : (
          label
        )
      }
      title={[node.label, node.profile, node.uid, node.error].filter(Boolean).join(' · ')}
      ariaLabel={phase ? `${phase.phase} phase, ${action}` : action}
      aria-current={leading === 'running' ? 'step' : undefined}
      data-workflow-phase={phase ? phaseState(phase) : undefined}
      disabled={!sessionId}
      onActivate={sessionId ? () => onOpenNode(sessionId) : undefined}
      description={node.error || line ? <Line error={node.error} text={line} /> : undefined}
      // The leading spinner says only "going": an idle or parked node keeps its dot.
      meta={<WorkflowStatus mark={mark} dot={!phase || (node.state === 'running' && !mark.pulse)} />}
    />
  )
}
