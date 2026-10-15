// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Composer } from './Composer'

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  // A phone: every layout query matches.
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: true,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }))
  root = createRoot(document.body.appendChild(document.createElement('div')))
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

async function render(compact: boolean) {
  await act(async () =>
    root?.render(
      <Composer
        model="anthropic::claude-sonnet-5-5"
        modelOptions={[]}
        permissionMode="manual"
        thinkingLevel="default"
        onModelChange={vi.fn()}
        onThinkingLevelChange={vi.fn()}
        onPermissionModeChange={vi.fn()}
        onSubmit={vi.fn()}
        compact={compact}
      />,
    ),
  )
}

const button = (label: string) =>
  [...document.querySelectorAll('button')].find(
    (el) =>
      el.getAttribute('aria-label') === label ||
      el.textContent?.trim() === label,
  )

describe('the compact composer on a phone', () => {
  it('is one row: settings, the field and send', async () => {
    await render(true)
    const shell = document.querySelector('[data-composer-compact]')
    expect(shell).not.toBeNull()
    const row = button('attach files, model and settings')?.parentElement
    expect(row?.querySelector('.composer-editor-slot')).not.toBeNull()
    expect(row?.querySelector('[aria-label="send message"]')).not.toBeNull()
    expect(button('chat settings')).toBeUndefined()
  })

  it('attaches files from the settings sheet', async () => {
    await render(true)
    const pick = vi
      .spyOn(HTMLInputElement.prototype, 'click')
      .mockImplementation(() => {})
    await act(async () => button('attach files, model and settings')?.click())
    await act(async () => button('Attach files')?.click())
    expect(pick).toHaveBeenCalledOnce()
    expect(button('Attach files')).toBeUndefined()
  })

  it('keeps the regular layout unless asked', async () => {
    await render(false)
    expect(document.querySelector('[data-composer-compact]')).toBeNull()
    expect(button('Attach files')).toBeUndefined()
  })
})
