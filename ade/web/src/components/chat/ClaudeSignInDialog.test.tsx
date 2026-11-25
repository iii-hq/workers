// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'

/* The worker's login_start / login_code / login_cancel, as fakes: no
   engine, no Anthropic. */
const calls = vi.hoisted(() => ({
  startLogin: vi.fn(),
  submitLoginCode: vi.fn(),
  cancelLogin: vi.fn(),
  refreshClaudeAccounts: vi.fn(),
}))
vi.mock('@/lib/backend/claude-accounts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/backend/claude-accounts')>()),
  ...calls,
}))

import { ClaudeSignInDialog } from './ClaudeSignInDialog'

const NOW = Date.now()
const target = {
  id: 'personal',
  name: 'Personal',
  color: 'violet',
  email: 'me@home.dev',
  isNew: true,
}

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  calls.startLogin.mockResolvedValue({
    loginId: 'login_1',
    account: 'personal',
    url: 'https://claude.ai/oauth/authorize?fake=1',
    expiresAtMs: NOW + 600_000,
    email: 'me@home.dev',
  })
  calls.cancelLogin.mockResolvedValue(undefined)
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
  for (const fn of Object.values(calls)) fn.mockReset()
})

async function render(onMakeGlobal = vi.fn()) {
  const onOpenChange = vi.fn()
  root = createRoot(document.body.appendChild(document.createElement('div')))
  await act(async () =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ClaudeSignInDialog
          target={target}
          onOpenChange={onOpenChange}
          globalName="Work"
          globalWeek={64}
          isGlobal={false}
          onMakeGlobal={onMakeGlobal}
        />
      </TooltipProvider>,
    ),
  )
  return { onOpenChange, onMakeGlobal }
}

const dialog = () =>
  document.body.querySelector('[role="dialog"]') as HTMLElement
const button = (text: string) =>
  [...dialog().querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined

async function submitCode(value: string) {
  const input = dialog().querySelector(
    'input[name="claude-sign-in-code"]',
  ) as HTMLInputElement
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  act(() => {
    setter?.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button('Sign in')?.click())
}

describe('signing a Claude Code account in', () => {
  it('opens the link, takes the code, then offers to make it global', async () => {
    calls.submitLoginCode.mockResolvedValue({ email: 'me@home.dev' })
    const { onMakeGlobal } = await render()
    expect(calls.startLogin).toHaveBeenCalledWith('personal')
    expect(dialog().textContent).toContain('Sign in to Claude Code')
    expect(dialog().textContent).toContain('Personal · new account')
    const link = dialog().querySelector('a[target="_blank"]')
    expect(link?.getAttribute('href')).toBe(
      'https://claude.ai/oauth/authorize?fake=1',
    )
    expect(link?.getAttribute('rel')).toBe('noopener noreferrer')
    await submitCode(' abc 123 ')
    expect(calls.submitLoginCode).toHaveBeenCalledWith('login_1', 'abc123')
    expect(calls.refreshClaudeAccounts).toHaveBeenCalledWith(true)
    expect(dialog().textContent).toContain('Personal is signed in')
    expect(dialog().textContent).toContain(
      'Work is the global account (64% this week)',
    )
    act(() => button('Make Personal global')?.click())
    expect(onMakeGlobal).toHaveBeenCalledWith('personal')
  })

  it('keeps the link on a code Anthropic rejected', async () => {
    calls.submitLoginCode.mockRejectedValue(
      new Error('{"code":"rejected","message":"invalid_grant"}'),
    )
    await render()
    await submitCode('wrong')
    expect(dialog().querySelector('[role="alert"]')?.textContent).toBe(
      "Anthropic didn't accept this code. A code works once, with the link it came from: open the link again for a new one.",
    )
    expect(button('Sign in')).toBeDefined()
  })

  it('stops on an expired link and cancels a sign-in left open', async () => {
    calls.submitLoginCode.mockRejectedValue(
      new Error('{"code":"expired","message":"expired"}'),
    )
    await render()
    await submitCode('late')
    expect(dialog().textContent).toContain('The sign-in link expired')
    // Nothing left to cancel once the worker ended it.
    act(() => root?.unmount())
    root = null
    expect(calls.cancelLogin).not.toHaveBeenCalled()
  })

  it('cancels the login when closed while it waits', async () => {
    await render()
    act(() => root?.unmount())
    root = null
    expect(calls.cancelLogin).toHaveBeenCalledWith('login_1')
  })
})
