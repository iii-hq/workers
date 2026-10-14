// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DEVICE_PROVIDERS } from '@/lib/onboarding/catalog'
import { DeviceSignIn } from './DeviceSignIn'

const copyText = vi.fn(async (_text: string) => true)
vi.mock('@/lib/clipboard', () => ({
  copyTextToClipboard: (text: string) => copyText(text),
}))
vi.mock('@/lib/onboarding/device-login', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/onboarding/device-login')>()),
  startDeviceLogin: async () => ({
    device_code: 'device-1',
    user_code: 'B45D-2A43',
    verification_uri: 'https://github.com/login/device',
    expires_in: 900,
    interval: 5,
  }),
}))

const copilot = DEVICE_PROVIDERS[0]
let container: HTMLDivElement
let root: Root

beforeEach(() => {
  ;(
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true
  vi.useFakeTimers()
  copyText.mockClear()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.useRealTimers()
})

const codeButton = () => {
  const found = container.querySelector<HTMLButtonElement>(
    'button[aria-label*="B45D-2A43"]',
  )
  if (!found) throw new Error('no code button')
  return found
}

describe('DeviceSignIn copy', () => {
  it('says the code was copied, then goes back to Copy', async () => {
    await act(async () => root.render(<DeviceSignIn provider={copilot} />))
    expect(codeButton().textContent).toContain('Copy')
    expect(container.textContent).toContain('github.com/login/device')

    await act(async () => codeButton().click())
    expect(copyText).toHaveBeenCalledWith('B45D-2A43')
    expect(codeButton().textContent).toContain('Copied')
    expect(codeButton().getAttribute('aria-label')).toBe(
      'Code B45D-2A43 copied',
    )
    expect(container.querySelector('[role="status"]')?.textContent).toBe(
      'Code copied to the clipboard.',
    )

    await act(async () => vi.advanceTimersByTime(2_000))
    expect(codeButton().textContent).not.toContain('Copied')
  })

  it('leaves out its own heading where the provider row already has one', async () => {
    await act(async () =>
      root.render(<DeviceSignIn provider={copilot} heading={false} />),
    )
    expect(container.textContent).not.toContain('Sign in with GitHub')
    expect(container.textContent).toContain('Copy code & open GitHub')
  })
})
