import { describe, expect, it } from 'vitest'
import { writerDigest } from '../src/digest.js'
import { controlPlaneFixture } from '../src/fixtures.js'
import type { ControlPlaneData } from '../src/types.js'

const base = controlPlaneFixture as unknown as ControlPlaneData

function bigBoard(cards: number): ControlPlaneData {
  const template = base.tickets[0]
  const tickets = Array.from({ length: cards }, (_, index) => ({
    ...template,
    id: `t-${index}`,
    key: `KAN-${index}`,
    title: `Card ${index} ${'with a long descriptive title '.repeat(6)}`,
    status: index % 3 === 0 ? 'in_progress' : index % 3 === 1 ? 'done' : 'todo',
    updated_at: new Date(Date.UTC(2026, 10, 1) + index * 60_000).toISOString(),
  }))
  return { ...base, tickets }
}

describe('writer digest', () => {
  it('stays bounded on a board with hundreds of cards', () => {
    const data = bigBoard(700)
    const activity = data.tickets.map((ticket, index) => ({
      ticket_id: ticket.id,
      kind: 'ticket.moved',
      actor: 'agent',
      at: new Date(Date.UTC(2026, 10, 2) + index * 1000).toISOString(),
      changes: [{ field: 'status', from: 'todo', to: 'in_progress' }],
    }))
    const digest = writerDigest(data, { scopeName: 'All projects', since: null, activity })
    expect(digest.active_cards).toHaveLength(40)
    expect(digest.changed_cards).toHaveLength(40)
    expect(digest.activity).toHaveLength(60)
    expect(digest.board?.total).toBe(700)
    expect(digest.active_cards[0].key).toBe('KAN-699')
    expect(digest.active_cards[0].title.length).toBeLessThanOrEqual(120)
    expect(JSON.stringify(digest).length).toBeLessThan(40_000)
  })

  it('keeps only cards changed since the checkpoint, and only activity on scoped cards', () => {
    const data = bigBoard(10)
    const since = data.tickets[7].updated_at
    const digest = writerDigest(data, {
      scopeName: 'Assistente',
      since,
      activity: [
        { ticket_id: 't-9', kind: 'ticket.moved', actor: 'agent', at: since },
        { ticket_id: 'outside-scope', kind: 'ticket.moved', actor: 'agent', at: since },
      ],
    })
    expect(digest.changed_cards.map((card) => card.key)).toEqual(['KAN-9', 'KAN-8'])
    expect(digest.activity.map((row) => row.card)).toEqual(['KAN-9'])
    expect(digest.scope).toBe('Assistente')
  })

  it('carries a marked card description and newest comments as its evidence', () => {
    const card = base.tickets[0]
    const item = {
      id: `card:${card.key}`,
      kind: 'card' as const,
      sessionId: '',
      title: card.title,
      marks: ['decisão'],
      card,
      blockedCards: 0,
      createdAt: 0,
      urgency: 'no-rush' as const,
    }
    const digest = writerDigest(
      { ...base, items: [item] },
      {
        scopeName: 'All',
        since: null,
        activity: [],
        evidence: {
          [item.id]: {
            description: `Pick the table.\n\n${'detail '.repeat(200)}`,
            comments: [
              { author: 'a', body: 'old' },
              { author: 'b', body: 'Owner: go with usage.' },
              { author: 'c', body: 'Agreed.' },
            ],
          },
        },
      },
    )
    const [candidate] = digest.needs_you
    expect(candidate).toMatchObject({ id: item.id, marks: ['decisão'] })
    expect(candidate.description?.startsWith('Pick the table. detail')).toBe(true)
    expect(candidate.description?.length).toBeLessThanOrEqual(700)
    expect(candidate.recent_comments?.map((comment) => comment.text)).toEqual([
      'Owner: go with usage.',
      'Agreed.',
    ])
  })

  it('marks pull requests unread instead of empty when no repository was read', () => {
    const unread = writerDigest(
      { ...base, repositories: [], prs: [] },
      { scopeName: 'All', since: null, activity: [] },
    )
    expect(unread.prs).toBeNull()
    const read = writerDigest(
      { ...base, repositories: ['iii-hq/workers'], prs: [] },
      { scopeName: 'All', since: null, activity: [] },
    )
    expect(read.prs).toEqual([])
  })
})
