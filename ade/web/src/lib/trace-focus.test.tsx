// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import {
  requestTraceFocus,
  takeTraceFocusRequest,
  useTraceFocusRequest,
} from './trace-focus'

function Probe({ onFocus }: { onFocus: (traceId: string) => void }) {
  useTraceFocusRequest(onFocus)
  return null
}

describe('trace focus requests', () => {
  it('wait for a traces screen that mounts later, and go to one taker', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    requestTraceFocus('  t1 ')
    const first = vi.fn()
    const second = vi.fn()
    const container = document.createElement('div')
    const root = createRoot(container)
    await act(async () =>
      root.render(
        <>
          <Probe onFocus={first} />
          <Probe onFocus={second} />
        </>,
      ),
    )
    expect(first).toHaveBeenCalledWith('t1')
    expect(second).not.toHaveBeenCalled()

    // A later request reaches the mounted screen right away.
    await act(async () => requestTraceFocus('t2'))
    expect([...first.mock.calls, ...second.mock.calls].flat()).toContain('t2')
    expect(takeTraceFocusRequest()).toBeNull()

    await act(async () => root.unmount())
    vi.unstubAllGlobals()
  })

  it('ignores a blank id', () => {
    requestTraceFocus('   ')
    expect(takeTraceFocusRequest()).toBeNull()
  })
})
