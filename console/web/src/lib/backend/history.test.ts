import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import { translateUiHistoryForBackend } from './history'

const user = (content: string, createdAt = 1): Message => ({
  id: `u-${createdAt}`,
  role: 'user',
  content,
  createdAt,
})

const assistant = (
  content: string,
  model?: string,
  createdAt = 2,
): Message => ({
  id: `a-${createdAt}`,
  role: 'assistant',
  content,
  model: model as Message extends { role: 'assistant' }
    ? Message['model']
    : never,
  createdAt,
})

describe('translateUiHistoryForBackend', () => {
  it('round-trips text user + assistant messages in order', () => {
    const out = translateUiHistoryForBackend([
      user('hello', 1),
      assistant('hi there', 'anthropic::claude-haiku-4-5', 2),
      user('follow up', 3),
    ])
    expect(out).toHaveLength(3)
    expect(out[0]).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'hello' }],
      timestamp: 1,
    })
    expect(out[1]).toMatchObject({
      role: 'assistant',
      content: [{ type: 'text', text: 'hi there' }],
      stop_reason: 'end',
      model: 'anthropic::claude-haiku-4-5',
      provider: 'anthropic',
      timestamp: 2,
    })
    expect(out[2]).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'follow up' }],
      timestamp: 3,
    })
  })

  it('drops thoughts, function-calls, and system notices', () => {
    const out = translateUiHistoryForBackend([
      user('hi', 1),
      {
        id: 't1',
        role: 'thought',
        content: 'thinking…',
        durationMs: 100,
        createdAt: 2,
      },
      {
        id: 'fc1',
        role: 'function-call',
        functionId: 'engine::echo',
        input: { x: 1 },
        output: { y: 2 },
        createdAt: 3,
      },
      {
        id: 's1',
        role: 'system',
        content: 'compacted — 1k tokens',
        tone: 'info',
        createdAt: 4,
      },
      assistant('done', 'anthropic::claude-haiku-4-5', 5),
    ])
    expect(out.map((m) => m.role)).toEqual(['user', 'assistant'])
  })

  it('drops assistant turns with empty content (aborted/errored)', () => {
    const out = translateUiHistoryForBackend([
      user('hi', 1),
      assistant('', undefined, 2),
      user('try again', 3),
    ])
    expect(out.map((m) => m.role)).toEqual(['user', 'user'])
  })

  it('infers provider from heuristic model ids without `::`', () => {
    const out = translateUiHistoryForBackend([
      assistant('a', 'claude-haiku-4-5', 1),
      assistant('b', 'gemini-2.0-flash', 2),
      assistant('c', 'gpt-4o', 3),
    ])
    expect(out).toHaveLength(3)
    if (out[0]?.role === 'assistant') expect(out[0].provider).toBe('anthropic')
    if (out[1]?.role === 'assistant') expect(out[1].provider).toBe('google')
    if (out[2]?.role === 'assistant') expect(out[2].provider).toBe('openai')
  })

  it('handles assistant turns with no model id (defaults to empty provider)', () => {
    const out = translateUiHistoryForBackend([assistant('hello', undefined, 1)])
    expect(out).toHaveLength(1)
    if (out[0]?.role === 'assistant') {
      expect(out[0].model).toBe('')
      expect(out[0].provider).toBe('')
    }
  })

  it('returns empty array for empty input', () => {
    expect(translateUiHistoryForBackend([])).toEqual([])
  })

  describe('compaction markers', () => {
    const compactionMarker = (
      summaryText: string,
      createdAt = 100,
    ): Message => ({
      id: `c-${createdAt}`,
      role: 'system',
      kind: 'compaction',
      content: 'compacted',
      tone: 'info',
      summaryText,
      tokensBefore: 12_345,
      createdAt,
    })

    it('replaces pre-marker history with one assistant <conversation-summary>', () => {
      const out = translateUiHistoryForBackend([
        user('first turn', 1),
        assistant('first answer', 'anthropic::claude-haiku-4-5', 2),
        user('second turn', 3),
        assistant('second answer', 'anthropic::claude-haiku-4-5', 4),
        compactionMarker('Discussed turns 1-2 about X and Y.', 5),
      ])
      // Pre-marker user+assistant turns are SHED; the marker itself becomes
      // a single assistant <conversation-summary> block.
      expect(out).toHaveLength(1)
      expect(out[0]).toMatchObject({
        role: 'assistant',
        stop_reason: 'end',
        timestamp: 5,
      })
      const block = out[0]?.role === 'assistant' ? out[0].content[0] : null
      if (block && block.type === 'text') {
        expect(block.text).toContain('<conversation-summary>')
        expect(block.text).toContain('Discussed turns 1-2 about X and Y.')
        expect(block.text).toContain('</conversation-summary>')
      } else {
        throw new Error('expected first content block to be text')
      }
    })

    it('keeps post-marker messages and ships them after the summary', () => {
      const out = translateUiHistoryForBackend([
        user('shed me', 1),
        assistant('shed me too', 'anthropic::claude-haiku-4-5', 2),
        compactionMarker('Summary of the shed turns.', 3),
        user('survives', 4),
        assistant('also survives', 'anthropic::claude-haiku-4-5', 5),
      ])
      expect(out.map((m) => m.role)).toEqual(['assistant', 'user', 'assistant'])
      const summary = out[0]?.role === 'assistant' ? out[0].content[0] : null
      if (summary && summary.type === 'text') {
        expect(summary.text).toContain('Summary of the shed turns.')
      } else {
        throw new Error('expected summary text block')
      }
      expect(out[1]).toMatchObject({
        role: 'user',
        content: [{ type: 'text', text: 'survives' }],
      })
    })

    it('uses only the LAST marker when several compactions stacked up', () => {
      const out = translateUiHistoryForBackend([
        user('ancient', 1),
        compactionMarker('first summary (covers ancient)', 2),
        user('older', 3),
        assistant('older answer', 'anthropic::claude-haiku-4-5', 4),
        compactionMarker('second summary (covers older too)', 5),
        user('current', 6),
      ])
      expect(out).toHaveLength(2)
      const summary = out[0]?.role === 'assistant' ? out[0].content[0] : null
      if (summary && summary.type === 'text') {
        expect(summary.text).toContain('second summary')
        expect(summary.text).not.toContain('first summary')
      } else {
        throw new Error('expected summary text block')
      }
      expect(out[1]).toMatchObject({ role: 'user' })
    })

    it('falls back to plain translation when marker has no summaryText', () => {
      const marker: Message = {
        id: 'm1',
        role: 'system',
        kind: 'compaction',
        content: 'compacted',
        tone: 'info',
        createdAt: 5,
      }
      const out = translateUiHistoryForBackend([
        user('a', 1),
        marker,
        user('b', 6),
      ])
      // Marker emits nothing (no summaryText); pre-marker user is shed; only
      // post-marker user survives. Pre-marker shedding is the important
      // invariant — we never want to ship summarised content alongside the
      // raw turns it replaces.
      expect(out).toHaveLength(1)
      expect(out[0]).toMatchObject({
        role: 'user',
        content: [{ type: 'text', text: 'b' }],
      })
    })
  })
})
