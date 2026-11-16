// @vitest-environment jsdom
/**
 * The prompt with its real hook and real storage: what each way out of the
 * dialog does to this run and to the next one. `EmailPrompt.test.tsx` covers
 * the dialog's own states with the hook mocked open.
 */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PROMPT_AFTER_MS, parseRecord, SNOOZE_MS } from '@/lib/email-prompt'
import { getIiiClient } from '@/lib/iii-client'
import { EmailPrompt } from './EmailPrompt'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const KEY = 'iii-email-prompt'
const call = vi.fn()
let root: Root
let host: HTMLDivElement

/** A prompt that has earned its thirty minutes and was never answered. */
const earn = () =>
  localStorage.setItem(
    KEY,
    JSON.stringify({
      status: 'pending',
      activeMs: PROMPT_AFTER_MS,
      snoozeUntil: 0,
    }),
  )

const stored = () => parseRecord(localStorage.getItem(KEY))
const isOpen = () => document.querySelector('[role="dialog"]') !== null

/** A run of the console: a fresh mount that reads storage again. */
async function run() {
  if (root) await act(async () => root.unmount())
  host?.remove()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root.render(<EmailPrompt />))
}

const button = (name: string) =>
  [...document.querySelectorAll('button')].find(
    (candidate) => candidate.textContent === name,
  )

async function submit(email: string) {
  const input = document.querySelector<HTMLInputElement>('input[type="email"]')
  if (!input) throw new Error('no email input')
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set?.call(input, email)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => input.form?.requestSubmit())
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'info').mockImplementation(() => {})
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
  localStorage.clear()
  earn()
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('the prompt across runs', () => {
  it('opens once the thirty minutes are earned', async () => {
    await run()
    expect(isOpen()).toBe(true)
  })

  it('stays closed before they are earned', async () => {
    localStorage.removeItem(KEY)
    await run()
    expect(isOpen()).toBe(false)
  })

  it('on an outage, closes for this run and asks again on the next', async () => {
    call.mockResolvedValue({ subscribed: false, outcome: 'retry_later' })
    await run()
    await submit('name@example.com')
    expect(isOpen()).toBe(false)
    // No decision and no snooze: nothing about the record changed.
    expect(stored()).toMatchObject({ status: 'pending', snoozeUntil: 0 })

    await run()
    expect(isOpen()).toBe(true)
  })

  it('after a signup, saves it before Done and never asks again', async () => {
    call.mockResolvedValue({ subscribed: true, outcome: 'subscribed' })
    await run()
    await submit('name@example.com')
    expect(stored().status).toBe('subscribed')
    expect(isOpen()).toBe(true)

    await act(async () => button('Done')?.click())
    expect(isOpen()).toBe(false)
    await run()
    expect(isOpen()).toBe(false)
  })

  it('on Not now, waits out the snooze', async () => {
    await run()
    const before = Date.now()
    await act(async () => button('Not now')?.click())
    expect(isOpen()).toBe(false)
    expect(stored().status).toBe('pending')
    expect(stored().snoozeUntil).toBeGreaterThanOrEqual(before + SNOOZE_MS)

    await run()
    expect(isOpen()).toBe(false)
  })

  it("on Don't ask again, never asks again", async () => {
    await run()
    await act(async () => button("Don't ask again")?.click())
    expect(isOpen()).toBe(false)
    expect(stored().status).toBe('dismissed')

    await run()
    expect(isOpen()).toBe(false)
    expect(call).not.toHaveBeenCalled()
  })

  // jsdom lets a disabled input take focus, so this pins the outcome rather
  // than reproducing the bug: in Chrome, focusing before the re-enabling
  // render left the cursor outside the field.
  it('puts the cursor back in a refused address once it is enabled again', async () => {
    call.mockResolvedValue({ subscribed: false, outcome: 'invalid_email' })
    await run()
    await submit('name@example.com')
    const input = document.querySelector('input[type="email"]')
    expect(input?.hasAttribute('disabled')).toBe(false)
    expect(document.activeElement).toBe(input)
  })
})
