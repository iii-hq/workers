// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CodeHighlight, JsonHighlight } from './syntax'

let root: Root | null = null
let host: HTMLDivElement
let observed: Array<(entries: { isIntersecting: boolean }[]) => void> = []

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  observed = []
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(cb: (entries: { isIntersecting: boolean }[]) => void) {
        observed.push(cb)
      }
      observe() {}
      disconnect() {}
    },
  )
  host = document.body.appendChild(document.createElement('div'))
  root = createRoot(host)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  host.remove()
  vi.unstubAllGlobals()
})

const SOURCE = 'const answer = 42\nconsole.log(answer)'

describe('lazy highlighting', () => {
  it('shows a block as plain text until it nears the viewport', async () => {
    await act(async () =>
      root?.render(<CodeHighlight code={SOURCE} language="javascript" />),
    )
    const pre = host.querySelector('pre')
    expect(pre?.hasAttribute('data-highlight-pending')).toBe(true)
    expect(pre?.textContent).toBe(SOURCE)
    expect(host.querySelectorAll('.token').length).toBe(0)

    await act(async () => observed[0]?.([{ isIntersecting: true }]))
    expect(host.querySelector('[data-highlight-pending]')).toBeNull()
    expect(host.querySelectorAll('.token').length).toBeGreaterThan(0)
    expect(host.querySelector('pre')?.textContent).toBe(SOURCE)
  })

  it('treats JSON payloads the same way', async () => {
    await act(async () => root?.render(<JsonHighlight code='{"a": 1}' />))
    expect(host.querySelector('[data-highlight-pending]')).not.toBeNull()
    await act(async () => observed[0]?.([{ isIntersecting: true }]))
    expect(host.querySelectorAll('.token').length).toBeGreaterThan(0)
  })
})
