// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryChip } from './MemoryChip'

const getMemory = vi.fn()
vi.mock('@/lib/memory', () => ({
  getMemory: (...args: unknown[]) => getMemory(...args),
}))

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }))
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  getMemory.mockReset()
  vi.unstubAllGlobals()
})

const chip = {
  bank: 'segralim',
  memories: 2,
  memoryIds: ['a', 'b'],
  memoryBanks: ['segralim', 'shared'],
  semantic: true,
}

async function open() {
  await act(async () => root.render(<MemoryChip memory={chip} />))
  const trigger =
    container.querySelector<HTMLButtonElement>('[data-memory-chip]')
  await act(async () => trigger?.click())
  for (let i = 0; i < 3; i++) {
    await act(async () => new Promise<void>((r) => setTimeout(r, 0)))
  }
  return trigger
}

describe('memory chip', () => {
  it('opens the memories in a popover, each from its own bank', async () => {
    getMemory.mockImplementation(async (bank: string, id: string) => ({
      id,
      text: `${id} from ${bank}`,
      pinned: id === 'b',
      updated_at: Date.parse('2026-11-20T12:00:00'),
      stated_at: null,
    }))
    const trigger = await open()
    expect(trigger?.textContent).toBe('2 memoriessegralim')
    expect(trigger?.getAttribute('aria-expanded')).toBe('true')
    expect(getMemory.mock.calls).toEqual([
      ['segralim', 'a'],
      ['shared', 'b'],
    ])
    const items = [...document.querySelectorAll('[data-memory-list] li')]
    expect(items.map((li) => li.textContent)).toEqual([
      'a from segralim2026-11-20',
      'b from sharedpinned · 2026-11-20',
    ])
    expect(items[1].querySelector('[data-pinned]')).not.toBeNull()
    // The popover renders outside the header: the chip's row holds the chip.
    expect(container.querySelector('[data-memory-list]')).toBeNull()
  })

  it('says a failed fetch failed, and retries it', async () => {
    getMemory.mockRejectedValueOnce(new Error('down'))
    await open()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'Could not load the memories.',
    )
    getMemory.mockResolvedValue({
      id: 'a',
      text: 'back',
      pinned: false,
      updated_at: 1,
    })
    const retry = [...document.querySelectorAll('button')].find(
      (b) => b.textContent === 'Retry',
    )
    await act(async () => retry?.click())
    expect(document.querySelector('[data-memory-list]')?.textContent).toContain(
      'back',
    )
  })
})
