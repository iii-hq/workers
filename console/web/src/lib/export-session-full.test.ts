import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionMeta } from '@/lib/sessions/types'
import type { UserMessage } from '@/types/chat'

const mockTrigger = vi.hoisted(() => vi.fn())
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: mockTrigger }),
}))

import type { Conversation } from '@/types/chat'

import {
  assembleFullExport,
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

function conv(id: string): Conversation {
  return {
    id,
    title: 'Root session',
    model: 'openai::gpt-5',
    mode: 'agent',
    messages: [{ id: 'u1', role: 'user', content: 'root msg', createdAt: 1 }],
    createdAt: Date.UTC(2025, 0, 1),
    updatedAt: Date.UTC(2025, 0, 1),
  }
}

/** Route mockTrigger by function id; per-session transcripts by session_id. */
function routeTriggers(opts: {
  sessions?: unknown
  transcripts?: Record<string, unknown>
}) {
  mockTrigger.mockImplementation(
    (fnId: string, payload: Record<string, unknown> = {}) => {
      if (fnId === 'engine::workers::list') {
        return Promise.resolve({
          workers: [{ id: 'w1', name: 'harness', version: '1.5.2' }],
        })
      }
      if (fnId === 'session::list') {
        if (opts.sessions instanceof Error) return Promise.reject(opts.sessions)
        return Promise.resolve({ sessions: opts.sessions, next_cursor: null })
      }
      if (fnId === 'session::messages') {
        const entry = opts.transcripts?.[payload.session_id as string]
        if (entry instanceof Error) return Promise.reject(entry)
        return Promise.resolve({ messages: entry ?? [], next_cursor: null })
      }
      return Promise.reject(new Error(`unrouted trigger ${fnId}`))
    },
  )
}

describe('assembleFullExport', () => {
  beforeEach(() => {
    mockTrigger.mockReset()
  })

  const childTranscript = [
    {
      entry_id: 'e1',
      message: {
        role: 'user',
        content: [{ type: 'text', text: 'child msg' }],
        timestamp: 1,
      },
    },
  ]

  it('bundles descendants with a count bullet and -full filename', async () => {
    routeTriggers({
      sessions: [meta('root', null), meta('child', 'root', 10)],
      transcripts: { child: childTranscript },
    })
    const { markdown, filename } = await assembleFullExport(conv('root'))
    expect(filename).toMatch(/^iii-session-root-full-\d{8}-\d{4}\.md$/)
    expect(markdown).toContain('- Sub-agents: 1')
    expect(markdown).toContain('# Sub-agent: title-child')
    expect(markdown).toContain('child msg')
    expect(markdown).toContain('- Workers:')
  })

  it('degrades to _(unavailable)_ when session discovery fails', async () => {
    routeTriggers({ sessions: new Error('offline') })
    const { markdown } = await assembleFullExport(conv('root'))
    expect(markdown).toContain('- Sub-agents: _(unavailable)_')
    expect(markdown).not.toContain('# Sub-agent:')
    expect(markdown).toContain('root msg')
  })

  it('keeps other sections when one transcript fails', async () => {
    routeTriggers({
      sessions: [
        meta('root', null),
        meta('bad', 'root', 10),
        meta('good', 'root', 20),
      ],
      transcripts: { bad: new Error('boom'), good: childTranscript },
    })
    const { markdown } = await assembleFullExport(conv('root'))
    expect(markdown).toContain('- Sub-agents: 2')
    expect(markdown).toContain('# Sub-agent: title-bad')
    expect(markdown).toContain('_(transcript unavailable)_')
    expect(markdown).toContain('# Sub-agent: title-good')
    expect(markdown).toContain('child msg')
  })
})
