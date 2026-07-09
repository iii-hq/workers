import type { SessionTriggerInfo } from '@/lib/backend/triggers'

/**
 * Pure logic behind the registered-triggers strip and its DAG view. A trigger
 * binding is an edge in a reactive pipeline: a source (a state key write, or a
 * watched session completing) fires an action (spawn a sub-agent into some
 * session, or notify this chat). This module derives that graph from the raw
 * bindings — no React, fully testable.
 */

export interface JoinMeta {
  id: string
  expect: string[]
  key?: string
  rearm?: boolean
}

export function joinMeta(trigger: SessionTriggerInfo): JoinMeta | null {
  const join = trigger.metadata?.join
  if (!join || typeof join !== 'object') return null
  const j = join as Record<string, unknown>
  if (typeof j.id !== 'string') return null
  return {
    id: j.id,
    expect: Array.isArray(j.expect)
      ? j.expect.filter((k): k is string => typeof k === 'string')
      : [],
    key: typeof j.key === 'string' ? j.key : undefined,
    rearm: typeof j.rearm === 'boolean' ? j.rearm : undefined,
  }
}

/** The reaction's model, shown wherever the row says "spawns sub-agent". */
export function reactModel(trigger: SessionTriggerInfo): string | null {
  if (trigger.functionId !== 'harness::react') return null
  const model = trigger.metadata?.model
  return typeof model === 'string' ? model : null
}

/** The reaction's opening task (the sub-agent's prompt). */
export function reactTask(trigger: SessionTriggerInfo): string | null {
  if (trigger.functionId !== 'harness::react') return null
  const task = trigger.metadata?.task
  return typeof task === 'string' ? task : null
}

/** The session this binding's reaction spawns into (explicit targets only). */
export function spawnTarget(trigger: SessionTriggerInfo): string | null {
  if (trigger.functionId !== 'harness::react') return null
  const target = trigger.metadata?.session_id
  return typeof target === 'string' ? target : null
}

/** The state key a `state`-type binding watches (`config { key, scope }`). */
export function stateWatch(
  trigger: SessionTriggerInfo,
): { scope?: string; key: string } | null {
  if (trigger.triggerType !== 'state') return null
  const config = trigger.config as Record<string, unknown> | null | undefined
  if (typeof config?.key !== 'string') return null
  return {
    key: config.key,
    scope: typeof config.scope === 'string' ? config.scope : undefined,
  }
}

/** The session whose turn events this binding watches. */
export function watchedSession(trigger: SessionTriggerInfo): string | null {
  if (
    trigger.triggerType !== 'harness::turn-completed' &&
    trigger.triggerType !== 'harness::turn-started'
  ) {
    return null
  }
  const config = trigger.config as Record<string, unknown> | null | undefined
  const watched = config?.session_id
  return typeof watched === 'string' ? watched : null
}

/** `console-9a8a0cbc-…` → `console-9a8a0cbc`; short ids pass through. */
export function shortSession(sessionId: string): string {
  return sessionId.length > 24 ? `${sessionId.slice(0, 21)}…` : sessionId
}

/* ------------------------------------------------------------------ */
/* Staged workflow (the collapsed strip's list view)                   */
/* ------------------------------------------------------------------ */

export interface TriggerUnit {
  key: string
  /** Set when this unit is a join fan-in group. */
  join: { id: string; expect: string[] } | null
  /** The bindings in the unit (exactly one unless `join` is set). */
  members: SessionTriggerInfo[]
}

export interface TriggerWorkflow {
  /** Topological stages, upstream first. */
  levels: TriggerUnit[][]
  /** False → nothing is connected; render the flat list. */
  hasStructure: boolean
}

