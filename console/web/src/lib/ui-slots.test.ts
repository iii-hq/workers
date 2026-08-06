/**
 * The session-chip slot's registry semantics — the piece worker chips
 * depend on without being able to see it: last registration wins per id,
 * unregistering restores what it shadowed, and removal is idempotent.
 */

import { describe, expect, it } from 'vitest'
import type { RegisteredSessionChip } from './ui-slots'
import { getExtSessionChips, registerExtSessionChip } from './ui-slots'

function chip(id: string, path: string): RegisteredSessionChip {
  return { id, path, scope: path.split('/')[0], render: () => null }
}

describe('session chip slot', () => {
  it('registers in order and dedupes by id, last registration winning', () => {
    const offA = registerExtSessionChip(chip('context', 'harness/page.js'))
    const offB = registerExtSessionChip(chip('cost', 'llm-budget/page.js'))
    const offC = registerExtSessionChip(chip('context', 'other/page.js'))

    const chips = getExtSessionChips()
    expect(chips.map((c) => c.id)).toEqual(['context', 'cost'])
    expect(chips.find((c) => c.id === 'context')?.path).toBe('other/page.js')

    offA()
    offB()
    offC()
    expect(getExtSessionChips()).toEqual([])
  })

  it('restores the shadowed chip when the override unregisters', () => {
    const offA = registerExtSessionChip(chip('context', 'harness/page.js'))
    const offB = registerExtSessionChip(chip('context', 'other/page.js'))

    offB()
    expect(getExtSessionChips().map((c) => c.path)).toEqual(['harness/page.js'])

    offA()
    offA()
    expect(getExtSessionChips()).toEqual([])
  })
})
