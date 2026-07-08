import { describe, expect, it } from 'vitest'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import {
  buildTriggerDag,
  type DagNode,
  layoutTriggerDag,
} from './trigger-graph'

function trigger(over: Partial<SessionTriggerInfo>): SessionTriggerInfo {
  return {
    id: over.id ?? `t_${Math.random().toString(36).slice(2, 8)}`,
    triggerType: 'state',
    functionId: 'harness::react',
    config: {},
    configSummary: '',
    ...over,
  }
}

/** A state binding: on scope/key, spawn model into target. */
function stateSpawn(
  id: string,
  key: string,
  target: string,
  model = 'claude-opus-4-8',
): SessionTriggerInfo {
  return trigger({
    id,
    triggerType: 'state',
    config: { scope: 'wiki', key },
    metadata: { model, session_id: target, task: 't' },
  })
}

/** A turn-completed binding: when `watch` completes, spawn model into target. */
function turnSpawn(
  id: string,
  watch: string,
  target: string,
  model = 'claude-opus-4-8',
): SessionTriggerInfo {
  return trigger({
    id,
    triggerType: 'harness::turn-completed',
    config: { session_id: watch },
    metadata: { model, session_id: target, task: 't' },
  })
}

function node(dag: ReturnType<typeof buildTriggerDag>, id: string): DagNode {
  const n = dag.nodes.find((x) => x.id === id)
  if (!n)
    throw new Error(
      `node ${id} not found; have ${dag.nodes.map((x) => x.id).join(', ')}`,
    )
  return n
}

describe('buildTriggerDag', () => {
  it('builds source→target edges and layers a state→session→session chain', () => {
    const dag = buildTriggerDag([
      stateSpawn('a', 'article_fetched', 'analyst1'),
      turnSpawn('b', 'analyst1', 'analyst1-deep'),
    ])
    // three nodes: the state key, analyst1, analyst1-deep
    expect(dag.nodes.map((n) => n.id).sort()).toEqual([
      'sess:analyst1',
      'sess:analyst1-deep',
      'state:wiki/article_fetched',
    ])
    // layered left→right
    expect(node(dag, 'state:wiki/article_fetched').col).toBe(0)
    expect(node(dag, 'sess:analyst1').col).toBe(1)
    expect(node(dag, 'sess:analyst1-deep').col).toBe(2)
    expect(dag.cols).toBe(3)
    // the spawned session carries the model as its sub
    expect(node(dag, 'sess:analyst1').sub).toBe('claude-opus-4-8')
    // edges chain through
    expect(dag.edges).toEqual([
      {
        from: 'state:wiki/article_fetched',
        to: 'sess:analyst1',
        kind: 'spawn',
      },
      { from: 'sess:analyst1', to: 'sess:analyst1-deep', kind: 'watch' },
    ])
  })

  it('dedupes a shared session node across bindings', () => {
    // Two bindings watch the SAME session analyst1 → one node, two out-edges.
    const dag = buildTriggerDag([
      stateSpawn('a', 'article_fetched', 'analyst1'),
      turnSpawn('b', 'analyst1', 'deep-1'),
      turnSpawn('c', 'analyst1', 'deep-2'),
    ])
    expect(dag.nodes.filter((n) => n.id === 'sess:analyst1')).toHaveLength(1)
    const out = dag.edges.filter((e) => e.from === 'sess:analyst1')
    expect(out.map((e) => e.to).sort()).toEqual(['sess:deep-1', 'sess:deep-2'])
  })

  it('models a join as predecessors → gate → one downstream', () => {
    const join = { id: 'J', expect: ['sum', 'fact'] }
    const dag = buildTriggerDag([
      turnSpawn('a', 'analyst1', 'reporter'),
      // two predecessors of join J, sharing the downstream spawn target
      trigger({
        id: 'm1',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'analyst1' },
        metadata: {
          model: 'm',
          session_id: 'reporter',
          task: 't',
          join: { ...join, key: 'sum' },
        },
      }),
      trigger({
        id: 'm2',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'analyst2' },
        metadata: {
          model: 'm',
          session_id: 'reporter',
          task: 't',
          join: { ...join, key: 'fact' },
        },
      }),
    ])
    const gate = dag.nodes.find((n) => n.kind === 'join')
    expect(gate?.id).toBe('join:J')
    expect(gate?.sub).toBe('expect sum + fact')
    // predecessors point into the gate, labeled by their key
    const intoGate = dag.edges.filter((e) => e.to === 'join:J')
    expect(intoGate.map((e) => e.label).sort()).toEqual(['fact', 'sum'])
    expect(intoGate.every((e) => e.kind === 'join')).toBe(true)
    // one gate→downstream edge
    const outGate = dag.edges.filter((e) => e.from === 'join:J')
    expect(outGate).toEqual([
      { from: 'join:J', to: 'sess:reporter', kind: 'gate' },
    ])
  })

  it('routes a targetless reaction (and notify) into the owner chat node', () => {
    const dag = buildTriggerDag([
      trigger({
        id: 'a',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'reviewer' },
        metadata: { model: 'm', task: 't' }, // no session_id → this chat
      }),
    ])
    expect(node(dag, 'owner').kind).toBe('owner')
    // turn-completed source → 'watch' (state sources are 'spawn').
    expect(dag.edges).toEqual([
      { from: 'sess:reviewer', to: 'owner', kind: 'watch' },
    ])
  })

  it('stamps state-key presence for coloring', () => {
    const dag = buildTriggerDag([stateSpawn('a', 'summary', 'analyst1')], {
      keyPresence: { a: false },
    })
    expect(node(dag, 'state:wiki/summary').present).toBe(false)
  })

  it('a watch cycle collapses to layer 0 instead of hanging', () => {
    const dag = buildTriggerDag([
      turnSpawn('a', 's-b', 's-a'),
      turnSpawn('b', 's-a', 's-b'),
    ])
    // does not throw; every node gets a finite column
    expect(dag.nodes.every((n) => Number.isFinite(n.col))).toBe(true)
  })
})

describe('layoutTriggerDag', () => {
  it('positions columns left-to-right with non-overlapping geometry', () => {
    const dag = buildTriggerDag([
      stateSpawn('a', 'article_fetched', 'analyst1'),
      turnSpawn('b', 'analyst1', 'analyst1-deep'),
    ])
    const layout = layoutTriggerDag(dag)
    expect(layout.width).toBeGreaterThan(0)
    expect(layout.height).toBeGreaterThan(0)
    expect(layout.boxes).toHaveLength(3)
    // each edge resolves to concrete endpoints flowing rightward
    for (const e of layout.edges) {
      expect(e.tx).toBeGreaterThanOrEqual(e.fx)
    }
    // columns strictly increase in x
    const xByCol = new Map(layout.boxes.map((b) => [b.col, b.x]))
    expect((xByCol.get(1) ?? 0) > (xByCol.get(0) ?? 0)).toBe(true)
    expect((xByCol.get(2) ?? 0) > (xByCol.get(1) ?? 0)).toBe(true)
  })
})
