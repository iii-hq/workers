// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SheetPage } from './SheetNavigation'

let root: Root | undefined
let container: HTMLDivElement | undefined

function renderPage(onBack?: () => void) {
  act(() => {
    root?.render(
      <SheetPage title="Details" onBack={onBack} dialogSemantics={false}>
        <input aria-label="detail value" />
      </SheetPage>,
    )
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    callback(0)
    return 1
  })
  vi.stubGlobal('cancelAnimationFrame', vi.fn())
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  vi.unstubAllGlobals()
})

describe('SheetPage focus handoff', () => {
  it('focuses Back after mounting a navigable page', () => {
    renderPage(() => {})

    expect(document.activeElement).toBe(
      container?.querySelector('button[aria-label="Back"]'),
    )
  })

  it('keeps child input focus when an inline onBack callback changes', () => {
    renderPage(() => {})
    const input = container?.querySelector<HTMLInputElement>(
      'input[aria-label="detail value"]',
    )
    expect(input).toBeTruthy()
    input?.focus()
    expect(document.activeElement).toBe(input)

    renderPage(() => {})

    expect(document.activeElement).toBe(input)
  })

  it('does not focus when the page has no Back control', () => {
    const prior = document.createElement('button')
    document.body.appendChild(prior)
    prior.focus()

    renderPage()

    expect(document.activeElement).toBe(prior)
    prior.remove()
  })
})
