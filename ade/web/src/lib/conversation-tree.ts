/**
 * Builds the main-chat → sub-agent hierarchy from the flat conversation list.
 *
 * Spawned sub-agents are real sessions whose `SessionMeta.metadata` carries
 * `parent_session_id` (stamped by the harness at spawn — see
 * `harness/src/subagent.rs`). `use-conversations` surfaces that as
 * `Conversation.parentId`; here we fold the flat list into a tree.
 *
 * Robustness: a child whose parent isn't loaded (off the 200-row page or
 * deleted) becomes a root so nothing ever disappears, and a parent cycle
 * (A→B→A) drops the involved nodes to roots rather than looping.
 */

import { isAwaitingAnswer } from '@/lib/chat-activity'
import type { Conversation } from '@/types/chat'

export interface ConvNode {
  conversation: Conversation
  children: ConvNode[]
  /** 0 = root/orchestrator; increments per nesting level. */
  depth: number
}

export interface FlatRow {
  conversation: Conversation
  depth: number
  hasChildren: boolean
}

/**
 * The fold under a parent's live sub-agents that holds its inactive ones:
 * "Show N inactive" while they are hidden, "Hide inactive" once shown.
 */
export interface InactiveToggleRow {
  kind: 'inactive-toggle'
  parentId: string
  /** The sub-agents' depth, so the toggle lines up with them. */
  depth: number
  inactive: number
  expanded: boolean
}

export type TreeRow = FlatRow | InactiveToggleRow

export function isInactiveToggle(row: TreeRow): row is InactiveToggleRow {
  return 'kind' in row && row.kind === 'inactive-toggle'
}

/**
 * A sub-agent that still wants a look: one working, one waiting on the
 * person's answer, or one that failed. Everything else is inactive: done
 * for now, but an observation or the main agent can start it again.
 */
export function isLiveConversation(c: Conversation): boolean {
  return c.status === 'working' || c.status === 'error' || isAwaitingAnswer(c)
}

/** Live sub-agents first, each half newest first. */
function compareChildren(a: ConvNode, b: ConvNode): number {
  const live =
    Number(isLiveConversation(b.conversation)) -
    Number(isLiveConversation(a.conversation))
  return live || b.conversation.createdAt - a.conversation.createdAt
}

/** Whether a node, or anything under it, is still live. */
function subtreeLive(node: ConvNode): boolean {
  return (
    isLiveConversation(node.conversation) || node.children.some(subtreeLive)
  )
}

/** Whether `id` is the node or sits somewhere under it. */
function subtreeHas(node: ConvNode, id: string): boolean {
  return (
    node.conversation.id === id || node.children.some((c) => subtreeHas(c, id))
  )
}

export function buildConversationTree(list: Conversation[]): ConvNode[] {
  const byId = new Map(list.map((c) => [c.id, c]))

  /* Candidate parent edges: only those whose parent is present and isn't the
     node itself. Cycles are filtered below against this raw map. */
  const rawParent = new Map<string, string>()
  for (const c of list) {
    const p = c.parentId
    if (p && p !== c.id && byId.has(p)) rawParent.set(c.id, p)
  }

  const createsCycle = (id: string): boolean => {
    const seen = new Set<string>([id])
    let cur = rawParent.get(id)
    while (cur) {
      if (seen.has(cur)) return true
      seen.add(cur)
      cur = rawParent.get(cur)
    }
    return false
  }

  const nodes = new Map<string, ConvNode>(
    list.map((c) => [c.id, { conversation: c, children: [], depth: 0 }]),
  )

  const roots: ConvNode[] = []
  for (const c of list) {
    // biome-ignore lint/style/noNonNullAssertion: nodes is seeded from list
    const node = nodes.get(c.id)!
    const parentId = rawParent.get(c.id)
    if (parentId && !createsCycle(c.id)) {
      // biome-ignore lint/style/noNonNullAssertion: parentId came from byId
      nodes.get(parentId)!.children.push(node)
    } else {
      roots.push(node)
    }
  }

  /* Roots newest-first (matches the flat list); children in spawn order. The
     `seen` guard is belt-and-suspenders — the tree is already acyclic here. */
  roots.sort((a, b) => b.conversation.updatedAt - a.conversation.updatedAt)
  const finalize = (node: ConvNode, depth: number, seen: Set<string>) => {
    if (seen.has(node.conversation.id)) return
    seen.add(node.conversation.id)
    node.depth = depth
    node.children.sort(compareChildren)
    for (const child of node.children) finalize(child, depth + 1, seen)
  }
  for (const root of roots) finalize(root, 0, new Set())

  return roots
}

/**
 * Depth-first flatten into render rows, skipping the subtree of any collapsed
 * node. `hasChildren` lets the row show a toggle caret.
 */
/**
 * Depth-first rows for the sidebar. Under each parent only its live
 * sub-agents show; the inactive ones fold behind one toggle row unless the
 * parent is in `showInactive`. An inactive sub-agent that is open (`keepId`)
 * or has a live one under it stays in view.
 */
export function flattenConversationTree(
  roots: ConvNode[],
  collapsed: ReadonlySet<string>,
  showInactive: ReadonlySet<string> = new Set(),
  keepId: string | null = null,
): TreeRow[] {
  const out: TreeRow[] = []
  const walk = (nodes: ConvNode[]) => {
    for (const node of nodes) {
      const hasChildren = node.children.length > 0
      out.push({
        conversation: node.conversation,
        depth: node.depth,
        hasChildren,
      })
      if (!hasChildren || collapsed.has(node.conversation.id)) continue
      const shown: ConvNode[] = []
      const folded: ConvNode[] = []
      for (const child of node.children) {
        const visible =
          subtreeLive(child) || (keepId !== null && subtreeHas(child, keepId))
        ;(visible ? shown : folded).push(child)
      }
      const expanded = showInactive.has(node.conversation.id)
      walk(expanded ? node.children : shown)
      if (folded.length > 0)
        out.push({
          kind: 'inactive-toggle',
          parentId: node.conversation.id,
          depth: node.depth + 1,
          inactive: folded.length,
          expanded,
        })
    }
  }
  walk(roots)
  return out
}
