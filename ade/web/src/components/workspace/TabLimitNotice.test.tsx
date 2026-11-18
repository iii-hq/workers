// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requestTabLimitNotice } from '@/lib/tab-limit'
import { TAB_LIMIT_NOTICE_MS, TabLimitNotice } from './TabLimitNotice'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.useFakeTimers()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(() => {
  act(() => root.unmount())
  host.remove()
  vi.useRealTimers()
})

describe('TabLimitNotice', () => {
  it('names the limit when a new workspace is refused, then goes', () => {
    act(() => root.render(<TabLimitNotice limit={3} />))
    expect(host.textContent).toBe('')
    act(() => requestTabLimitNotice())
    expect(host.querySelector('[role="status"]')?.textContent).toContain(
      'Tab limit reached (3)',
    )
    act(() => vi.advanceTimersByTime(TAB_LIMIT_NOTICE_MS + 10))
    expect(host.textContent).toBe('')
  })
})
