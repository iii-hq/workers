import { describe, expect, it, vi } from 'vitest'
import {
  requestTraceFocus,
  subscribeTraceFocus,
  takeTraceFocusRequest,
} from './trace-focus'

describe('trace focus requests', () => {
  it('wait for a traces screen that subscribes later, and go to one taker', () => {
    requestTraceFocus('  t1 ')
    const first = vi.fn()
    const second = vi.fn()
    const offFirst = subscribeTraceFocus(first)
    const offSecond = subscribeTraceFocus(second)
    expect(first).toHaveBeenCalledWith('t1')
    expect(second).not.toHaveBeenCalled()

    // A later request reaches the subscribed screen right away.
    requestTraceFocus('t2')
    expect([...first.mock.calls, ...second.mock.calls].flat()).toContain('t2')
    expect(takeTraceFocusRequest()).toBeNull()

    offFirst()
    offSecond()
  })

  it('ignores a blank id', () => {
    requestTraceFocus('   ')
    expect(takeTraceFocusRequest()).toBeNull()
  })
})
