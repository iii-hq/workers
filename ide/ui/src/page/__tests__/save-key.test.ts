import { describe, expect, it } from 'vitest'
import { isSaveKey, saveKeyAria, saveKeyLabel } from '../save-key'

const key = (overrides: Partial<Parameters<typeof isSaveKey>[0]>) => ({
  key: 's',
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...overrides,
})

describe('isSaveKey', () => {
  it('takes ⌘S (macOS) and Ctrl+S (Windows/Linux)', () => {
    expect(isSaveKey(key({ metaKey: true }))).toBe(true)
    expect(isSaveKey(key({ ctrlKey: true }))).toBe(true)
  })

  it('takes the chord with Caps Lock on', () => {
    expect(isSaveKey(key({ key: 'S', metaKey: true }))).toBe(true)
  })

  it('leaves a bare s and other chords alone', () => {
    expect(isSaveKey(key({}))).toBe(false)
    expect(isSaveKey(key({ key: 'p', metaKey: true }))).toBe(false)
    expect(isSaveKey(key({ altKey: true }))).toBe(false)
  })
})

describe('save key labels', () => {
  it('prints the chord for the reader', () => {
    expect(saveKeyLabel('mac')).toBe('⌘S')
    expect(saveKeyLabel('other')).toBe('Ctrl+S')
    expect(saveKeyAria('mac')).toBe('Meta+S')
    expect(saveKeyAria('other')).toBe('Control+S')
  })
})
