// @vitest-environment jsdom

import { act, Profiler } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

/**
 * The conversations context as the real provider hands it out: a NEW object
 * on every store change (a streamed token included), with the active id in
 * it. Driven from the test.
 */
const ctx = vi.hoisted(() => {
  let value: { activeId: string | null; tick: number } = {
    activeId: 'console-a',
    tick: 0,
  }
  const listeners = new Set<() => void>()
  return {
    get: () => value,
    set(next: Partial<{ activeId: string | null }>) {
      value = { ...value, ...next, tick: value.tick + 1 }
      for (const listener of [...listeners]) listener()
    },
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
})

vi.mock('@/lib/conversations-context', async () => {
  const { useSyncExternalStore } = await import('react')
  return {
    useConversationsCtxOptional: () =>
      useSyncExternalStore(ctx.subscribe, ctx.get),
  }
})

import { registerExtOverlay } from '@/lib/ui-slots'
import { ExtOverlays } from './ExtOverlays'

let root: Root | null = null
let off: (() => void) | null = null
const seen: (string | null | undefined)[] = []
let overlayCommits = 0

function Counter({ conversationId }: { conversationId?: string | null }) {
  seen.push(conversationId)
  return null
}

beforeEach(() => {
  seen.length = 0
  overlayCommits = 0
  off = registerExtOverlay({
    id: 'test-counter',
    scope: 'test',
    path: 'test/page.js',
    render: (props) => (
      <Profiler id="overlay" onRender={() => overlayCommits++}>
        <Counter {...props} />
      </Profiler>
    ),
  })
  root = createRoot(document.createElement('div'))
  act(() => root?.render(<ExtOverlays />))
})

afterEach(() => {
  act(() => root?.unmount())
  off?.()
  root = null
})

describe('ExtOverlays', () => {
  it('does not re-render an overlay when the chat streams', () => {
    expect(seen).toEqual(['console-a'])
    // Streaming: the context changes many times, the active chat does not.
    for (let i = 0; i < 20; i++) act(() => ctx.set({}))
    expect(seen).toEqual(['console-a'])
    expect(overlayCommits).toBe(1)
  })

  it('re-renders an overlay with the new chat when the active chat changes', () => {
    act(() => ctx.set({ activeId: 'console-b' }))
    expect(seen).toEqual(['console-a', 'console-b'])
    act(() => ctx.set({ activeId: null }))
    expect(seen.at(-1)).toBeNull()
  })
})
