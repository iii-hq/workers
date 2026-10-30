import { describe, expect, it, vi } from 'vitest'
import {
  onShortcutsDialogRequest,
  requestShortcutsDialog,
} from './shortcuts-request'

describe('shortcuts dialog requests', () => {
  it('reaches every listener until it unsubscribes', () => {
    const first = vi.fn()
    const second = vi.fn()
    const offFirst = onShortcutsDialogRequest(first)
    onShortcutsDialogRequest(second)

    requestShortcutsDialog()
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)

    offFirst()
    requestShortcutsDialog()
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(2)
  })

  it('is a no-op with nobody listening', () => {
    expect(() => requestShortcutsDialog()).not.toThrow()
  })
})
