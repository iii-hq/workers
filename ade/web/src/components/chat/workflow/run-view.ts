import { formatDuration, formatRelative } from '@iii-dev/console-ui/format'
import { z } from 'zod'
import { formatTokenCount } from '@/lib/token-estimate'
import type { Conversation, Message } from '@/types/chat'
import {
  consumedDeps,
  type DagCounts,
  type NodeState,
  nodeStateSchema,
  type RunStatus,
  safeParseRequest,
  safeParseResponse,
  startRequestSchema,
  startResponseSchema,
  statusResponseSchema,
  tallyNodes,
  type WorkflowDef,
} from './parsers'

/* ---------------- workflow::status {details:true} ---------------- */

const count = z.number().nullish()

const nodeDetailSchema = z
  .object({
    state: nodeStateSchema,
    label: z.string().nullish(),
    phase: z.string().nullish(),
    model: z.string().nullish(),
    profile: z.string().nullish(),
    session_id: z.string().nullish(),
    attempt: count,
    started_at: count,
    finished_at: count,
    last_generate_at: count,
    harness_status: z.string().nullish(),
    expects_wake: z.boolean().nullish(),
    usage: z
      .object({
        complete: z.boolean().nullish(),
        input_tokens: count,
        output_tokens: count,
        cache_read_tokens: count,
        cache_write_tokens: count,
        cost_usd: count,
      })
      .passthrough()
      .nullish(),
  })
  .passthrough()

const phaseCountsSchema = z
  .object({ total: z.number(), done: z.number() })
  .passthrough()

export const runDetailsSchema = statusResponseSchema.extend({
  created_at: count,
  updated_at: count,
  phases: z.record(z.string(), phaseCountsSchema).nullish(),
  node_details: z.record(z.string(), nodeDetailSchema).nullish(),
})
export type RunDetails = z.infer<typeof runDetailsSchema>
type NodeDetail = z.infer<typeof nodeDetailSchema>

/** A snapshot the panel can show: `null` for an unknown (collected) run. */
export function parseRunDetails(
  value: unknown,
): (RunDetails & { status: RunStatus }) | null {
  const parsed = safeParseResponse(runDetailsSchema, value)
  return parsed?.status ? { ...parsed, status: parsed.status } : null
}

export const isLiveRun = (status: RunStatus) =>
  status === 'running' || status === 'awaiting_nodes'

/* ---------------- which runs a chat started ---------------- */

export interface WorkflowRunRef {
  runId: string
  /** When it was started (the call, or its first node session). */
  at: number
  /** The agent's own words for the start call. */
  title?: string
  /** The definition, when the start call is loaded: orders waiting nodes. */
  def?: WorkflowDef
}

const startRefs = new WeakMap<object, WorkflowRunRef | null>()

function startRef(message: Message): WorkflowRunRef | null {
  if (message.role !== 'function-call') return null
  if (message.functionId !== 'workflow::start') return null
  const cached = startRefs.get(message)
  if (cached !== undefined) return cached
  const runId = safeParseResponse(startResponseSchema, message.output)?.run_id
  const ref = runId
    ? {
        runId,
        at: message.createdAt,
        title: message.description,
        def: safeParseRequest(startRequestSchema, message.input)?.definition,
      }
    : null
  startRefs.set(message, ref)
  return ref
}

/**
 * The runs of a chat, oldest first: every `workflow::start` result in its
 * transcript, plus the runs its node sessions name (`workflow_run_id`), which
 * also covers a start call a paged read left out and a sub-agent's runs.
 */
export function collectWorkflowRuns(
  messages: readonly Message[],
  sessions: readonly Conversation[],
): WorkflowRunRef[] {
  const runs = new Map<string, WorkflowRunRef>()
  for (const message of messages) {
    const ref = startRef(message)
    if (ref && !runs.has(ref.runId)) runs.set(ref.runId, ref)
  }
  for (const session of sessions) {
    const runId = session.sessionMetadata?.workflow_run_id
    if (typeof runId !== 'string' || !runId) continue
    const known = runs.get(runId)
    if (!known) runs.set(runId, { runId, at: session.createdAt })
    else if (!known.def && session.createdAt < known.at)
      runs.set(runId, { ...known, at: session.createdAt })
  }
  return [...runs.values()].sort((a, b) => a.at - b.at)
}

