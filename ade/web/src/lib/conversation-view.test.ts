import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import { buildConversationTree } from './conversation-tree'
import {
  compareConversations,
  DEFAULT_CONVERSATION_LIST_VIEW,
  describeConversationListView,
  effectiveConversationKind,
  filterConversationsByKind,
  isDefaultConversationListView,
  kindsLabel,
  normalizeKinds,
  parseConversationListView,
  sortConversationRoots,
  toggleKind,
} from './conversation-view'

const NOW = 1_700_000_000_000

function conv(id: string, partial: Partial<Conversation> = {}): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: NOW,
    updatedAt: NOW,
    ...partial,
  }
}

const ids = (list: { id: string }[]) => list.map((c) => c.id)

describe('parseConversationListView', () => {
  it('falls back field by field, never blanking the list', () => {
    expect(parseConversationListView(undefined)).toEqual(
      DEFAULT_CONVERSATION_LIST_VIEW,
    )
    expect(parseConversationListView('garbage')).toEqual(
      DEFAULT_CONVERSATION_LIST_VIEW,
    )
    expect(
      parseConversationListView({ kinds: ['e2e', 'nope'], sort: 'sideways' }),
    ).toEqual({ kinds: ['e2e'], grouping: 'recent', sort: 'activity' })
  })

  it('honours the older grouping-only key only when the record has none', () => {
    expect(parseConversationListView(undefined, 'project').grouping).toBe(
      'project',
    )
    expect(
      parseConversationListView({ grouping: 'none' }, 'project').grouping,
    ).toBe('none')
    expect(parseConversationListView(undefined, 'bogus').grouping).toBe(
      'recent',
    )
  })

  it('canonicalises kind order and drops duplicates', () => {
    expect(normalizeKinds(['e2e', 'user', 'e2e', 7])).toEqual(['user', 'e2e'])
  })
})

describe('defaults and toggling', () => {
  it('knows when nothing differs from the defaults', () => {
    expect(isDefaultConversationListView(DEFAULT_CONVERSATION_LIST_VIEW)).toBe(
      true,
    )
    expect(
      isDefaultConversationListView({
        ...DEFAULT_CONVERSATION_LIST_VIEW,
        kinds: ['user', 'e2e'],
      }),
    ).toBe(false)
    expect(
      isDefaultConversationListView({
        ...DEFAULT_CONVERSATION_LIST_VIEW,
        grouping: 'none',
      }),
    ).toBe(false)
  })

  it('toggles a kind in and out, allowing an empty selection', () => {
    const withE2e = toggleKind(DEFAULT_CONVERSATION_LIST_VIEW, 'e2e', true)
    expect(withE2e.kinds).toEqual(['user', 'e2e'])
    const none = toggleKind(toggleKind(withE2e, 'user', false), 'e2e', false)
    expect(none.kinds).toEqual([])
  })

  it('labels the Type row from the selection', () => {
    expect(kindsLabel(['user'])).toBe('User')
    expect(kindsLabel(['user', 'e2e'])).toBe('User, E2E')
    expect(kindsLabel(['user', 'automation', 'e2e'])).toBe('All')
    expect(kindsLabel([])).toBe('None')
  })

  it('describes every setting for the trigger tooltip', () => {
    expect(describeConversationListView(DEFAULT_CONVERSATION_LIST_VIEW)).toBe(
      'User · Recent · Last activity',
    )
    expect(
      describeConversationListView({
        kinds: ['e2e'],
        grouping: 'project',
        sort: 'title',
      }),
    ).toBe('E2E · Project · Title')
  })
})

describe('effectiveConversationKind', () => {
  it('lists a sub-agent under its root, however deep', () => {
    const root = conv('root', { kind: 'e2e' })
    const child = conv('child', { parentId: 'root' })
    const grandchild = conv('grandchild', { parentId: 'child', kind: 'user' })
    const byId = new Map([root, child, grandchild].map((c) => [c.id, c]))
    expect(effectiveConversationKind(grandchild, byId)).toBe('e2e')
  })

  it('treats a missing kind and a missing parent as a user root', () => {
    const orphan = conv('orphan', { parentId: 'gone' })
    expect(
      effectiveConversationKind(orphan, new Map([[orphan.id, orphan]])),
    ).toBe('user')
  })

  it('stops on a parent cycle instead of looping', () => {
    const a = conv('a', { parentId: 'b', kind: 'automation' })
    const b = conv('b', { parentId: 'a' })
    const byId = new Map([a, b].map((c) => [c.id, c]))
    expect(effectiveConversationKind(a, byId)).toBe('user')
  })
})

describe('filterConversationsByKind', () => {
  const list = [
    conv('chat'),
    conv('suite', { kind: 'e2e' }),
    conv('suite-worker', { parentId: 'suite' }),
    conv('nightly', { kind: 'automation' }),
  ]

  it('shows only user roots by default, hiding an e2e subtree whole', () => {
    expect(ids(filterConversationsByKind(list, ['user']))).toEqual(['chat'])
  })

  it('keeps a sub-agent with its root when the root is shown', () => {
    expect(ids(filterConversationsByKind(list, ['e2e']))).toEqual([
      'suite',
      'suite-worker',
    ])
  })

  it('returns the input untouched when every kind is shown', () => {
    expect(filterConversationsByKind(list, ['user', 'automation', 'e2e'])).toBe(
      list,
    )
  })

  it('hides everything for an empty selection', () => {
    expect(filterConversationsByKind(list, [])).toEqual([])
  })
})

describe('sorting', () => {
  const older = conv('older', {
    title: 'Zeta',
    createdAt: NOW - 3,
    updatedAt: NOW - 1,
  })
  const newer = conv('newer', {
    title: 'alpha',
    createdAt: NOW - 1,
    updatedAt: NOW - 3,
  })
  const mid = conv('mid', {
    title: 'Beta',
    createdAt: NOW - 2,
    updatedAt: NOW - 2,
  })

  it('orders by last activity, creation, or title', () => {
    const byActivity = [older, newer, mid].sort((a, b) =>
      compareConversations(a, b, 'activity'),
    )
    expect(ids(byActivity)).toEqual(['older', 'mid', 'newer'])
    const byCreated = [older, newer, mid].sort((a, b) =>
      compareConversations(a, b, 'created'),
    )
    expect(ids(byCreated)).toEqual(['newer', 'mid', 'older'])
    const byTitle = [older, newer, mid].sort((a, b) =>
      compareConversations(a, b, 'title'),
    )
    expect(ids(byTitle)).toEqual(['newer', 'mid', 'older'])
  })

  it('reorders roots only; children keep their spawn order', () => {
    const roots = buildConversationTree([
      conv('b-root', { title: 'B', updatedAt: NOW - 1 }),
      conv('a-root', { title: 'A', updatedAt: NOW - 2 }),
      conv('z-child', { title: 'Z', parentId: 'a-root', createdAt: NOW - 5 }),
      conv('c-child', { title: 'C', parentId: 'a-root', createdAt: NOW - 4 }),
    ])
    const sorted = sortConversationRoots(roots, 'title')
    expect(sorted.map((n) => n.conversation.id)).toEqual(['a-root', 'b-root'])
    expect(sorted[0]?.children.map((n) => n.conversation.id)).toEqual([
      'z-child',
      'c-child',
    ])
    /* The input array is left alone: sorting is a view, not a mutation. */
    expect(roots.map((n) => n.conversation.id)).toEqual(['b-root', 'a-root'])
  })
})
