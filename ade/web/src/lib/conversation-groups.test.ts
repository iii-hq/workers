import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import {
  calendarDaysAgo,
  countConversations,
  groupConversationRoots,
  normalizeProjectPath,
} from './conversation-groups'
import { buildConversationTree } from './conversation-tree'

/* A fixed local noon keeps every offset inside its own calendar day, so the
   assertions do not depend on the machine's timezone. */
const NOW = new Date(2024, 4, 15, 12, 0, 0).getTime()
const DAY = 86_400_000

function conv(
  id: string,
  partial: Partial<Conversation> & { updatedAt?: number } = {},
): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: partial.updatedAt ?? NOW,
    updatedAt: partial.updatedAt ?? NOW,
    ...partial,
  }
}

const labels = (groups: { label: string }[]) => groups.map((g) => g.label)
const ids = (nodes: { conversation: Conversation }[]) =>
  nodes.map((n) => n.conversation.id)

describe('calendarDaysAgo', () => {
  it('counts whole calendar days, not elapsed 24h windows', () => {
    const lateYesterday = new Date(2024, 4, 14, 23, 30).getTime()
    const earlyToday = new Date(2024, 4, 15, 0, 15).getTime()
    expect(calendarDaysAgo(lateYesterday, NOW)).toBe(1)
    expect(calendarDaysAgo(earlyToday, NOW)).toBe(0)
  })
})

describe('normalizeProjectPath', () => {
  it('folds trailing separators, blanks and whitespace', () => {
    expect(normalizeProjectPath('/a/b/')).toBe('/a/b')
    expect(normalizeProjectPath('/a/b')).toBe('/a/b')
    expect(normalizeProjectPath('/')).toBe('/')
    expect(normalizeProjectPath('   ')).toBeNull()
    expect(normalizeProjectPath(null)).toBeNull()
    expect(normalizeProjectPath(undefined)).toBeNull()
  })
})

describe('groupConversationRoots — recent', () => {
  it('buckets by calendar day and drops empty sections', () => {
    const roots = buildConversationTree([
      conv('now', { updatedAt: NOW - 60_000 }),
      conv('yesterday', { updatedAt: NOW - DAY }),
      conv('this-week', { updatedAt: NOW - 4 * DAY }),
      conv('last-month', { updatedAt: NOW - 20 * DAY }),
      conv('ancient', { updatedAt: NOW - 400 * DAY }),
    ])
    const groups = groupConversationRoots(roots, 'recent', NOW)
    expect(labels(groups)).toEqual([
      'Today',
      'Yesterday',
      'Previous 7 days',
      'Previous 30 days',
      'Older',
    ])
    expect(groups.map((g) => ids(g.roots))).toEqual([
      ['now'],
      ['yesterday'],
      ['this-week'],
      ['last-month'],
      ['ancient'],
    ])
  })

  it('omits sections nothing falls into', () => {
    const roots = buildConversationTree([
      conv('a', { updatedAt: NOW }),
      conv('b', { updatedAt: NOW - 200 * DAY }),
    ])
    expect(labels(groupConversationRoots(roots, 'recent', NOW))).toEqual([
      'Today',
      'Older',
    ])
  })

  it('folds a timestamp ahead of the clock into Today', () => {
    const roots = buildConversationTree([
      conv('skewed', { updatedAt: NOW + 3 * DAY }),
    ])
    expect(labels(groupConversationRoots(roots, 'recent', NOW))).toEqual([
      'Today',
    ])
  })

  it('keeps a sub-agent inside its root section rather than its own', () => {
    const roots = buildConversationTree([
      conv('parent', { updatedAt: NOW }),
      conv('child', { parentId: 'parent', updatedAt: NOW - 200 * DAY }),
    ])
    const groups = groupConversationRoots(roots, 'recent', NOW)
    expect(labels(groups)).toEqual(['Today'])
    expect(countConversations(groups[0].roots)).toBe(2)
  })
})

describe('groupConversationRoots — project', () => {
  it('buckets by working directory, newest project first', () => {
    const roots = buildConversationTree([
      conv('web-a', { workingDir: '/home/me/web', updatedAt: NOW }),
      conv('api-a', { workingDir: '/home/me/api', updatedAt: NOW - DAY }),
      conv('web-b', { workingDir: '/home/me/web/', updatedAt: NOW - 2 * DAY }),
    ])
    const groups = groupConversationRoots(roots, 'project', NOW)
    expect(labels(groups)).toEqual(['web', 'api'])
    expect(groups[0].title).toBe('/home/me/web')
    expect(ids(groups[0].roots)).toEqual(['web-a', 'web-b'])
  })

  it('puts sessions without a scope in a trailing catch-all', () => {
    const roots = buildConversationTree([
      conv('loose', { updatedAt: NOW }),
      conv('scoped', { workingDir: '/home/me/api', updatedAt: NOW - DAY }),
      conv('blank', { workingDir: '  ', updatedAt: NOW - 2 * DAY }),
    ])
    const groups = groupConversationRoots(roots, 'project', NOW)
    expect(labels(groups)).toEqual(['api', 'No project'])
    expect(ids(groups[1].roots)).toEqual(['loose', 'blank'])
    expect(groups[1].title).toBeUndefined()
  })

  it('qualifies two projects that share a folder name', () => {
    const roots = buildConversationTree([
      conv('one', { workingDir: '/home/me/acme/web', updatedAt: NOW }),
      conv('two', { workingDir: '/home/me/beta/web', updatedAt: NOW - DAY }),
      conv('three', { workingDir: '/home/me/api', updatedAt: NOW - 2 * DAY }),
    ])
    expect(labels(groupConversationRoots(roots, 'project', NOW))).toEqual([
      'acme/web',
      'beta/web',
      'api',
    ])
  })

  it('groups a sub-agent with its root, not with its own scope', () => {
    const roots = buildConversationTree([
      conv('parent', { workingDir: '/home/me/web', updatedAt: NOW }),
      conv('child', {
        parentId: 'parent',
        workingDir: '/home/me/api',
        updatedAt: NOW - DAY,
      }),
    ])
    const groups = groupConversationRoots(roots, 'project', NOW)
    expect(labels(groups)).toEqual(['web'])
    expect(countConversations(groups[0].roots)).toBe(2)
  })
})

describe('countConversations', () => {
  it('counts nested sub-agents', () => {
    const roots = buildConversationTree([
      conv('root'),
      conv('kid', { parentId: 'root' }),
      conv('grandkid', { parentId: 'kid' }),
      conv('other'),
    ])
    expect(countConversations(roots)).toBe(4)
  })

  it('is zero for no sections', () => {
    expect(countConversations([])).toBe(0)
  })
})
