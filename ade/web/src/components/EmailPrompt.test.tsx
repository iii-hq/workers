// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { EmailPrompt } from './EmailPrompt'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const prompt = {
  open: true,
  onSnooze: vi.fn(),
  onDismiss: vi.fn(),
  onSubscribed: vi.fn(),
  onRetryLater: vi.fn(),
  onClose: vi.fn(),
}
vi.mock('@/hooks/use-email-prompt', () => ({ useEmailPrompt: () => prompt }))

const call = vi.fn()
let root: Root
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** Type an address into the dialog and send it. */
async function submit(email: string) {
  await act(async () => root.render(<EmailPrompt />))
  const input = document.querySelector<HTMLInputElement>('input[type="email"]')
  if (!input) throw new Error('no email input')
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    setValue?.call(input, email)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => {
    input.form?.requestSubmit()
  })
}

const button = (name: string) =>
  [...document.querySelectorAll('button')].find(
    (candidate) => candidate.textContent === name,
  )

const alertText = () =>
  document.querySelector('[role="alert"]')?.textContent ?? null

describe('the signup answer', () => {
  it('saves the signup and asks the person to confirm from their inbox', async () => {
    call.mockResolvedValue({ subscribed: true, outcome: 'subscribed' })
    await submit(' name@example.com ')
    expect(call).toHaveBeenCalledWith(
      'ade::subscribe',
      { email: 'name@example.com', source: 'console_prompt' },
      expect.anything(),
    )
    // Saved straight away, so a reload never asks for the address again.
    expect(prompt.onSubscribed).toHaveBeenCalledOnce()
    expect(prompt.onRetryLater).not.toHaveBeenCalled()
    expect(alertText()).toBeNull()

    const dialog = document.querySelector('[role="dialog"]')
    expect(dialog?.textContent).toContain('Check your inbox')
    expect(dialog?.textContent).toContain(
      'We sent a confirmation link to name@example.com.',
    )
    expect(document.querySelector('input[type="email"]')).toBeNull()

    await act(async () => button('Done')?.click())
    expect(prompt.onClose).toHaveBeenCalledOnce()
    // Closing the confirmation step is not a snooze or a dismissal.
    expect(prompt.onSnooze).not.toHaveBeenCalled()
    expect(prompt.onDismiss).not.toHaveBeenCalled()
  })

  it('goes back to the address, as typed, to fix a typo', async () => {
    call.mockResolvedValue({ subscribed: true, outcome: 'subscribed' })
    await submit('nmae@example.com')
    await act(async () => button('Use a different email')?.click())
    const input = document.querySelector<HTMLInputElement>(
      'input[type="email"]',
    )
    expect(input?.value).toBe('nmae@example.com')
    expect(prompt.onClose).not.toHaveBeenCalled()
  })

  it('keeps the box open with an example when the list refuses the address', async () => {
    call.mockResolvedValue({ subscribed: false, outcome: 'invalid_email' })
    await submit('name@example.com')
    expect(alertText()).toBe('Enter an email address like name@example.com')
    const input = document.querySelector('input[type="email"]')
    expect(input?.getAttribute('aria-invalid')).toBe('true')
    expect(input?.getAttribute('aria-describedby')).toBe(
      document.querySelector('[role="alert"]')?.id,
    )
    expect(prompt.onSubscribed).not.toHaveBeenCalled()
    expect(prompt.onRetryLater).not.toHaveBeenCalled()
  })

  it('closes quietly and waits for a later run when the list cannot take it', async () => {
    call.mockResolvedValue({ subscribed: false, outcome: 'retry_later' })
    await submit('name@example.com')
    expect(prompt.onRetryLater).toHaveBeenCalledOnce()
    expect(prompt.onSnooze).not.toHaveBeenCalled()
    expect(prompt.onDismiss).not.toHaveBeenCalled()
    expect(alertText()).toBeNull()
  })

  it('treats an unreachable worker like any other outage', async () => {
    call.mockRejectedValue({ code: 'timeout', message: 'timed out' })
    await submit('name@example.com')
    expect(prompt.onRetryLater).toHaveBeenCalledOnce()
    expect(alertText()).toBeNull()
  })
})

describe('closing without an address', () => {
  it("asks again later on Not now, and never again on Don't ask again", async () => {
    await act(async () => root.render(<EmailPrompt />))
    await act(async () => button('Not now')?.click())
    expect(prompt.onSnooze).toHaveBeenCalledOnce()
    await act(async () => button("Don't ask again")?.click())
    expect(prompt.onDismiss).toHaveBeenCalledOnce()
    expect(call).not.toHaveBeenCalled()
  })
})
