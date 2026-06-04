// Coverage for the waterfall's tree construction + flattening. These
// are the trickiest pure functions in the feature: parent linking,
// critical-path greedy DFS, depth-offset propagation when hiding
// engine routing parents, and the handle_invocation+call pair merge.

import { describe, expect, it } from 'vitest'
import { buildSpanTree, flattenTree, type SpanNode } from './spanTree'
import type { VisualizationSpan } from './traceTransform'

function makeSpan(
  overrides: Partial<VisualizationSpan> = {},
): VisualizationSpan {
  // Engine routing fixtures (`handle_invocation X` / `call X`) carry a
  // `function_id` attribute in reality — `isEngineRoutingSpan` now requires it
  // to avoid sweeping up worker `call X` spans. Default it from the name so
  // engine-routing cases don't have to spell it out (an explicit `attributes`
  // override still wins).
  const name = overrides.name ?? 'span'
  const engineFid = /^(?:handle_invocation|call) (.+)$/.exec(name)?.[1]
  const attributes: Record<string, unknown> = engineFid
    ? { function_id: engineFid }
    : {}
  return {
    span_id: 's-1',
    trace_id: 't-1',
    name: 'span',
    duration_ms: 10,
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    status: 'ok',
    attributes,
    events: [],
    links: [],
    service_name: 'svc',
    start_time_unix_nano: 0,
    end_time_unix_nano: 0,
    ...overrides,
  } as VisualizationSpan
}

function expandAll(nodes: SpanNode[]): Set<string> {
  const ids = new Set<string>()
  function walk(n: SpanNode) {
    ids.add(n.span_id)
    n.children.forEach(walk)
  }
  nodes.forEach(walk)
  return ids
}

describe('buildSpanTree — linking', () => {
  it('returns an empty array when given no spans', () => {
    expect(buildSpanTree([])).toEqual([])
  })

  it('treats every span with no parent as a root', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a' }),
      makeSpan({ span_id: 'b' }),
      makeSpan({ span_id: 'c' }),
    ])
    expect(tree).toHaveLength(3)
    expect(tree.every((n) => n.children.length === 0)).toBe(true)
  })

  it('links children to their parent via parent_span_id', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a' }),
      makeSpan({ span_id: 'b', parent_span_id: 'a' }),
      makeSpan({ span_id: 'c', parent_span_id: 'a' }),
    ])
    expect(tree).toHaveLength(1)
    expect(tree[0].span_id).toBe('a')
    expect(tree[0].children.map((c) => c.span_id)).toEqual(['b', 'c'])
  })

  it('treats spans whose parent_span_id is missing from input as roots', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', parent_span_id: 'gone' }),
      makeSpan({ span_id: 'b', parent_span_id: 'a' }),
    ])
    expect(tree.map((r) => r.span_id)).toEqual(['a'])
    expect(tree[0].children.map((c) => c.span_id)).toEqual(['b'])
  })

  it('preserves children order based on input order', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'root' }),
      makeSpan({ span_id: 'b', parent_span_id: 'root' }),
      makeSpan({ span_id: 'a', parent_span_id: 'root' }),
      makeSpan({ span_id: 'c', parent_span_id: 'root' }),
    ])
    expect(tree[0].children.map((c) => c.span_id)).toEqual(['b', 'a', 'c'])
  })
})