/* ---------------- one run, the way a person follows it ---------------- */

/** `v#it2` → loop pass 2 of `v`; `map#3` → fanout item 3 of `map`. */
export function splitNodeUid(uid: string): {
  base: string
  suffix?: string
  index: number
} {
  const match = /^(.+)#(it)?(\d+)$/.exec(uid)
  if (!match) return { base: uid, index: 0 }
  return {
    base: match[1],
    suffix: `#${match[2] ?? ''}${match[3]}`,
    index: Number(match[3]),
  }
}

/** How many nodes a node waits behind: 0 for a source. */
export function nodeDepths(def: WorkflowDef): Record<string, number> {
  const depths: Record<string, number> = {}
  const depth = (id: string, seen: ReadonlySet<string>): number => {
    if (id in depths) return depths[id]
    const node = def.nodes[id]
    if (!node || seen.has(id)) return 0
    const deps = [
      ...(node.depends_on ?? []),
      ...consumedDeps(node.input?.from),
      ...consumedDeps(node.fanout?.over),
    ]
    const next = new Set(seen).add(id)
    depths[id] = deps.length
      ? 1 + Math.max(...deps.map((dep) => depth(dep, next)))
      : 0
    return depths[id]
  }
  for (const id of Object.keys(def.nodes)) depth(id, new Set())
  return depths
}

export interface WorkflowNodeView {
  uid: string
  label: string
  /** `#it2` for a loop pass, `#3` for a fanout item. */
  suffix?: string
  /** Empty for a node without a phase. */
  phase: string
  state: NodeState
  model?: string
  profile?: string
  sessionId?: string
  startedAt?: number
  finishedAt?: number
  lastGenerateAt?: number
  /** Running, its session waiting on an armed wake. */
  parked: boolean
  /** Running, its turn waiting on deferred calls: quiet, not idle. */
  awaitingFunctions: boolean
  tokens: number | null
  costUsd: number | null
  error?: string
}

export interface WorkflowPhaseView {
  phase: string
  done: number
  total: number
  nodes: WorkflowNodeView[]
}

export interface WorkflowRunView {
  runId: string
  title: string
  status: RunStatus
  live: boolean
  createdAt?: number
  updatedAt?: number
  counts: DagCounts
  tokens: number | null
  costUsd: number | null
  error?: string
  /** In order of first appearance; empty until a read with details. */
  phases: WorkflowPhaseView[]
  nodeCount: number
  /** Nodes the status left without details (it describes at most 100). */
  unlisted: number
}

function usageTokens(usage: NodeDetail['usage']): number | null {
  if (!usage) return null
  return (
    (usage.input_tokens ?? 0) +
    (usage.output_tokens ?? 0) +
    (usage.cache_read_tokens ?? 0) +
    (usage.cache_write_tokens ?? 0)
  )
}

/**
 * A run's name from its node labels, for a run whose start call the transcript
 * page left out: the one label, the words the labels start with, or the first
 * label and how many others.
 */
export function labelTitle(
  nodes: readonly { label: string }[],
): string | undefined {
  const labels = [...new Set(nodes.map((node) => node.label))]
  if (labels.length <= 1) return labels[0]
  let shared = labels[0]
  for (const label of labels) {
    while (!label.startsWith(shared)) shared = shared.slice(0, -1)
  }
  // Keep whole words only: `KAN-` out of `KAN-6 …` and `KAN-7 …` names nothing.
  if (/\w/.test(labels[0].charAt(shared.length))) {
    shared = shared.slice(0, Math.max(0, shared.lastIndexOf(' ')))
  }
  shared = shared.replace(/[\s·:,;-]+$/, '')
  return shared.length >= 3 ? shared : `${labels[0]} +${labels.length - 1}`
}

const sum = (values: (number | null)[]): number | null => {
  const known = values.filter((value) => value !== null)
  return known.length ? known.reduce((a, b) => a + b, 0) : null
}

