import { describe, expect, it } from 'vitest'
import {
  badgeCount,
  contextItemCount,
  parseContextItems,
} from './context-items'

const wire = {
  session_id: 's1',
  message_count: 12,
  generated_at_ms: 5,
  subagents: [
    {
      session_id: 'c1',
      title: 'Find senders',
      status: 'done',
      display: 'Explore',
      started_at_ms: 1,
      descendants: 0,
    },
    { title: 'no id' },
  ],
  pull_requests: [
    {
      repo: 'acme/web',
      number: 482,
      url: 'https://github.com/acme/web/pull/482',
      state: 'open',
      checks: 'passing',
      entry_id: 'e1',
      source: 'created',
    },
    {
      repo: 'acme/web',
      number: 479,
      url: 'https://github.com/acme/web/pull/479',
      state: 'weird',
      entry_id: 'e2',
      source: 'mentioned',
    },
  ],
  links: [{ url: 'https://docs.example.com/x', entry_id: 'e3', count: 2 }],
  attachments: [
    {
      name: 'log.txt',
      mime: 'text/plain',
      size: 48,
      direction: 'sent',
      entry_id: 'e4',
    },
  ],
  images: [
    { mime: 'image/png', direction: 'sent', entry_id: 'e5', block_index: 0 },
    {
      attachment_id: 'a1',
      mime: 'image/png',
      direction: 'received',
      entry_id: 'e6',
      block_index: 1,
    },
  ],
  side_chats: [],
  truncated: { links: true },
}

describe('harness::context-items parsing (§1.2.4)', () => {
  it('keeps the known fields and drops rows without ids', () => {
    const items = parseContextItems(wire)
    expect(items.subagents).toHaveLength(1)
    expect(items.subagents[0]).toMatchObject({
      display: 'Explore',
      status: 'done',
    })
    expect(items.pull_requests[0]).toMatchObject({
      state: 'open',
      checks: 'passing',
      source: 'created',
    })
    // An unknown state is left out, not guessed.
    expect(items.pull_requests[1].state).toBeUndefined()
    // A link without a host gets the URL's.
    expect(items.links[0].host).toBe('docs.example.com')
    expect(items.images.map((i) => i.direction)).toEqual(['sent', 'received'])
    expect(items.truncated).toEqual({ links: true })
  })

  it('counts every listed item for the badge, less dismissed pull requests', () => {
    const items = parseContextItems(wire)
    expect(contextItemCount(items)).toBe(1 + 2 + 1 + 1 + 2)
    expect(contextItemCount(items, new Set(['acme/web#479']))).toBe(6)
  })

  it('caps the badge at 99+', () => {
    expect(badgeCount(16)).toBe('16')
    expect(badgeCount(100)).toBe('99+')
  })

  it('survives an empty or malformed answer', () => {
    expect(parseContextItems(null)).toMatchObject({ links: [], images: [] })
  })
})