export function buildTriggerWorkflow(
  triggers: SessionTriggerInfo[],
): TriggerWorkflow {
  const groups = new Map<string, SessionTriggerInfo[]>()
  const singles: SessionTriggerInfo[] = []
  for (const trigger of triggers) {
    const join = joinMeta(trigger)
    if (join) {
      const list = groups.get(join.id) ?? []
      list.push(trigger)
      groups.set(join.id, list)
    } else {
      singles.push(trigger)
    }
  }

  const units: TriggerUnit[] = [
    ...[...groups.entries()].map(([id, members]) => ({
      key: `join:${id}`,
      join: { id, expect: joinMeta(members[0])?.expect ?? [] },
      members,
    })),
    ...singles.map((trigger) => ({
      key: `t:${trigger.id}`,
      join: null,
      members: [trigger],
    })),
  ]

  const spawns = (unit: TriggerUnit) =>
    unit.members.map(spawnTarget).filter((s): s is string => s !== null)
  const watches = (unit: TriggerUnit) =>
    unit.members.map(watchedSession).filter((s): s is string => s !== null)

  // parents[k] = units whose spawn target this unit watches.
  const parents = new Map<string, TriggerUnit[]>()
  let hasEdge = false
  for (const child of units) {
    const watched = new Set(watches(child))
    const feeding = units.filter(
      (parent) =>
        parent.key !== child.key && spawns(parent).some((s) => watched.has(s)),
    )
    if (feeding.length > 0) hasEdge = true
    parents.set(child.key, feeding)
  }

  // Longest-path level with a visiting guard (a cycle collapses to level 0).
  const levelByKey = new Map<string, number>()
  const visiting = new Set<string>()
  const levelOf = (unit: TriggerUnit): number => {
    const known = levelByKey.get(unit.key)
    if (known !== undefined) return known
    if (visiting.has(unit.key)) return 0
    visiting.add(unit.key)
    const feeding = parents.get(unit.key) ?? []
    const level =
      feeding.length === 0 ? 0 : 1 + Math.max(...feeding.map(levelOf))
    visiting.delete(unit.key)
    levelByKey.set(unit.key, level)
    return level
  }

  const levels: TriggerUnit[][] = []
  for (const unit of units) {
    const level = levelOf(unit)
    if (!levels[level]) levels[level] = []
    levels[level].push(unit)
  }

  return {
    levels: levels.filter((l) => l.length > 0),
    hasStructure: hasEdge || groups.size > 0,
  }
}

/** Distinct sessions a stage's units wait on — the divider's "after …" label. */
export function levelWatches(units: TriggerUnit[]): string[] {
  return [
    ...new Set(
      units.flatMap((unit) =>
        unit.members.map(watchedSession).filter((s): s is string => s !== null),
      ),
    ),
  ]
}

/* ------------------------------------------------------------------ */
/* DAG model (the flow view)                                            */
/* ------------------------------------------------------------------ */

export type DagNodeKind = 'state' | 'session' | 'join' | 'owner'

export interface DagNode {
  id: string
  kind: DagNodeKind
  label: string
  /** Secondary line: the spawned model, a join's expect list, etc. */
  sub?: string
  /** For state nodes: whether the watched key exists yet (`undefined` = unknown). */
  present?: boolean
  /** Grid position, assigned by layering + barycenter ordering. */
  col: number
  row: number
}

export interface DagEdge {
  from: string
  to: string
  /** Join edges carry the predecessor's key; spawn/watch edges are unlabeled. */
  label?: string
  kind: 'spawn' | 'watch' | 'join' | 'gate'
}

export interface TriggerDag {
  nodes: DagNode[]
  edges: DagEdge[]
  cols: number
  rows: number
}

/**
 * Derive the reactive-pipeline DAG from the bindings: state keys and watched
 * sessions are sources, spawn targets and "this chat" are sinks, joins are
 * fan-in gates. Nodes are layered by longest path and ordered within each
 * layer by a barycenter pass to reduce edge crossings. Pure and deterministic.
 */