export function buildRunView(
  ref: WorkflowRunRef,
  status: RunDetails & { status: RunStatus },
): WorkflowRunView {
  const depths = ref.def ? nodeDepths(ref.def) : {}
  const details = Object.entries(status.node_details ?? {})
  const nodes = details
    .map(([uid, detail]) => {
      const { base, suffix, index } = splitNodeUid(uid)
      const view: WorkflowNodeView = {
        uid,
        label: detail.label ?? base,
        suffix,
        phase: detail.phase ?? '',
        state: detail.state,
        model: detail.model ?? undefined,
        profile: detail.profile ?? undefined,
        // Only a send that reached the harness stamps `started_at`; a reserved node's session does not exist yet.
        sessionId:
          detail.started_at != null
            ? (detail.session_id ?? undefined)
            : undefined,
        startedAt: detail.started_at ?? undefined,
        finishedAt: detail.finished_at ?? undefined,
        lastGenerateAt: detail.last_generate_at ?? undefined,
        parked: detail.state === 'running' && detail.expects_wake === true,
        awaitingFunctions: detail.harness_status === 'awaiting_functions',
        tokens: usageTokens(detail.usage),
        costUsd: detail.usage?.cost_usd ?? null,
        error: status.node_errors?.[uid],
      }
      return { view, base, index, depth: depths[base] ?? 0 }
    })
    // Started nodes in start order; waiting ones behind them, sources first.
    .sort(
      (a, b) =>
        (a.view.startedAt ?? Number.POSITIVE_INFINITY) -
          (b.view.startedAt ?? Number.POSITIVE_INFINITY) ||
        a.depth - b.depth ||
        a.base.localeCompare(b.base, undefined, { numeric: true }) ||
        a.index - b.index,
    )
    .map((node) => node.view)

  const phases: WorkflowPhaseView[] = []
  for (const node of nodes) {
    let group = phases.find((phase) => phase.phase === node.phase)
    if (!group) {
      const counted = status.phases?.[node.phase]
      group = {
        phase: node.phase,
        done: counted?.done ?? 0,
        total: counted?.total ?? 0,
        nodes: [],
      }
      phases.push(group)
    }
    group.nodes.push(node)
  }
  // Nodes without a phase are not in the status's `phases`: count them here.
  for (const group of phases) {
    if (group.total > 0) continue
    group.total = group.nodes.length
    group.done = group.nodes.filter((node) => node.state === 'done').length
  }

  const nodeCount = Object.keys(status.nodes).length
  return {
    runId: ref.runId,
    title: ref.title ?? labelTitle(nodes) ?? `Run ${ref.runId.slice(0, 10)}`,
    status: status.status,
    live: isLiveRun(status.status),
    createdAt: status.created_at ?? undefined,
    updatedAt: status.updated_at ?? undefined,
    counts: tallyNodes(status.nodes),
    tokens: sum(nodes.map((node) => node.tokens)),
    costUsd: sum(nodes.map((node) => node.costUsd)),
    error: status.result_error ?? undefined,
    phases,
    nodeCount,
    unlisted: Math.max(0, nodeCount - nodes.length),
  }
}

/* ---------------- row copy ---------------- */

export type WorkflowTone = 'accent' | 'alert' | 'warn' | 'ink' | 'ok' | 'ghost'
export interface WorkflowMark {
  tone: WorkflowTone
  pulse?: boolean
  text: string
}

/** A running node counts as idle once its model has been quiet this long. */
const IDLE_AFTER_MS = 60_000

/** The dot and word at a node row's end: how long it took, or what it waits on. */
export function nodeMark(node: WorkflowNodeView, now: number): WorkflowMark {
  if (node.state === 'done') {
    return {
      tone: 'ok',
      text:
        node.startedAt !== undefined && node.finishedAt !== undefined
          ? formatDuration(node.finishedAt - node.startedAt)
          : 'done',
    }
  }
  if (node.state === 'failed') return { tone: 'alert', text: 'failed' }
  if (node.state === 'cancelled') return { tone: 'warn', text: 'cancelled' }
  if (node.state === 'pending') return { tone: 'ghost', text: 'pending' }
  if (node.parked) return { tone: 'ink', text: 'parked' }
  if (
    !node.awaitingFunctions &&
    node.lastGenerateAt !== undefined &&
    now - node.lastGenerateAt >= IDLE_AFTER_MS
  )
    return {
      tone: 'warn',
      text: `idle ${formatRelative(node.lastGenerateAt, now)}`,
    }
  return {
    tone: 'accent',
    pulse: true,
    text:
      node.startedAt !== undefined
        ? formatRelative(node.startedAt, now)
        : 'running',
  }
}

