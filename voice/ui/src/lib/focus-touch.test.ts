import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { dictationStartedByTouch, isTouchPointer, setDictationStartedByTouch } from './focus-target'
import { deliverTranscript } from './mic'

vi.mock('@iii-dev/console-ui', () => ({ IconButton: 'button' }))

// No DOM here: just enough globals for deliverTranscript to look for a field.
beforeEach(() => {
  vi.stubGlobal('HTMLElement', class {})
  vi.stubGlobal('document', { activeElement: null })
  vi.stubGlobal('window', { setTimeout: vi.fn() })
})

afterEach(() => {
  setDictationStartedByTouch(false)
  vi.unstubAllGlobals()
})

describe('touch-started dictation', () => {
  it('treats fingers and pens as touch, never a mouse or a missing event', () => {
    expect(isTouchPointer({ pointerType: 'touch' })).toBe(true)
    expect(isTouchPointer({ pointerType: 'pen' })).toBe(true)
    expect(isTouchPointer({ pointerType: 'mouse' })).toBe(false)
    expect(isTouchPointer(undefined)).toBe(false)
  })

  it('releases the composer caret only after a dictation a finger started', async () => {
    const host = { chat: { compose: vi.fn() } }
    setDictationStartedByTouch(true)
    expect(await deliverTranscript(host, 'hello')).toBe('composer')
    expect(window.setTimeout).toHaveBeenCalled()
    expect(dictationStartedByTouch()).toBe(false)
  })

  it('keeps the caret in the composer for a keyboard or command dictation', async () => {
    const host = { chat: { compose: vi.fn() } }
    setDictationStartedByTouch(false)
    expect(await deliverTranscript(host, 'hello')).toBe('composer')
    expect(host.chat.compose).toHaveBeenCalledWith({ text: 'hello ' })
    expect(window.setTimeout).not.toHaveBeenCalled()
  })
})
