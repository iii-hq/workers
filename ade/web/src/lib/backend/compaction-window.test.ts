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
      entryId: 'c2',
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
    ).toEqual({ entryId: 'c1', summary: null, tailStartEntryId: null })
    expect(
      latestCompactionAnchor([
        { entry_id: 'c', custom: { custom_type: 'compaction', data: 'junk' } },
      ]),
    ).toEqual({ entryId: 'c', summary: null, tailStartEntryId: undefined })
    expect(
      latestCompactionAnchor([
        compaction('c1', { summary: 's', tail_start_entry_id: 7 }),
      ]),
    ).toEqual({ entryId: 'c1', summary: 's', tailStartEntryId: undefined })
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

  const anchor = (tail: string | null) => ({
    entryId: 'c1',
    summary: 's',
    tailStartEntryId: tail,
  })

  it('opens at the boundary and skips custom entries and custom-role messages', () => {
    expect(
      compactionWindow(items, anchor('u1')).map((e) => e.entry_id),
    ).toEqual(['u1', 'a1', 'u2', 'a2'])
    expect(
      compactionWindow(items, anchor('u2')).map((e) => e.entry_id),
    ).toEqual(['u2', 'a2'])
  })

  it('a never-compacted path is the whole path minus non-model-bound rows', () => {
    expect(compactionWindow(items, null).map((e) => e.entry_id)).toEqual([
      'u1',
      'a1',
      'u2',
      'a2',
    ])
  })

  it('a null boundary opens right after the compaction entry', () => {
    // context::compact with tail_turns 0 summarised everything before it.
    expect(
      compactionWindow(items, anchor(null)).map((e) => e.entry_id),
    ).toEqual(['u2', 'a2'])
  })

  it('a record without a summary or a usable boundary is the whole path', () => {
    // Only an explicit null opens after the record; a malformed record must
    // not drop the history it never summarised.
    for (const bad of [
      { entryId: 'c1', summary: null, tailStartEntryId: null },
      { entryId: 'c1', summary: 's', tailStartEntryId: undefined },
    ]) {
      expect(compactionWindow(items, bad).map((e) => e.entry_id)).toEqual([
        'u1',
        'a1',
        'u2',
        'a2',
      ])
    }
  })

  it('a boundary missing from the path falls back to the whole path', () => {
    expect(
      compactionWindow(items, anchor('gone')).map((e) => e.entry_id),
    ).toEqual(['u1', 'a1', 'u2', 'a2'])
  })

  it('replays model_notice entries as the user message the model was shown', () => {
    // Persisted by the harness (window.rs `notice_data`) and replayed to the
    // model on every later step, so the summariser must see them too.
    const stored = {
      role: 'user',
      content: [{ type: 'text', text: '<memory update="rules">R</memory>' }],
      timestamp: 5,
    }
    const notices: TranscriptItem[] = [
      user('u1', 'first'),
      {
        entry_id: 'n1',
        custom: {
          custom_type: 'model_notice',
          data: { kind: 'hook', text: 'ignored', message: stored },
        },
      },
      {
        entry_id: 'n2',
        custom: {
          custom_type: 'model_notice',
          data: { kind: 'runtime-context', text: 'fs_scope changed' },
        },
      },
      {
        entry_id: 'n3',
        custom: { custom_type: 'model_notice', data: { text: '' } },
      },
      {
        entry_id: 'o1',
        custom: { custom_type: 'message_order', data: {} },
      },
      assistant('a1', 'r1'),
    ]
    const window = compactionWindow(notices, null)
    expect(window.map((e) => e.entry_id)).toEqual(['u1', 'n1', 'n2', 'a1'])
    expect(window[1].message).toEqual(stored)
    expect(window[2].message).toEqual({
      role: 'user',
      content: [{ type: 'text', text: 'fs_scope changed' }],
      timestamp: 0,
    })
  })

  function order(
    entryId: string,
    data: Record<string, unknown>,
  ): TranscriptItem {
    return { entry_id: entryId, custom: { custom_type: 'message_order', data } }
  }

  it('applies message_order records, then cuts in the order the model saw', () => {
    // window.rs `the_compaction_tail_is_cut_in_the_order_the_model_saw`:
    // model order is u1 a1 u2 a2.
    const reordered: TranscriptItem[] = [
      user('u1', 'go'),
      user('u2', 'steer'),
      assistant('a1', 'ok'),
      order('o1', { after: 'a1', moved: ['u2'] }),
      assistant('a2', 'answer'),
    ]
    const ids = (tail: string | null) =>
      compactionWindow(reordered, {
        entryId: 'c1',
        summary: 's',
        tailStartEntryId: tail,
      }).map((e) => e.entry_id)
    expect(compactionWindow(reordered, null).map((e) => e.entry_id)).toEqual([
      'u1',
      'a1',
      'u2',
      'a2',
    ])
    // A tail from u2 drops a1: the order's anchor sits before the cut.
    expect(ids('u2')).toEqual(['u2', 'a2'])
    // A tail from a1 keeps u2, logged before a1 but shown after it.
    expect(ids('a1')).toEqual(['a1', 'u2', 'a2'])
    // A boundary on the record itself opens at the next message logged.
    expect(ids('o1')).toEqual(['a2'])
  })

  it('an order whose anchor is not model-bound or not on the path is a no-op', () => {
    for (const after of ['k1', 'gone']) {
      const path: TranscriptItem[] = [
        user('u1', 'go'),
        user('u2', 'steer'),
        items[4],
        assistant('a1', 'ok'),
        order('o1', { after, moved: ['u1'] }),
      ]
      expect(compactionWindow(path, null).map((e) => e.entry_id)).toEqual([
        'u1',
        'u2',
        'a1',
      ])
    }
  })

  it('an order listing its own anchor moves only the rest', () => {
    const path: TranscriptItem[] = [
      user('u1', 'go'),
      user('u2', 'steer'),
      assistant('a1', 'ok'),
      order('o1', { after: 'a1', moved: ['a1', 'u2', 'gone'] }),
      order('o2', { after: 'u1', moved: ['u1'] }),
    ]
    expect(compactionWindow(path, null).map((e) => e.entry_id)).toEqual([
      'u1',
      'a1',
      'u2',
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