/** The dot and word at a run row's end. */
export function runMark(run: WorkflowRunView, now: number): WorkflowMark {
  if (run.live) return { tone: 'accent', pulse: true, text: 'running' }
  if (run.status === 'failed') return { tone: 'alert', text: 'failed' }
  if (run.status === 'cancelled') return { tone: 'warn', text: 'cancelled' }
  return {
    tone: 'ok',
    text: run.updatedAt !== undefined ? formatRelative(run.updatedAt, now) : '',
  }
}

const formatUsd = (usd: number) => `$${usd.toFixed(usd < 1 ? 3 : 2)}`

const facts = (parts: (string | null | undefined | false)[]) =>
  parts.filter(Boolean).join(' · ')

/** `codex/gpt-6-luna · 36k tokens · $0.42`; the profile goes in the row's tooltip. */
export function nodeFacts(node: WorkflowNodeView): string {
  return facts([
    node.model,
    node.tokens !== null && `${formatTokenCount(node.tokens)} tokens`,
    node.costUsd !== null && node.costUsd > 0 && formatUsd(node.costUsd),
  ])
}

/**
 * The one node of a run that has nothing else to show: a single node, no loop
 * pass or fanout item, none left out. Such a run reads as one row.
 */
export function soleNode(run: WorkflowRunView): WorkflowNodeView | null {
  const nodes = run.phases.flatMap((phase) => phase.nodes)
  if (nodes.length !== 1 || run.unlisted > 0 || run.nodeCount !== 1) return null
  return nodes[0]
}

/** A one-node run's line: the node's model, tokens and cost, then how long the run took. */
export function soleFacts(
  run: WorkflowRunView,
  node: WorkflowNodeView,
): string {
  return facts([
    nodeFacts(node),
    !run.live &&
      run.createdAt !== undefined &&
      run.updatedAt !== undefined &&
      formatDuration(run.updatedAt - run.createdAt),
  ])
}

/**
 * A node's label under its run: the run's name left out when the label starts
 * with it (`KAN-9 batch r1w1 tests` under `KAN-9 batch r1w1` reads `tests`).
 */
export function nodeLabel(
  run: WorkflowRunView,
  node: WorkflowNodeView,
): string {
  const rest = node.label.startsWith(run.title)
    ? node.label.slice(run.title.length).replace(/^[\s·:,;-]+/, '')
    : ''
  return rest || node.label
}

/**
 * What a node row says under its label: its phase first when the run has
 * several and the label does not already name it, then its facts.
 */
export function nodeLine(run: WorkflowRunView, node: WorkflowNodeView): string {
  const named = nodeLabel(run, node).toLowerCase()
  const phase =
    run.phases.length > 1 &&
    node.phase &&
    !named.startsWith(node.phase.toLowerCase()) &&
    node.phase
  return facts([phase, nodeFacts(node)])
}

/**
 * `test → check · 290k tokens · 1m 52s` for a finished run, `2/4 done ·
 * 1 running · 1 failed · 290k tokens` for one still going or with failures.
 */
export function runFacts(run: WorkflowRunView): string {
  const { counts } = run
  const failed = counts.failed + counts.cancelled
  // Totals over the described nodes only: the status details at most 100.
  const atLeast = run.unlisted > 0 ? '≥' : ''
  const settled = counts.done === counts.total && failed === 0
  const named = run.phases.map((phase) => phase.phase).filter(Boolean)
  const shape = !settled
    ? `${counts.done}/${counts.total} done`
    : named.length > 1
      ? named.join(' → ')
      : counts.total > 1
        ? `${counts.total} nodes`
        : null
  return facts([
    shape,
    counts.running > 0 && `${counts.running} running`,
    failed > 0 && `${failed} failed`,
    run.tokens !== null && `${atLeast}${formatTokenCount(run.tokens)} tokens`,
    run.costUsd !== null &&
      run.costUsd > 0 &&
      `${atLeast}${formatUsd(run.costUsd)}`,
    !run.live &&
      run.createdAt !== undefined &&
      run.updatedAt !== undefined &&
      formatDuration(run.updatedAt - run.createdAt),
  ])
}