describe('buildSpanTree — critical path', () => {
  it('marks the lone path as critical when the tree is linear', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', duration_ms: 100 }),
      makeSpan({ span_id: 'b', parent_span_id: 'a', duration_ms: 50 }),
      makeSpan({ span_id: 'c', parent_span_id: 'b', duration_ms: 30 }),
    ])
    expect(tree[0].isCriticalPath).toBe(true)
    expect(tree[0].children[0].isCriticalPath).toBe(true)
    expect(tree[0].children[0].children[0].isCriticalPath).toBe(true)
  })

  it('picks the slower of two sibling subtrees as critical', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'root', duration_ms: 100 }),
      makeSpan({ span_id: 'fast', parent_span_id: 'root', duration_ms: 5 }),
      makeSpan({ span_id: 'slow', parent_span_id: 'root', duration_ms: 50 }),
    ])
    const [fast, slow] = tree[0].children
    expect(slow.isCriticalPath).toBe(true)
    expect(fast.isCriticalPath).toBe(false)
  })

  it('chooses based on cumulative path duration, not just immediate child duration', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'root', duration_ms: 100 }),
      // Branch A: short immediate but long descendant chain.
      makeSpan({ span_id: 'a', parent_span_id: 'root', duration_ms: 10 }),
      makeSpan({ span_id: 'a-deep', parent_span_id: 'a', duration_ms: 200 }),
      // Branch B: longer immediate but shorter total.
      makeSpan({ span_id: 'b', parent_span_id: 'root', duration_ms: 50 }),
    ])
    const [a, b] = tree[0].children
    // A's cumulative path is 10 + 200 = 210; B's is 50. A wins.
    expect(a.isCriticalPath).toBe(true)
    expect(a.children[0].isCriticalPath).toBe(true)
    expect(b.isCriticalPath).toBe(false)
  })

  it('unmarks non-critical siblings recursively (their descendants too)', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'root', duration_ms: 100 }),
      makeSpan({ span_id: 'slow', parent_span_id: 'root', duration_ms: 100 }),
      makeSpan({ span_id: 'fast', parent_span_id: 'root', duration_ms: 5 }),
      makeSpan({
        span_id: 'fast-child',
        parent_span_id: 'fast',
        duration_ms: 1,
      }),
    ])
    const fast = tree[0].children.find((c) => c.span_id === 'fast') as SpanNode
    expect(fast.isCriticalPath).toBe(false)
    expect(fast.children[0].isCriticalPath).toBe(false)
  })
})

describe('flattenTree — basic ordering', () => {
  it('emits one row per node in DFS pre-order when all expanded', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', depth: 0 }),
      makeSpan({ span_id: 'b', parent_span_id: 'a', depth: 1 }),
      makeSpan({ span_id: 'c', parent_span_id: 'b', depth: 2 }),
      makeSpan({ span_id: 'd', parent_span_id: 'a', depth: 1 }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: false,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['a', 'b', 'c', 'd'])
  })

  it('hides descendants of a collapsed node', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', depth: 0 }),
      makeSpan({ span_id: 'b', parent_span_id: 'a', depth: 1 }),
      makeSpan({ span_id: 'c', parent_span_id: 'b', depth: 2 }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: new Set(['a']),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: false,
    })
    // 'a' is expanded, 'b' is not in expandedIds -> only 'a' and 'b'.
    expect(flat.map((r) => r.span_id)).toEqual(['a', 'b'])
  })

  it('preserves displayDepth from node.depth when no hide is in effect', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', depth: 0 }),
      makeSpan({ span_id: 'b', parent_span_id: 'a', depth: 1 }),
      makeSpan({ span_id: 'c', parent_span_id: 'b', depth: 2 }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: false,
    })
    expect(flat.map((r) => r.displayDepth)).toEqual([0, 1, 2])
  })
})

describe('flattenTree — hideEngineRouting depth offset', () => {
  it('skips engine routing spans and shifts descendants left by 1', () => {
    // iii / handle_invocation X is a routing span; its children shift up.
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'root',
        service_name: 'iii',
        name: 'handle_invocation fn',
        depth: 0,
      }),
      makeSpan({
        span_id: 'user',
        parent_span_id: 'root',
        service_name: 'billing',
        name: 'charge',
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: false,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['user'])
    expect(flat[0].displayDepth).toBe(0) // 1 - 1 (one hidden ancestor)
  })

  it('stacks the offset for multiple hidden ancestors', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'r1',
        service_name: 'iii',
        name: 'handle_invocation a',
        depth: 0,
      }),
      makeSpan({
        span_id: 'r2',
        parent_span_id: 'r1',
        service_name: 'iii',
        name: 'call a',
        depth: 1,
      }),
      makeSpan({
        span_id: 'user',
        parent_span_id: 'r2',
        service_name: 'billing',
        name: 'charge',
        depth: 2,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: false,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['user'])
    expect(flat[0].displayDepth).toBe(0) // 2 - 2 (two hidden ancestors)
  })

  it('clamps displayDepth to 0 (never negative)', () => {
    // Pathological: a non-routing span at depth 0 under a hidden parent.
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'r',
        service_name: 'iii',
        name: 'handle_invocation x',
        depth: 0,
      }),
      makeSpan({
        span_id: 'user',
        parent_span_id: 'r',
        service_name: 'billing',
        name: 'charge',
        depth: 0,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: false,
    })
    expect(flat[0].displayDepth).toBe(0)
  })

  it('leaves non-engine spans unchanged when hideEngineRouting is on', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'r',
        service_name: 'billing',
        name: 'charge',
        depth: 0,
      }),
      makeSpan({
        span_id: 'c',
        parent_span_id: 'r',
        service_name: 'billing',
        name: 'inner',
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: false,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['r', 'c'])
    expect(flat.map((r) => r.displayDepth)).toEqual([0, 1])
  })
})

