import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import {
  buildConversationTree,
  flattenConversationTree,
} from './conversation-tree'

let clock = 0
function conv(id: string, parentId?: string, at = clock++): Conversation {
  return {
    id,
    title: id,
    model: null,
    mode: 'agent',
    messages: [],
    parentId,
    createdAt: at,
    updatedAt: at,
  }
}

const ids = (rows: { conversation: Conversation }[]) =>
  rows.map((r) => r.conversation.id)

describe('buildConversationTree', () => {
  it('nests children under their parent', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('a1', 'a'),
      conv('a2', 'a'),
    ])
    expect(roots.map((r) => r.conversation.id)).toEqual(['a'])
    expect(roots[0].children.map((c) => c.conversation.id)).toEqual([
      'a1',
      'a2',
    ])
    expect(roots[0].depth).toBe(0)
    expect(roots[0].children[0].depth).toBe(1)
  })

  it('treats a child with a missing parent as a root (off-page/deleted)', () => {
    const roots = buildConversationTree([conv('orphan', 'gone')])
    expect(roots.map((r) => r.conversation.id)).toEqual(['orphan'])
    expect(roots[0].depth).toBe(0)
  })

  it('does not loop on a parent cycle and keeps every node reachable', () => {
    const a = conv('a', 'b')
    const b = conv('b', 'a')
    const roots = buildConversationTree([a, b])
    // Both promoted to roots rather than vanishing or looping.
    expect(new Set(roots.map((r) => r.conversation.id))).toEqual(
      new Set(['a', 'b']),
    )
  })

  it('ignores a self-parent', () => {
    const roots = buildConversationTree([conv('a', 'a')])
    expect(roots.map((r) => r.conversation.id)).toEqual(['a'])
  })

  it('sorts roots newest-first and children in spawn order', () => {
    const older = conv('old', undefined, 1)
    const newer = conv('new', undefined, 5)
    const c2 = conv('c2', 'new', 7)
    const c1 = conv('c1', 'new', 6)
    const roots = buildConversationTree([older, newer, c2, c1])
    expect(roots.map((r) => r.conversation.id)).toEqual(['new', 'old'])
    expect(roots[0].children.map((c) => c.conversation.id)).toEqual([
      'c1',
      'c2',
    ])
  })
})

describe('flattenConversationTree', () => {
  it('flattens depth-first with hasChildren flags', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('a1', 'a'),
      conv('a1a', 'a1'),
    ])
    const rows = flattenConversationTree(roots, new Set())
    expect(ids(rows)).toEqual(['a', 'a1', 'a1a'])
    expect(rows.map((r) => r.hasChildren)).toEqual([true, true, false])
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 2])
  })

  it('hides the subtree of a collapsed node', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('a1', 'a'),
      conv('a1a', 'a1'),
    ])
    const rows = flattenConversationTree(roots, new Set(['a1']))
    expect(ids(rows)).toEqual(['a', 'a1'])
  })
})
