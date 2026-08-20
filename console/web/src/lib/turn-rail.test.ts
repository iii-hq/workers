import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import {
  currentTickIndex,
  nearestTickIndex,
  steppedTickIndex,
  turnsFromMessages,
  viewportSegment,
} from './turn-rail'

let seq = 0
const user = (content: string, extra: Partial<Message> = {}): Message =>
  ({
    id: `u${++seq}`,
    createdAt: seq,
    role: 'user',
    content,
    ...extra,
  }) as Message
const assistant = (content: string, extra: Partial<Message> = {}): Message =>
  ({
    id: `a${++seq}`,
    createdAt: seq,
    role: 'assistant',
    content,
    ...extra,
  }) as Message
const call = (): Message =>
  ({
    id: `f${++seq}`,
    createdAt: seq,
    role: 'function-trigger',
    functionId: 'shell::exec',
    input: {},
  }) as Message

describe('turnsFromMessages', () => {
  it('ticks user prompts and agent prose, attributing calls to the tick before them', () => {
    const turns = turnsFromMessages([
      user('  first\nquestion  '),
      call(),
      call(),
      assistant(''),
      assistant('the answer\nwith detail'),
      call(),
      user('ping', { notification: true } as Partial<Message>),
      user('second'),
      assistant('ok', { streaming: true } as Partial<Message>),
    ])
    expect(turns.map((t) => [t.kind, t.calls, t.tone])).toEqual([
      ['user', 2, 'ink'],
      ['agent', 1, 'ink'],
      ['user', 0, 'ink'],
      ['agent', 0, 'accent'],
    ])
    expect(turns[0]).toMatchObject({
      prompt: 'first question',
      reply: 'the answer with detail',
    })
    expect(turns[1]).toMatchObject({
      prompt: '',
      reply: 'the answer with detail',
    })
    expect(turns[3].reply).toBe('ok')
  })

  it('marks failed replies and system errors alert', () => {
    const turns = turnsFromMessages([
      user('one'),
      assistant('boom', { stopReason: 'error' } as Partial<Message>),
      user('two'),
      {
        id: 's1',
        createdAt: 99,
        role: 'system',
        content: 'x',
        tone: 'error',
      } as Message,
      assistant('', { streaming: true } as Partial<Message>),
    ])
    expect(turns.map((t) => [t.kind, t.tone])).toEqual([
      ['user', 'ink'],
      ['agent', 'alert'],
      ['user', 'alert'],
    ])
  })

  it('truncates long prompts and replies', () => {
    const turns = turnsFromMessages([
      user('p'.repeat(400)),
      assistant('r'.repeat(400)),
    ])
    expect(turns[0].prompt.length).toBe(140)
    expect(turns[0].prompt.endsWith('…')).toBe(true)
    expect(turns[0].reply.length).toBe(240)
    expect(turns[1].reply.length).toBe(240)
  })

  it('ignores leading replies with no turn and empty input', () => {
    expect(turnsFromMessages([assistant('stray')])).toEqual([])
    expect(turnsFromMessages([])).toEqual([])
  })
})

describe('viewportSegment', () => {
  it('maps the visible window to fractions and clamps to the range', () => {
    expect(viewportSegment(0, 500, 2000)).toEqual({ top: 0, height: 0.25 })
    expect(viewportSegment(1500, 500, 2000)).toEqual({
      top: 0.75,
      height: 0.25,
    })
    expect(viewportSegment(1900, 500, 2000)).toEqual({
      top: 0.75,
      height: 0.25,
    })
    expect(viewportSegment(0, 500, 300)).toEqual({ top: 0, height: 1 })
    expect(viewportSegment(10, 500, 0)).toEqual({ top: 0, height: 1 })
  })
})

describe('tick lookups', () => {
  const ticks = [0, 0.3, 0.6, 0.9]

  it('finds the nearest tick to a fraction', () => {
    expect(nearestTickIndex(0.1, ticks)).toBe(0)
    expect(nearestTickIndex(0.46, ticks)).toBe(2)
    expect(nearestTickIndex(1, ticks)).toBe(3)
    expect(nearestTickIndex(0.5, [])).toBe(-1)
  })

  it('reports the turn containing the viewport top and steps within bounds', () => {
    const offsets = [0, 300, 600, 900]
    expect(currentTickIndex(0, offsets)).toBe(0)
    expect(currentTickIndex(450, offsets)).toBe(1)
    expect(currentTickIndex(601, offsets)).toBe(2)
    expect(currentTickIndex(-5, offsets)).toBe(-1)
    expect(steppedTickIndex(1, 1, 4)).toBe(2)
    expect(steppedTickIndex(0, -1, 4)).toBe(0)
    expect(steppedTickIndex(3, 1, 4)).toBe(3)
    expect(steppedTickIndex(-1, 1, 4)).toBe(0)
    expect(steppedTickIndex(-1, -1, 4)).toBe(0)
    expect(steppedTickIndex(0, 1, 0)).toBe(-1)
  })
})
