import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createDictationKeyGesture, DICTATION_KEY_HOLD_MS, isDictationKey } from './dictation-key'

const key = (code: string, key: string, extra: Partial<KeyboardEventInit> = {}) => ({
  code,
  key,
  repeat: false,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  ...extra,
})
const rightOption = (extra: Partial<KeyboardEventInit> = {}) => key('AltRight', 'Alt', extra)

describe('isDictationKey', () => {
  it('matches the right Option key alone', () => {
    expect(isDictationKey(rightOption())).toBe(true)
    expect(isDictationKey(key('AltLeft', 'Alt'))).toBe(false)
    expect(isDictationKey(key('AltRight', 'AltGraph'))).toBe(false)
    expect(isDictationKey(rightOption({ ctrlKey: true }))).toBe(false)
    expect(isDictationKey(rightOption({ metaKey: true }))).toBe(false)
    expect(isDictationKey(key('Fn', 'Fn'))).toBe(false)
  })
})

describe('right Option gesture', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  const setup = () => {
    const actions = { toggle: vi.fn(), holdStart: vi.fn(), holdEnd: vi.fn() }
    const gesture = createDictationKeyGesture(actions, {
      set: (fn, ms) => setTimeout(fn, ms) as unknown as number,
      clear: (id) => clearTimeout(id),
    })
    return { actions, gesture }
  }

  it('a quick tap toggles', () => {
    const { actions, gesture } = setup()
    gesture.keydown(rightOption())
    vi.advanceTimersByTime(100)
    gesture.keyup(rightOption())
    expect(actions.toggle).toHaveBeenCalledTimes(1)
    expect(actions.holdStart).not.toHaveBeenCalled()
  })

  it('holding talks until release', () => {
    const { actions, gesture } = setup()
    gesture.keydown(rightOption())
    gesture.keydown(rightOption({ repeat: true }))
    vi.advanceTimersByTime(DICTATION_KEY_HOLD_MS)
    expect(actions.holdStart).toHaveBeenCalledTimes(1)
    gesture.keyup(rightOption())
    expect(actions.holdEnd).toHaveBeenCalledTimes(1)
    expect(actions.toggle).not.toHaveBeenCalled()
  })

  it('a ⌥ character or shortcut is not dictation', () => {
    const { actions, gesture } = setup()
    gesture.keydown(rightOption())
    gesture.keydown(key('KeyE', 'Dead'))
    vi.advanceTimersByTime(DICTATION_KEY_HOLD_MS * 2)
    gesture.keyup(rightOption())
    expect(actions.toggle).not.toHaveBeenCalled()
    expect(actions.holdStart).not.toHaveBeenCalled()
  })

  it('losing focus mid-hold finishes the dictation', () => {
    const { actions, gesture } = setup()
    gesture.keydown(rightOption())
    vi.advanceTimersByTime(DICTATION_KEY_HOLD_MS)
    gesture.blur()
    expect(actions.holdEnd).toHaveBeenCalledTimes(1)
    gesture.keyup(rightOption())
    expect(actions.holdEnd).toHaveBeenCalledTimes(1)
  })
})