export function buildTriggerDag(
  triggers: SessionTriggerInfo[],
  opts?: { keyPresence?: Record<string, boolean> },
): TriggerDag {
  type NodeSpec = Omit<DagNode, 'col' | 'row'>
  const specs = new Map<string, NodeSpec>()
  const rawEdges: DagEdge[] = []

  const stateNodeId = (w: { scope?: string; key: string }) =>
    `state:${w.scope ? `${w.scope}/${w.key}` : w.key}`
  const sessNodeId = (s: string) => `sess:${s}`

  const ensure = (id: string, spec: NodeSpec): string => {
    const existing = specs.get(id)
    if (!existing) {
      specs.set(id, spec)
    } else {
      // Merge: keep a model sub / a known presence if a later binding has one.
      if (!existing.sub && spec.sub) existing.sub = spec.sub
      if (existing.present === undefined && spec.present !== undefined)
        existing.present = spec.present
      else if (spec.present === true) existing.present = true
    }
    return id
  }

  const sourceOf = (t: SessionTriggerInfo): string | null => {
    const w = stateWatch(t)
    if (w) {
      return ensure(stateNodeId(w), {
        id: stateNodeId(w),
        kind: 'state',
        label: w.scope ? `${w.scope}/${w.key}` : w.key,
        present: opts?.keyPresence?.[t.id],
      })
    }
    const ws = watchedSession(t)
    if (ws) {
      return ensure(sessNodeId(ws), {
        id: sessNodeId(ws),
        kind: 'session',
        label: shortSession(ws),
      })
    }
    return null
  }

  const targetOf = (t: SessionTriggerInfo): string => {
    const tgt = spawnTarget(t)
    if (tgt) {
      return ensure(sessNodeId(tgt), {
        id: sessNodeId(tgt),
        kind: 'session',
        label: shortSession(tgt),
        sub: reactModel(t) ?? undefined,
      })
    }
    return ensure('owner', { id: 'owner', kind: 'owner', label: 'this chat' })
  }

  // Group joins so a fan-in renders as predecessors → gate → one downstream.
  const joinGroups = new Map<string, SessionTriggerInfo[]>()
  for (const t of triggers) {
    const j = joinMeta(t)
    if (j) joinGroups.set(j.id, [...(joinGroups.get(j.id) ?? []), t])
  }

  for (const t of triggers) {
    if (joinMeta(t)) continue
    const src = sourceOf(t)
    const tgt = targetOf(t)
    if (!src) continue // no watchable source (e.g. cron) — omit from the DAG
    rawEdges.push({
      from: src,
      to: tgt,
      kind: watchedSession(t) ? 'watch' : 'spawn',
    })
  }

  for (const [id, members] of joinGroups) {
    const jm = joinMeta(members[0])
    const gateId = ensure(`join:${id}`, {
      id: `join:${id}`,
      kind: 'join',
      label: id,
      sub: jm?.expect.length ? `expect ${jm.expect.join(' + ')}` : undefined,
    })
    for (const m of members) {
      const src = sourceOf(m)
      if (src)
        rawEdges.push({
          from: src,
          to: gateId,
          label: joinMeta(m)?.key,
          kind: 'join',
        })
    }
    const dt = spawnTarget(members[0])
    const downstream = dt
      ? ensure(sessNodeId(dt), {
          id: sessNodeId(dt),
          kind: 'session',
          label: shortSession(dt),
          sub: reactModel(members[0]) ?? undefined,
        })
      : ensure('owner', { id: 'owner', kind: 'owner', label: 'this chat' })
    rawEdges.push({ from: gateId, to: downstream, kind: 'gate' })
  }

  // Dedupe edges (a repeated source/target/label pair is one line).
  const seen = new Set<string>()
  const edges = rawEdges.filter((e) => {
    const key = `${e.from}|${e.to}|${e.label ?? ''}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })

  const ids = [...specs.keys()]
  const incoming = new Map<string, string[]>(ids.map((id) => [id, []]))
  const outgoing = new Map<string, string[]>(ids.map((id) => [id, []]))
  for (const e of edges) {
    incoming.get(e.to)?.push(e.from)
    outgoing.get(e.from)?.push(e.to)
  }

  // Longest-path layering (cycle-safe: a back edge collapses to layer 0).
  const layer = new Map<string, number>()
  const visiting = new Set<string>()
  const layerOf = (id: string): number => {
    const known = layer.get(id)
    if (known !== undefined) return known
    if (visiting.has(id)) return 0
    visiting.add(id)
    const parents = incoming.get(id) ?? []
    const L = parents.length ? 1 + Math.max(...parents.map(layerOf)) : 0
    visiting.delete(id)
    layer.set(id, L)
    return L
  }
  for (const id of ids) layerOf(id)

  const colCount = Math.max(0, ...ids.map((id) => layer.get(id) ?? 0)) + 1
  const cols: string[][] = Array.from({ length: colCount }, () => [])
  for (const id of ids) cols[layer.get(id) ?? 0].push(id)

  // Barycenter ordering: alternate down/up sweeps, sort each column by the mean
  // row of its neighbours in the adjacent column. Cheap crossing reduction.
  const rowOf = new Map<string, number>()
  const commitRows = (col: string[]) => {
    for (let i = 0; i < col.length; i++) rowOf.set(col[i], i)
  }
  for (const col of cols) commitRows(col)
  for (let iter = 0; iter < 4; iter++) {
    const forward = iter % 2 === 0
    const order = forward
      ? cols.map((_, i) => i)
      : cols.map((_, i) => i).reverse()
    for (const c of order) {
      const neighbours = (id: string) =>
        (forward ? incoming.get(id) : outgoing.get(id)) ?? []
      cols[c] = cols[c]
        .map((id) => {
          const rows = neighbours(id)
            .map((n) => rowOf.get(n))
            .filter((x): x is number => x !== undefined)
          const bary = rows.length
            ? rows.reduce((a, b) => a + b, 0) / rows.length
            : (rowOf.get(id) ?? 0)
          return { id, bary }
        })
        .sort((a, b) => a.bary - b.bary)
        .map((x) => x.id)
      commitRows(cols[c])
    }
  }

  const nodes: DagNode[] = []
  for (let c = 0; c < cols.length; c++) {
    cols[c].forEach((id, r) => {
      const spec = specs.get(id)
      if (spec) nodes.push({ ...spec, col: c, row: r })
    })
  }
  const rows = Math.max(1, ...cols.map((c) => c.length))
  return { nodes, edges, cols: colCount, rows }
}

/* ---------------- pixel layout for the DAG ---------------- */

export const DAG = {
  pad: 16,
  nodeW: 176,
  nodeH: 42,
  /** Horizontal run between columns. */
  hGap: 60,
  /** Vertical gap between sibling nodes. */
  vGap: 14,
} as const

export interface DagNodeBox extends DagNode {
  x: number
  y: number
  w: number
  h: number
}

export interface DagEdgeGeom extends DagEdge {
  /** Source right-edge center. */
  fx: number
  fy: number
  /** Target left-edge center. */
  tx: number
  ty: number
  /** X of the vertical elbow segment. */
  midX: number
}

export interface DagLayout {
  width: number
  height: number
  boxes: DagNodeBox[]
  edges: DagEdgeGeom[]
}

export function layoutTriggerDag(dag: TriggerDag): DagLayout {
  const { pad, nodeW, nodeH, hGap, vGap } = DAG
  const step = nodeH + vGap
  const colCounts = Array.from({ length: dag.cols }, () => 0)
  for (const n of dag.nodes) colCounts[n.col]++
  const totalH = dag.rows * nodeH + (dag.rows - 1) * vGap

  const pos = (n: DagNode) => {
    const colH = colCounts[n.col] * nodeH + (colCounts[n.col] - 1) * vGap
    const yOffset = (totalH - colH) / 2
    return {
      x: pad + n.col * (nodeW + hGap),
      y: pad + yOffset + n.row * step,
    }
  }

  const boxes: DagNodeBox[] = dag.nodes.map((n) => {
    const { x, y } = pos(n)
    return { ...n, x, y, w: nodeW, h: nodeH }
  })
  const byId = new Map(boxes.map((b) => [b.id, b]))

  const edges: DagEdgeGeom[] = dag.edges.flatMap((e) => {
    const from = byId.get(e.from)
    const to = byId.get(e.to)
    if (!from || !to) return []
    const fx = from.x + from.w
    const fy = from.y + from.h / 2
    const tx = to.x
    const ty = to.y + to.h / 2
    return [{ ...e, fx, fy, tx, ty, midX: (fx + tx) / 2 }]
  })

  return {
    width: pad * 2 + dag.cols * nodeW + (dag.cols - 1) * hGap,
    height: pad * 2 + totalH,
    boxes,
    edges,
  }
}
