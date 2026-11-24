import { describe, expect, it } from 'vitest'
import type { TimelineMessageListRow } from '@/components/observer-activity/grouping'
import type { AssistantMessage, ThoughtMessage } from '@/types/chat'
import { mergeThoughtRuns } from './thought-runs'

const thought = (
  id: string,
  content: string,
  extra: Partial<ThoughtMessage> = {},
): TimelineMessageListRow => ({
  kind: 'message',
  message: {
    id,
    role: 'thought',
    content,
    durationMs: 1000,
    createdAt: 0,
    ...extra,
  } as ThoughtMessage,
})

const prose = (id: string): TimelineMessageListRow => ({
  kind: 'message',
  message: {
    id,
    role: 'assistant',
    content: 'answer',
    createdAt: 0,
  } as AssistantMessage,
})

describe('mergeThoughtRuns', () => {
  it('folds adjacent thoughts into one row keyed by the first', () => {
    const rows = mergeThoughtRuns([
      thought('t1', 'one'),
      thought('t2', 'two'),
      thought('t3', 'three'),
      prose('a1'),
    ])
    expect(rows).toHaveLength(2)
    const merged = rows[0].kind === 'message' ? rows[0].message : null
    expect(merged).toMatchObject({
      id: 't1',
      role: 'thought',
      content: 'one\n\ntwo\n\nthree',
      durationMs: 3000,
    })
    expect(merged && 'streaming' in merged).toBe(false)
  })

  it('is live while its newest thought streams', () => {
    const rows = mergeThoughtRuns([
      thought('t1', 'one'),
      thought('t2', 'two', { streaming: true, durationMs: 0 }),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0].kind === 'message' && rows[0].message).toMatchObject({
      id: 't1',
      streaming: true,
      durationMs: 1000,
    })
  })

  it('leaves single thoughts and separated thoughts as they are', () => {
    const input = [thought('t1', 'one'), prose('a1'), thought('t2', 'two')]
    const rows = mergeThoughtRuns(input)
    expect(rows).toHaveLength(3)
    expect(rows[0]).toEqual(input[0])
    expect(rows[2]).toEqual(input[2])
  })
})
