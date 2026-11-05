// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  loadDismissedTriggers,
  onDismissedTriggersChange,
  saveDismissedTriggers,
} from './dismissed-triggers'

beforeEach(() => localStorage.clear())

describe('dismissed triggers', () => {
  it('merges with what another pane saved instead of overwriting it', () => {
    saveDismissedTriggers(new Set(['a']))
    // A second pane mounted before the first dismissal still holds [].
    saveDismissedTriggers(new Set(['b']))
    expect([...loadDismissedTriggers()].sort()).toEqual(['a', 'b'])
  })

  it('tells mounted cards when a dismissal is saved', () => {
    const listener = vi.fn()
    const off = onDismissedTriggersChange(listener)
    saveDismissedTriggers(new Set(['c']))
    expect(listener).toHaveBeenCalledTimes(1)
    off()
    saveDismissedTriggers(new Set(['d']))
    expect(listener).toHaveBeenCalledTimes(1)
  })
})
