// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { loadPaneChat, savePaneChat } from './chat-pane-selection'

const blocked = (name: 'sessionStorage' | 'localStorage') =>
  Object.defineProperty(globalThis, name, {
    configurable: true,
    get() {
      throw new DOMException('blocked', 'SecurityError')
    },
  })

const original = {
  sessionStorage: Object.getOwnPropertyDescriptor(globalThis, 'sessionStorage'),
  localStorage: Object.getOwnPropertyDescriptor(globalThis, 'localStorage'),
}

afterEach(() => {
  for (const [name, descriptor] of Object.entries(original))
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
})

describe('chat pane selection with storage blocked', () => {
  it('neither loading nor saving throws', () => {
    blocked('sessionStorage')
    blocked('localStorage')
    expect(loadPaneChat('tab:pane')).toBeNull()
    expect(() => savePaneChat('tab:pane', 'chat-a')).not.toThrow()
  })
})
