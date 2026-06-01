// Pure tree construction + flattening for the waterfall view.
//
// Extracted from `WaterfallChart` so the critical-path marking and
// the depth-offset / hide / collapse-pair logic are testable without
// rendering the component. A porter that wants to wire the same
// waterfall to a different render layer (canvas, native, SVG export)
// can reuse these helpers directly.

import { isEngineRoutingPair } from './spanLabel'
import type { VisualizationSpan } from './traceTransform'

const HANDLE_INVOCATION_PREFIX = 'handle_invocation ' as const
const CALL_PREFIX = 'call ' as const

function functionIdFromCallName(name: string): string | null {
  if (!name.startsWith(CALL_PREFIX)) return null
  return name.slice(CALL_PREFIX.length)
}

function functionIdFromHandleInvocationName(name: string): string | null {
  if (!name.startsWith(HANDLE_INVOCATION_PREFIX)) return null
  return name.slice(HANDLE_INVOCATION_PREFIX.length)
}

export interface SpanNode extends VisualizationSpan {
  children: SpanNode[]
  isExpanded: boolean
  isCriticalPath: boolean
}

/**
 * Whether a span should be hidden when `hideEngineRouting` is on.
 *
 * Hides engine dispatch wrappers but keeps the worker's `call X` SERVER
 * span (the row that carries invocation events/logs). Uses tree structure
 * and cross-service boundaries — not a hardcoded engine service name —
 * so it survives `OTEL_SERVICE_NAME` overrides.
 */
export function isHideableRoutingNode(
  node: Pick<SpanNode, 'name' | 'service_name' | 'children'>,
  parent: Pick<SpanNode, 'name'> | undefined,
): boolean {
  if (node.name.startsWith(HANDLE_INVOCATION_PREFIX)) return true

  const callFn = functionIdFromCallName(node.name)
  if (callFn === null) return false

  const parentFn = parent
    ? functionIdFromHandleInvocationName(parent.name)
    : null
  if (parentFn !== null && parentFn === callFn) return true

  return node.children.some((child) => {
    const childFn = functionIdFromCallName(child.name)
    if (childFn === null || childFn !== callFn) return false
    return child.service_name !== node.service_name
  })
}

export interface FlatSpanRow extends SpanNode {
  /**
   * Visible indentation depth after applying `hideEngineRouting`.
   * Equals `node.depth` minus the count of ancestors that were hidden.
   * Always >= 0.
   */
  displayDepth: number
  /**
   * True when this row absorbed an engine `call X` child that was
   * collapsed into its `handle_invocation X` parent. Renderers should
   * show a "+1" affordance to signal the merge.
   */
  mergedRouting: boolean
}

export interface FlattenOptions {
  /** Span IDs the user has expanded. Collapsed nodes hide their subtree. */
  expandedIds: Set<string>
  /** When true, engine dispatch wrappers are skipped during render and their
   *  children render at the parent's depth instead: every `handle_invocation X`,
   *  the engine's own `call X` under that wrapper, and any `call X` that has a
   *  same-named `call X` child on a different service (engine→worker RPC).
   *  The worker's innermost `call X` row is kept. */
  hideEngineRouting: boolean
  /** When true, a `handle_invocation X` parent with a single `call X` child
   *  is rendered as ONE row, with the child's subtree promoted under the
   *  parent. The row gets `mergedRouting: true`. */
  collapseEngineRoutingPairs: boolean
  /** When true, only spans on the critical path are emitted. Because the
   *  critical path is a single chain from root to leaf (each parent has at
   *  most one critical child, and non-critical siblings recursively unmark
   *  their subtrees), we can skip non-critical nodes entirely without
   *  losing visible descendants. */
  onlyCriticalPath?: boolean
}

/**
 * Build a parent/child tree from a flat list of `VisualizationSpan`s,
 * then mark the critical path.
 *
 * Parent linking: each span's `parent_span_id` is looked up in the
 * input set; spans with no parent (or whose parent isn't in the set)
 * become roots. Spans linked into the tree appear in the order they
 * were added to the flat input — caller controls ordering.
 *
 * Critical-path marking: greedy DFS from each root. A node is on the
 * critical path if its longest child path is the dominant subtree
 * (the slowest leaf-to-root chain). Tied children: first wins.
 * Non-critical siblings are recursively unmarked.
 */
