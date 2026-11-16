import { describe, expect, it, vi } from 'vitest'
import {
  draftWithPulledBack,
  pullBackQueued,
  pulledBackNotice,
} from './stop-queued'

const rows = [
  { id: 'e_q1', text: 'queued A: say apple' },
  { id: 'e_q2', text: 'queued B: say pear' },
]

describe('pullBackQueued', () => {
  it('takes every still-parked message back out of the queue', async () => {
    const unqueue = vi.fn(async () => true)
    const { restored, kept } = await pullBackQueued(rows, unqueue)
    expect(unqueue.mock.calls).toEqual([['e_q1'], ['e_q2']])
    expect(restored).toEqual(rows)
    expect(kept).toEqual([])
  })

  it('keeps a message the agent already took, and one that could not be taken back', async () => {
    const unqueue = vi.fn(async (id: string) => {
      if (id === 'e_q2') throw new Error('timeout')
      return false
    })
    const { restored, kept } = await pullBackQueued(rows, unqueue)
    expect(restored).toEqual([])
    expect(kept).toEqual(rows)
  })

  it('an older backend that answers nothing counts as taken back', async () => {
    const { restored } = await pullBackQueued(rows, async () => undefined)
    expect(restored).toEqual(rows)
  })
})

describe('draftWithPulledBack', () => {
  it('returns the queued messages in order, then what was being typed', () => {
    expect(draftWithPulledBack('  and this  ', rows)).toBe(
      'queued A: say apple\n\nqueued B: say pear\n\nand this',
    )
  })

  it('a message with only attachments adds no text', () => {
    expect(draftWithPulledBack(undefined, [{ id: 'e', text: '' }])).toBe('')
  })
})

describe('pulledBackNotice', () => {
  it('says the message was not sent and where it went', () => {
    expect(pulledBackNotice(1)).toBe(
      'Your queued message was not sent. It is back in the composer.',
    )
    expect(pulledBackNotice(2)).toBe(
      'Your 2 queued messages were not sent. They are back in the composer.',
    )
  })
})
