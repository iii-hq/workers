import { describe, expect, it } from 'vitest'
import { nextHistoryTarget, type QueuedForEdit } from './queue-history'

const list: QueuedForEdit[] = [
  { id: 'a', text: 'first', attachments: [] },
  { id: 'b', text: 'second', attachments: [] },
  { id: 'c', text: 'third', attachments: [] },
]

/** Shorthand: the loaded target id, or 'noop'. */
function nav(
  browseId: string | null,
  current: string,
  dir: 'up' | 'down',
): string | null | 'noop' {
  const r = nextHistoryTarget(list, browseId, current, dir)
  return r.kind === 'noop' ? 'noop' : r.target.id
}

describe('nextHistoryTarget', () => {
  it('↑ from a blank composer enters at the newest', () => {
    expect(nav(null, '', 'up')).toBe('c')
  })

  it('↑ walks older, then clamps at the oldest', () => {
    expect(nav('c', 'third', 'up')).toBe('b')
    expect(nav('b', 'second', 'up')).toBe('a')
    expect(nav('a', 'first', 'up')).toBe('noop')
  })

  it('↓ walks newer, then exits to a live draft past the newest', () => {
    expect(nav('a', 'first', 'down')).toBe('b')
    expect(nav('b', 'second', 'down')).toBe('c')
    const exit = nextHistoryTarget(list, 'c', 'third', 'down')
    expect(exit).toEqual({
      kind: 'load',
      target: { id: null, text: '', attachments: [] },
    })
  })

  it('↓ from a live draft is a caret move, never enters browse', () => {
    expect(nav(null, '', 'down')).toBe('noop')
    expect(nav(null, 'typing', 'down')).toBe('noop')
  })

  it('never navigates once the editor is edited (pristine gate)', () => {
    // Browsing b but the text no longer matches → protect the edit.
    expect(nav('b', 'second, edited', 'up')).toBe('noop')
    expect(nav('b', 'second, edited', 'down')).toBe('noop')
    // A live draft with text is protected too.
    expect(nav(null, 'half-written', 'up')).toBe('noop')
  })

  it('loads the message text + attachments verbatim', () => {
    const withAttach: QueuedForEdit[] = [
      {
        id: 'x',
        text: 'hi',
        attachments: [{ id: 'f', name: 'a.txt', size: 1, type: 'text/plain' }],
      },
    ]
    const r = nextHistoryTarget(withAttach, null, '', 'up')
    expect(r).toEqual({
      kind: 'load',
      target: withAttach[0],
    })
  })

  it('clamps when the browsed id has left the list', () => {
    // Browsing a stale id (drained) with its old text: not pristine vs '' →
    // noop until the composer resets the cursor.
    expect(nav('gone', 'orphaned text', 'up')).toBe('noop')
  })
})