export function buildSpanTree(spans: VisualizationSpan[]): SpanNode[] {
  const spanMap = new Map<string, SpanNode>()
  const roots: SpanNode[] = []

  spans.forEach((span) => {
    spanMap.set(span.span_id, {
      ...span,
      children: [],
      isExpanded: true,
      isCriticalPath: false,
    })
  })

  // Classify each span's parent chain as either acyclic-to-a-root or
  // cyclic. A span is only linked under its parent when its whole ancestor
  // chain terminates at a real root without revisiting a node; otherwise it
  // is promoted to a root. This keeps a self-parent (`parent === self`) or a
  // mutual cycle (a↔b) from (a) dropping the span out of `roots` entirely,
  // and (b) building a child cycle that would infinite-loop the
  // critical-path DFS below and `flattenTree` downstream. Mirrors the
  // `visiting` guard already used by `calculateDepths`.
  const SAFE = 1
  const CYCLIC = 2
  const chainMark = new Map<string, 1 | 2>()

  function chainReachesRoot(startId: string): boolean {
    const path: string[] = []
    let cur: string | undefined = startId
    let safe = true
    while (cur !== undefined) {
      const cached = chainMark.get(cur)
      if (cached !== undefined) {
        safe = cached === SAFE
        break
      }
      if (path.includes(cur)) {
        safe = false
        break
      }
      path.push(cur)
      const parentId: string | undefined = spanMap.get(cur)?.parent_span_id
      if (!parentId || parentId === cur || !spanMap.has(parentId)) {
        safe = true
        break
      }
      cur = parentId
    }
    for (const id of path) chainMark.set(id, safe ? SAFE : CYCLIC)
    return safe
  }

  spans.forEach((span) => {
    const node = spanMap.get(span.span_id)
    if (!node) return
    const parentId = span.parent_span_id
    if (
      parentId &&
      parentId !== span.span_id &&
      spanMap.has(parentId) &&
      chainReachesRoot(span.span_id)
    ) {
      spanMap.get(parentId)?.children.push(node)
    } else {
      roots.push(node)
    }
  })

  function markCriticalPath(node: SpanNode): number {
    if (node.children.length === 0) {
      node.isCriticalPath = true
      return node.duration_ms
    }

    let maxDuration = 0
    let criticalChild: SpanNode | null = null

    node.children.forEach((child) => {
      const duration = markCriticalPath(child)
      if (duration > maxDuration) {
        maxDuration = duration
        criticalChild = child
      }
    })

    node.isCriticalPath = true
    node.children.forEach((child) => {
      if (child !== criticalChild) {
        unmarkCriticalPath(child)
      }
    })

    return node.duration_ms + maxDuration
  }

  function unmarkCriticalPath(node: SpanNode) {
    node.isCriticalPath = false
    node.children.forEach(unmarkCriticalPath)
  }

  roots.forEach(markCriticalPath)

  return roots
}

/**
 * Flatten the tree into a render-ordered list of rows, respecting
 * collapse state and the two engine-routing affordances.
 *
 * Depth-offset rule (for `hideEngineRouting`): when a parent is
 * hidden, its visible descendants shift left by 1. Multiple stacked
 * hidden ancestors stack the offset, so a deeply nested user span
 * under three hidden routing parents renders at displayDepth = depth - 3.
 * `Math.max(0, ...)` ensures the depth never goes negative.
 *
 * Children-visibility rule: a node's children are emitted if the
 * node is in `expandedIds` OR if the node itself is hidden (in which
 * case the children take its place at the shifted depth).
 */
export function flattenTree(
  nodes: SpanNode[],
  opts: FlattenOptions,
): FlatSpanRow[] {
  const result: FlatSpanRow[] = []

  function traverse(
    node: SpanNode,
    depthOffset: number,
    parent: SpanNode | undefined,
  ) {
    if (opts.onlyCriticalPath && !node.isCriticalPath) return

    const hidden = opts.hideEngineRouting && isHideableRoutingNode(node, parent)

    let mergedRouting = false
    let descendants = node.children
    if (
      !hidden &&
      opts.collapseEngineRoutingPairs &&
      node.children.length === 1 &&
      isEngineRoutingPair(node, node.children[0])
    ) {
      mergedRouting = true
      descendants = node.children[0].children
    }

    if (!hidden) {
      result.push({
        ...node,
        displayDepth: Math.max(0, node.depth - depthOffset),
        mergedRouting,
      })
    }

    const nextOffset = hidden ? depthOffset + 1 : depthOffset
    const childrenVisible = hidden || opts.expandedIds.has(node.span_id)
    if (childrenVisible) {
      for (const child of descendants) {
        traverse(child, nextOffset, node)
      }
    }
  }

  for (const node of nodes) {
    traverse(node, 0, undefined)
  }
  return result
}
