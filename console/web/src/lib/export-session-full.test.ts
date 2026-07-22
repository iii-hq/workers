import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionMeta } from '@/lib/sessions/types'
import type { UserMessage } from '@/types/chat'

const mockTrigger = vi.hoisted(() => vi.fn())
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: mockTrigger }),
}))

import {
  collectDescendants,
  listAllSessions,
  subagentSectionToMarkdown,
} from './export-session-full'

function meta(id: string, parent: string | null, createdAt = 0): SessionMeta {
  return {
    session_id: id,
    title: `title-${id}`,
    description: '',
    status: 'done',
    created_at: createdAt,
    updated_at: createdAt,
    metadata: parent ? { parent_session_id: parent } : {},
  }
}

describe('collectDescendants', () => {
  it('walks depth-first with siblings ordered by created_at', () => {
    const sessions = [
      meta('root', null),
      meta('b', 'root', 20),
      meta('a', 'root', 10),
      meta('a1', 'a', 30),
      meta('other', null, 5),
    ]
    const out = collectDescendants(sessions, 'root')
    expect(out.map((d) => [d.meta.session_id, d.depth])).toEqual([
      ['a', 1],
      ['a1', 2],
      ['b', 1],
    ])
  })

  it('ignores cycles and never revisits a session', () => {
    const x = meta('x', 'root')
    const y = meta('y', 'x')
    // y claims x as its child too — cycle
    const xAsChildOfY = { ...x, metadata: { parent_session_id: 'y' } }
    const out = collectDescendants(
      [meta('root', null), x, y, xAsChildOfY],
      'root',
    )
    const ids = out.map((d) => d.meta.session_id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('returns [] when the root has no descendants', () => {
    expect(collectDescendants([meta('root', null)], 'root')).toEqual([])
  })
})

describe('listAllSessions', () => {
  beforeEach(() => {
    mockTrigger.mockReset()
  })

  it('follows next_cursor until exhausted', async () => {
    mockTrigger
      .mockResolvedValueOnce({
        sessions: [meta('s1', null)],
        next_cursor: 'c1',
      })
      .mockResolvedValueOnce({
        sessions: [meta('s2', null)],
        next_cursor: null,
      })
    const out = await listAllSessions()
    expect(out?.map((s) => s.session_id)).toEqual(['s1', 's2'])
    expect(mockTrigger).toHaveBeenCalledTimes(2)
    expect(mockTrigger).toHaveBeenLastCalledWith('session::list', {
      limit: 200,
      order: 'updated_desc',
      cursor: 'c1',
    })
  })

  it('returns null when the trigger rejects', async () => {
    mockTrigger.mockRejectedValue(new Error('offline'))
    await expect(listAllSessions()).resolves.toBeNull()
  })

  it('returns null on a malformed page', async () => {
    mockTrigger.mockResolvedValue({ nope: true })
    await expect(listAllSessions()).resolves.toBeNull()
  })
})

describe('subagentSectionToMarkdown', () => {
  const sub = { meta: meta('child-1', 'root', Date.UTC(2025, 0, 1)), depth: 2 }

  it('renders header bullets and the transcript body', () => {
    const user: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'do the thing',
      createdAt: 1,
    }
    const out = subagentSectionToMarkdown(sub, [user])
    expect(out).toContain('# Sub-agent: title-child-1')
    expect(out).toContain('- ID: `child-1`')
    expect(out).toContain('- Parent: `root`')
    expect(out).toContain('- Depth: 2')
    expect(out).toContain('- Message count: 1')
    expect(out).toContain('## User\ndo the thing')
  })

  it('renders _(transcript unavailable)_ without a message count on null', () => {
    const out = subagentSectionToMarkdown(sub, null)
    expect(out).toContain('_(transcript unavailable)_')
    expect(out).not.toContain('- Message count')
  })

  it('renders _(no messages)_ for an empty transcript', () => {
    const out = subagentSectionToMarkdown(sub, [])
    expect(out).toContain('- Message count: 0')
    expect(out).toContain('_(no messages)_')
  })
})
