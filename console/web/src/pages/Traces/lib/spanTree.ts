// Pure tree construction + flattening for the waterfall view.
//
// Extracted from `WaterfallChart` so the critical-path marking and
// the depth-offset / hide / collapse-pair logic are testable without
// rendering the component. A porter that wants to wire the same
// waterfall to a different render layer (canvas, native, SVG export)
// can reuse these helpers directly.

import { isEngineRoutingPair, isEngineRoutingSpan } from './spanLabel'
import type { VisualizationSpan } from './traceTransform'

export interface SpanNode extends VisualizationSpan {
  children: SpanNode[]
  isExpanded: boolean
  isCriticalPath: boolean
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
  /** When true, engine routing spans (`handle_invocation X`, `call X` on the
   *  `iii` service) are skipped during render and their children render at
   *  the parent's depth instead. */
  hideEngineRouting: boolean
  /** When true, a `handle_invocation X` parent with a single `call X` child
   *  is rendered as ONE row, with the child's subtree promoted under the
   *  parent. The row gets `mergedRouting: true`. */
  collapseEngineRoutingPairs: boolean
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

  spans.forEach((span) => {
    const node = spanMap.get(span.span_id)
    if (!node) return
    if (span.parent_span_id && spanMap.has(span.parent_span_id)) {
      spanMap.get(span.parent_span_id)?.children.push(node)
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

  function traverse(node: SpanNode, depthOffset: number) {
    const hidden = opts.hideEngineRouting && isEngineRoutingSpan(node)

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
        traverse(child, nextOffset)
      }
    }
  }

  for (const node of nodes) {
    traverse(node, 0)
  }
  return result
}