describe('flattenTree — collapseEngineRoutingPairs', () => {
  it('merges a handle_invocation X parent with its single call X child', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'h',
        service_name: 'iii',
        name: 'handle_invocation fn',
        depth: 0,
      }),
      makeSpan({
        span_id: 'c',
        parent_span_id: 'h',
        service_name: 'iii',
        name: 'call fn',
        depth: 1,
      }),
      makeSpan({
        span_id: 'inner',
        parent_span_id: 'c',
        service_name: 'billing',
        name: 'charge',
        depth: 2,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: true,
    })
    // 'c' is absorbed; 'h' renders with mergedRouting=true, and inner
    // becomes h's effective grandchild rendered at its own depth.
    expect(flat.map((r) => r.span_id)).toEqual(['h', 'inner'])
    expect(flat[0].mergedRouting).toBe(true)
    expect(flat[1].mergedRouting).toBe(false)
  })

  it('does NOT merge when the parent has multiple children', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'h',
        service_name: 'iii',
        name: 'handle_invocation fn',
        depth: 0,
      }),
      makeSpan({
        span_id: 'c1',
        parent_span_id: 'h',
        service_name: 'iii',
        name: 'call fn',
        depth: 1,
      }),
      makeSpan({
        span_id: 'c2',
        parent_span_id: 'h',
        service_name: 'iii',
        name: 'call fn',
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: true,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['h', 'c1', 'c2'])
    expect(flat.every((r) => !r.mergedRouting)).toBe(true)
  })

  it('does NOT merge when the function names mismatch', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'h',
        service_name: 'iii',
        name: 'handle_invocation foo',
        depth: 0,
      }),
      makeSpan({
        span_id: 'c',
        parent_span_id: 'h',
        service_name: 'iii',
        name: 'call bar',
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: true,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['h', 'c'])
  })

  it('hide takes precedence over merge — a hidden parent does not get mergedRouting', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'h',
        service_name: 'iii',
        name: 'handle_invocation fn',
        depth: 0,
      }),
      makeSpan({
        span_id: 'c',
        parent_span_id: 'h',
        service_name: 'iii',
        name: 'call fn',
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: true,
    })
    // Both routing spans hide; tree is empty.
    expect(flat).toEqual([])
  })
})

describe('flattenTree — onlyCriticalPath', () => {
  it('emits the full chain when the tree is linear (every node is critical)', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'a', duration_ms: 100, depth: 0 }),
      makeSpan({
        span_id: 'b',
        parent_span_id: 'a',
        duration_ms: 50,
        depth: 1,
      }),
      makeSpan({
        span_id: 'c',
        parent_span_id: 'b',
        duration_ms: 30,
        depth: 2,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: false,
      onlyCriticalPath: true,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['a', 'b', 'c'])
  })

  it('skips non-critical siblings and their subtrees', () => {
    const tree = buildSpanTree([
      makeSpan({ span_id: 'root', duration_ms: 100, depth: 0 }),
      makeSpan({
        span_id: 'fast',
        parent_span_id: 'root',
        duration_ms: 5,
        depth: 1,
      }),
      makeSpan({
        span_id: 'fast-child',
        parent_span_id: 'fast',
        duration_ms: 1,
        depth: 2,
      }),
      makeSpan({
        span_id: 'slow',
        parent_span_id: 'root',
        duration_ms: 80,
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: false,
      collapseEngineRoutingPairs: false,
      onlyCriticalPath: true,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['root', 'slow'])
  })

  it('composes with hideEngineRouting — critical user span still depth-shifts', () => {
    const tree = buildSpanTree([
      makeSpan({
        span_id: 'r',
        service_name: 'iii',
        name: 'handle_invocation fn',
        duration_ms: 100,
        depth: 0,
      }),
      makeSpan({
        span_id: 'user',
        parent_span_id: 'r',
        service_name: 'billing',
        name: 'charge',
        duration_ms: 80,
        depth: 1,
      }),
    ])
    const flat = flattenTree(tree, {
      expandedIds: expandAll(tree),
      hideEngineRouting: true,
      collapseEngineRoutingPairs: false,
      onlyCriticalPath: true,
    })
    expect(flat.map((r) => r.span_id)).toEqual(['user'])
    expect(flat[0].displayDepth).toBe(0)
  })
})
