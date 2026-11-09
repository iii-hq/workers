import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import {
  buildConversationTree,
  type FlatRow,
  flattenConversationTree,
  isInactiveToggle,
  type TreeRow,
} from './conversation-tree'

let clock = 0
function conv(id: string, parentId?: string, at = clock++): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    parentId,
    createdAt: at,
    updatedAt: at,
  }
}

const ids = (rows: TreeRow[]) =>
  rows.map((r) => (isInactiveToggle(r) ? `+${r.inactive}` : r.conversation.id))
const chats = (rows: TreeRow[]) =>
  rows.filter((r): r is FlatRow => !isInactiveToggle(r))
const working = (c: Conversation): Conversation => ({ ...c, status: 'working' })

describe('buildConversationTree', () => {
  it('nests children under their parent', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('a1', 'a'),
      conv('a2', 'a'),
    ])
    expect(roots.map((r) => r.conversation.id)).toEqual(['a'])
    // Newest sub-agent first.
    expect(roots[0].children.map((c) => c.conversation.id)).toEqual([
      'a2',
      'a1',
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

  it('sorts roots newest-first and children live-first, newest first', () => {
    const older = conv('old', undefined, 1)
    const newer = conv('new', undefined, 5)
    const c1 = conv('c1', 'new', 6)
    const c2 = conv('c2', 'new', 7)
    const c3 = working(conv('c3', 'new', 2))
    const roots = buildConversationTree([older, newer, c1, c2, c3])
    expect(roots.map((r) => r.conversation.id)).toEqual(['new', 'old'])
    expect(roots[0].children.map((c) => c.conversation.id)).toEqual([
      'c3',
      'c2',
      'c1',
    ])
  })
})

describe('flattenConversationTree', () => {
  it('flattens depth-first with hasChildren flags', () => {
    const roots = buildConversationTree([
      conv('a'),
      working(conv('a1', 'a')),
      working(conv('a1a', 'a1')),
    ])
    const rows = chats(flattenConversationTree(roots, new Set()))
    expect(ids(rows)).toEqual(['a', 'a1', 'a1a'])
    expect(rows.map((r) => r.hasChildren)).toEqual([true, true, false])
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 2])
  })

  it('hides the subtree of a collapsed node', () => {
    const roots = buildConversationTree([
      conv('a'),
      working(conv('a1', 'a')),
      working(conv('a1a', 'a1')),
    ])
    const rows = flattenConversationTree(roots, new Set(['a1']))
    expect(ids(rows)).toEqual(['a', 'a1'])
  })

  it('folds inactive sub-agents behind one toggle under the live ones', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('done1', 'a'),
      working(conv('live', 'a')),
      conv('done2', 'a'),
    ])
    const rows = flattenConversationTree(roots, new Set())
    expect(ids(rows)).toEqual(['a', 'live', '+2'])
    const toggle = rows.at(-1)
    expect(toggle && isInactiveToggle(toggle) && toggle.depth).toBe(1)

    const shown = flattenConversationTree(roots, new Set(), new Set(['a']))
    expect(ids(shown)).toEqual(['a', 'live', 'done2', 'done1', '+2'])
  })

  it('counts a waiting question or an error as live', () => {
    const roots = buildConversationTree([
      conv('a'),
      {
        ...conv('asked', 'a'),
        status: 'done',
        statusReason: 'awaiting answer',
      },
      { ...conv('failed', 'a'), status: 'error' },
      { ...conv('done', 'a'), status: 'done' },
    ])
    expect(ids(flattenConversationTree(roots, new Set()))).toEqual([
      'a',
      'failed',
      'asked',
      '+1',
    ])
  })

  it('keeps the open chat and an inactive parent of a live one in view', () => {
    const roots = buildConversationTree([
      conv('a'),
      conv('open', 'a'),
      conv('mid', 'a'),
      working(conv('deep', 'mid')),
      conv('gone', 'a'),
    ])
    expect(
      ids(flattenConversationTree(roots, new Set(), new Set(), 'open')),
    ).toEqual(['a', 'mid', 'deep', 'open', '+1'])
  })
})
