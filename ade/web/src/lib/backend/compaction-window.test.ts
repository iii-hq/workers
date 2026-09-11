import { describe, expect, it } from 'vitest'

import type { TranscriptItem } from '@/lib/sessions/types'

import { compactionWindow, latestCompactionAnchor } from './compaction-window'

function user(entryId: string, text: string): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'user',
      content: [{ type: 'text', text }],
      timestamp: 1,
    },
  }
}

function assistant(entryId: string, text: string): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'assistant',
      content: [{ type: 'text', text }],
      stop_reason: 'end',
      model: 'm',
      provider: 'p',
      timestamp: 2,
    },
  }
}

function compaction(
  entryId: string,
  data: Record<string, unknown>,
): TranscriptItem {
  return { entry_id: entryId, custom: { custom_type: 'compaction', data } }
}

describe('latestCompactionAnchor', () => {
  it('is null for a never-compacted path', () => {
    expect(
      latestCompactionAnchor([user('u1', 'hi'), assistant('a1', 'yo')]),
    ).toBe(null)
  })

  it('reads summary and boundary from the LATEST compaction entry', () => {
    const items = [
      user('u1', 'first'),
      compaction('c1', { summary: 'old', tail_start_entry_id: 'u1' }),
      user('u2', 'second'),
      compaction('c2', { summary: 'new', tail_start_entry_id: 'u2' }),
      user('u3', 'third'),
    ]
    expect(latestCompactionAnchor(items)).toEqual({
      summary: 'new',
      tailStartEntryId: 'u2',
    })
  })

  it('tolerates malformed or partial data and ignores other custom types', () => {
    expect(
      latestCompactionAnchor([
        {
          entry_id: 'x',
          custom: { custom_type: 'reaction', data: { summary: 's' } },
        },
        compaction('c1', { summary: 42, tail_start_entry_id: null }),
      ]),
    ).toEqual({ summary: null, tailStartEntryId: null })
    expect(
      latestCompactionAnchor([
        { entry_id: 'c', custom: { custom_type: 'compaction', data: 'junk' } },
      ]),
    ).toEqual({ summary: null, tailStartEntryId: null })
  })
})

describe('compactionWindow', () => {
  const items: TranscriptItem[] = [
    user('u1', 'first'),
    assistant('a1', 'r1'),
    compaction('c1', { summary: 's', tail_start_entry_id: 'u2' }),
    user('u2', 'second'),
    {
      entry_id: 'k1',
      message: {
        role: 'custom',
        custom_type: 'ui_marker',
        content: [],
        timestamp: 3,
      },
    },
    assistant('a2', 'r2'),
  ]

  it('opens at the boundary and skips custom entries and custom-role messages', () => {
    expect(compactionWindow(items, 'u2').map((e) => e.entry_id)).toEqual([
      'u2',
      'a2',
    ])
  })

  it('null boundary means the whole path minus non-model-bound rows', () => {
    expect(compactionWindow(items, null).map((e) => e.entry_id)).toEqual([
      'u1',
      'a1',
      'u2',
      'a2',
    ])
  })

  it('a boundary missing from the path falls back to the whole path', () => {
    expect(compactionWindow(items, 'gone').map((e) => e.entry_id)).toEqual([
      'u1',
      'a1',
      'u2',
      'a2',
    ])
  })

  it('keeps entry ids aligned with message positions for tail_start_index mapping', () => {
    const window = compactionWindow(items, null)
    // context::compact indexes the `messages` array it receives; the window
    // is that array with its entry id alongside, so index 2 -> 'u2'.
    expect(window[2].entry_id).toBe('u2')
    expect(window[2].message.role).toBe('user')
  })
})
