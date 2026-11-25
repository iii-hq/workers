// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { openGeneralSettingsRow } from './reveal-general-row'

describe('openGeneralSettingsRow', () => {
  afterEach(() => {
    document.body.innerHTML = ''
    window.location.hash = ''
    vi.restoreAllMocks()
  })

  it('opens the General settings tab and scrolls to the first anchor that exists', () => {
    const row = document.createElement('div')
    row.id = 'settings-general-allowlist'
    document.body.append(row)
    const scroll = vi.fn()
    row.scrollIntoView = scroll
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((cb) => {
      cb(0)
      return 0
    })

    openGeneralSettingsRow(['missing-row', 'settings-general-allowlist'])

    expect(window.location.hash).toBe('#/configuration')
    expect(scroll).toHaveBeenCalledWith(
      expect.objectContaining({ block: 'center' }),
    )
    expect(row.hasAttribute('data-search-flash')).toBe(true)
  })
})
