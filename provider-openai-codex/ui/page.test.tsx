import { act, type ComponentType, type ButtonHTMLAttributes } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { Host, ProviderConfigFormProps } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import setup from './page'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({
    variant,
    size,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: string
    size?: string
  }) => <button {...props} />,
}))

const prefix = 'provider::openai-codex::'
const signedOut = { status: 'signed_out', source: null, account_id: null, login: null }
const authenticated = { status: 'authenticated', source: 'managed', account_id: 'account-1', login: null }
// Synthetic userinfo exercises rejection of credential-bearing links.
const verificationLinkWithUserInfo = new URL('https://example.com/')
verificationLinkWithUserInfo.username = 'user'
verificationLinkWithUserInfo.password = 'password'
const attempt = {
  login_id: 'login-1',
  verification_uri: 'https://auth.openai.com/codex/device',
  user_code: 'ABCD-EFGH',
  expires_at: 1_800_000_060,
  interval: 5,
}
const rpc = vi.fn<(method: string, payload?: Record<string, unknown>) => Promise<unknown>>()
let root: Root
let container: HTMLDivElement

async function mount(modelCount = 0) {
  let Form: ComponentType<ProviderConfigFormProps> | undefined
  const host = {
    iii: { trigger: rpc },
    providerConfigForms: {
      register: vi.fn((id, form) => {
        expect(id).toBe('openai-codex')
        Form = form
      }),
    },
  } as unknown as Host
  setup(host)
  if (!Form) throw new Error('Provider form was not registered')
  const RegisteredForm = Form
  await act(async () =>
    root.render(
      <RegisteredForm providerId="openai-codex" schema={null} value={{}} onChange={() => {}} modelCount={modelCount} />,
    ),
  )
}

function button(name: string) {
  const result = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === name)
  if (!result) throw new Error(`Missing button: ${name}\n${container.textContent}`)
  return result
}
async function click(name: string) {
  await act(async () => button(name).click())
}
async function advance(ms: number) {
  await act(async () => vi.advanceTimersByTimeAsync(ms))
}
function calls(method: string) {
  return rpc.mock.calls.filter(([name]) => name === prefix + method)
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(1_800_000_000_000)
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  rpc.mockReset().mockImplementation(async (method) => {
    if (method === `${prefix}auth::status`) return signedOut
    if (method === `${prefix}login::start`) return attempt
    if (method === `${prefix}login::poll`) return { status: 'pending' }
    if (method === `${prefix}refresh_models`) return { count: 3 }
    return { ok: true }
  })
})
afterEach(async () => {
  await act(async () => root.unmount())
  expect(vi.getTimerCount()).toBe(0)
  container.remove()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('Codex console login', () => {
  it('reads authentication independently of the model count', async () => {
    await mount(20)
    expect(calls('auth::status')).toEqual([[`${prefix}auth::status`, {}]])
    expect(container.textContent).toContain('Signed out')
    expect(button('Sign in with ChatGPT').disabled).toBe(false)
  })

  it('starts login with a safe browser link, code and seconds-based expiry', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    expect(calls('login::start')).toEqual([[`${prefix}login::start`, {}]])
    const link = container.querySelector('a')!
    expect(link.href).toBe(attempt.verification_uri)
    expect(link.target).toBe('_blank')
    expect(link.rel).toContain('noopener')
    expect(link.rel).toContain('noreferrer')
    expect(container.textContent).toContain(attempt.user_code)
    expect(container.textContent).toContain('1:00')
    await advance(4000)
    expect(calls('login::poll')).toHaveLength(0)
    await advance(1000)
    expect(calls('login::poll')).toEqual([[`${prefix}login::poll`, { login_id: attempt.login_id }]])
    expect(container.textContent).toContain('0:55')
  })

  it('resumes a backend login and stops timers without canceling it on unmount', async () => {
    rpc.mockResolvedValueOnce({ ...signedOut, login: attempt })
    await mount()
    expect(container.textContent).toContain(attempt.user_code)
    await advance(5000)
    await act(async () => root.unmount())
    root = createRoot(container)
    await advance(60_000)
    expect(calls('login::poll')).toHaveLength(1)
    expect(calls('login::cancel')).toHaveLength(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('retries transient polling errors and keeps successful auth when catalog refresh fails', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    rpc.mockRejectedValueOnce(new Error('transport secret must not be rendered'))
    await advance(5000)
    expect(container.textContent).toContain('Retrying automatically')
    expect(container.textContent).not.toContain('transport secret')
    rpc
      .mockResolvedValueOnce({ status: 'ok' })
      .mockResolvedValueOnce(authenticated)
      .mockRejectedValueOnce(new Error('catalog unavailable'))
    await advance(5000)
    expect(container.textContent).toContain('Signed in')
    expect(container.textContent).toContain('account-1')
    expect(container.textContent).toContain('Could not refresh models')
    expect(calls('auth::status')).toHaveLength(2)
    expect(calls('refresh_models')).toHaveLength(1)
    await click('Refresh models')
    expect(container.textContent).not.toContain('Could not refresh models')
  })

  it('recovers an initial status error without assuming signed out', async () => {
    rpc.mockRejectedValueOnce(new Error('offline'))
    await mount(10)
    expect(container.textContent).toContain('Could not check account status')
    expect(container.textContent).not.toContain('Signed out')
    rpc.mockResolvedValueOnce(authenticated)
    await click('Retry status')
    expect(container.textContent).toContain('Signed in')
  })

  it('expires the code, stops polling and offers a new attempt', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    await advance(60_000)
    expect(container.textContent).toContain('Sign-in code expired')
    const count = calls('login::poll').length
    await advance(5000)
    expect(calls('login::poll')).toHaveLength(count)
    await click('New attempt')
    expect(calls('login::start')).toHaveLength(2)
  })

  it('cancels explicitly and ignores a pending poll response', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    const poll = deferred<unknown>()
    rpc.mockReturnValueOnce(poll.promise)
    await advance(5000)
    await click('Cancel sign-in')
    expect(calls('login::cancel')).toEqual([[`${prefix}login::cancel`, { login_id: attempt.login_id }]])
    await act(async () => poll.resolve({ status: 'ok' }))
    expect(calls('refresh_models')).toHaveLength(0)
    expect(container.textContent).not.toContain(attempt.user_code)
    expect(button('New attempt').disabled).toBe(false)
  })

  it('copies the code with recoverable clipboard feedback', async () => {
    const writeText = vi.fn().mockRejectedValueOnce(new Error('denied')).mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    await mount()
    await click('Sign in with ChatGPT')
    await click('Copy code')
    expect(container.textContent).toContain('Select and copy the code manually')
    await click('Copy code')
    expect(writeText).toHaveBeenLastCalledWith(attempt.user_code)
    expect(container.textContent).toContain('Code copied')
    await advance(3000)
    expect(container.textContent).not.toContain('Code copied')
  })

  it.each(['local', 'vault'])('keeps the %s session while switching and supports explicit logout', async (source) => {
    rpc.mockResolvedValueOnce({ ...authenticated, source })
    await mount(0)
    expect(container.textContent).toContain('Signed in')
    await click('Switch account')
    expect(container.textContent).toContain('account-1')
    expect(container.textContent).toContain('stays signed in')
    expect(calls('auth::logout')).toHaveLength(0)
    rpc.mockResolvedValueOnce({ ok: true }).mockResolvedValueOnce({ ...authenticated, source })
    await click('Cancel sign-in')
    await click('Log out')
    expect(calls('auth::logout')).toEqual([[`${prefix}auth::logout`, {}]])
    expect(container.textContent).toContain('Signed out')
  })

  it('offers reconnect for expired auth and refreshes idle status automatically', async () => {
    rpc.mockResolvedValueOnce({ ...authenticated, status: 'expired' })
    await mount()
    expect(button('Reconnect').disabled).toBe(false)
    rpc.mockResolvedValueOnce(authenticated)
    await advance(30_000)
    expect(container.textContent).toContain('Signed in')
  })

  it('can retry status after completed login when status storage is temporarily unavailable', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    rpc.mockResolvedValueOnce({ status: 'ok' }).mockRejectedValueOnce(new Error('storage unavailable'))
    await advance(5000)
    expect(container.textContent).toContain('Sign-in completed')
    expect(button('Retry status').disabled).toBe(false)
    rpc.mockResolvedValueOnce(authenticated)
    await click('Retry status')
    expect(container.textContent).toContain('Signed in')
  })

  it('recovers a failed start and blocks duplicate starts while the request is pending', async () => {
    await mount()
    rpc.mockRejectedValueOnce(new Error('private details'))
    await click('Sign in with ChatGPT')
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not start sign-in')
    expect(container.textContent).not.toContain('private details')
    const start = deferred<unknown>()
    rpc.mockReturnValueOnce(start.promise)
    await click('Sign in with ChatGPT')
    expect(button('Starting sign-in…').disabled).toBe(true)
    await click('Starting sign-in…')
    expect(calls('login::start')).toHaveLength(2)
    await act(async () => start.resolve(attempt))
    expect(container.textContent).toContain(attempt.user_code)
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('explains the device-login prerequisite before signing in', async () => {
    await mount()
    expect(container.textContent).toContain(
      'Enable device code login in ChatGPT security settings or ask your workspace administrator.',
    )
    expect(button('Sign in with ChatGPT').disabled).toBe(false)
  })

  it('offers actionable recovery and a working retry when device login is disabled', async () => {
    await mount()
    const guidance = 'Enable device code login in ChatGPT security settings or ask your workspace administrator.'
    rpc.mockRejectedValueOnce(Object.assign(new Error(guidance), { code: 'device_login_disabled' }))
    await click('Sign in with ChatGPT')
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not start sign-in')
    expect(container.textContent).toContain(guidance)
    expect(button('Sign in with ChatGPT').disabled).toBe(false)

    await click('Sign in with ChatGPT')
    expect(calls('login::start')).toHaveLength(2)
    expect(container.textContent).toContain(attempt.user_code)
    expect(container.querySelector('[role="alert"]')).toBeNull()
    await advance(5000)
    expect(calls('login::poll')).toHaveLength(1)
  })

  it.each([false, 'throw'])('recovers a failed cancel (%s) and keeps polling', async (failure) => {
    await mount()
    await click('Sign in with ChatGPT')
    if (failure === 'throw') rpc.mockRejectedValueOnce(new Error('offline'))
    else rpc.mockResolvedValueOnce({ ok: false })
    await click('Cancel sign-in')
    expect(container.textContent).toContain('Could not cancel sign-in')
    expect(container.textContent).toContain(attempt.user_code)
    await advance(5000)
    expect(calls('login::poll')).toHaveLength(1)
    await click('Cancel sign-in')
    expect(container.textContent).not.toContain(attempt.user_code)
  })

  it.each([false, 'throw'])('preserves authentication on failed logout (%s)', async (failure) => {
    rpc.mockResolvedValueOnce(authenticated)
    await mount()
    if (failure === 'throw') rpc.mockRejectedValueOnce(new Error('storage failed'))
    else rpc.mockResolvedValueOnce({ ok: false })
    await click('Log out')
    expect(container.textContent).toContain('Could not log out')
    expect(container.textContent).toContain('Signed in')
    await click('Log out')
    expect(container.textContent).toContain('Signed out')
  })

  it.each(['expired', 'canceled', 'error'])(
    'handles terminal %s polls without hiding an existing session',
    async (status) => {
      rpc.mockResolvedValueOnce(authenticated)
      await mount()
      await click('Switch account')
      rpc.mockResolvedValueOnce({
        status,
        error: { code: 'authorization_failed', message: 'sensitive server details' },
      })
      await advance(5000)
      expect(container.textContent).toContain('Signed in')
      expect(container.textContent).toContain('account-1')
      expect(container.textContent).not.toContain('sensitive server details')
      expect(container.textContent).not.toContain(attempt.user_code)
      expect(button('New attempt').disabled).toBe(false)
      await advance(5000)
      expect(calls('login::poll')).toHaveLength(1)
    },
  )

  it('never overlaps slow polls or accepts responses from an expired attempt', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    const poll = deferred<unknown>()
    rpc.mockReturnValueOnce(poll.promise)
    await advance(20_000)
    expect(calls('login::poll')).toHaveLength(1)
    await advance(40_000)
    expect(container.textContent).toContain('Sign-in code expired')
    await act(async () => poll.resolve({ status: 'ok' }))
    expect(calls('refresh_models')).toHaveLength(0)
  })

  it('ignores a stale status response after logout', async () => {
    rpc.mockResolvedValueOnce(authenticated)
    await mount()
    const status = deferred<unknown>()
    rpc.mockReturnValueOnce(status.promise)
    await advance(30_000)
    await click('Log out')
    await act(async () => status.resolve(authenticated))
    expect(container.textContent).toContain('Signed out')
    expect(container.textContent).not.toContain('account-1')
  })

  it.each(['javascript:alert(1)', 'http://example.com/login', verificationLinkWithUserInfo.href, 'invalid'])(
    'rejects unsafe verification link %s',
    async (verification_uri) => {
      rpc.mockResolvedValueOnce({ ...signedOut, login: { ...attempt, verification_uri } })
      await mount()
      expect(container.querySelector('a')).toBeNull()
      expect(container.textContent).toContain('sign-in link is unavailable')
      expect(button('Cancel sign-in').disabled).toBe(false)
    },
  )

  it('refreshes status when the window regains focus', async () => {
    await mount()
    rpc.mockResolvedValueOnce(authenticated)
    await act(async () => window.dispatchEvent(new Event('focus')))
    expect(container.textContent).toContain('Signed in')
  })

  it('replaces the account only after approval and refreshes the catalog', async () => {
    rpc.mockResolvedValueOnce({ ...authenticated, source: 'local' })
    await mount()
    await click('Switch account')
    expect(container.textContent).toContain('account-1')
    rpc.mockResolvedValueOnce({ status: 'ok' }).mockResolvedValueOnce({ ...authenticated, account_id: 'account-2' })
    await advance(5000)
    expect(container.textContent).toContain('account-2')
    expect(container.textContent).not.toContain('account-1')
    expect(calls('auth::logout')).toHaveLength(0)
    expect(calls('refresh_models')).toHaveLength(1)
  })

  it('does not start follow-up work after unmount during account confirmation', async () => {
    await mount()
    await click('Sign in with ChatGPT')
    const status = deferred<unknown>()
    rpc.mockResolvedValueOnce({ status: 'ok' }).mockReturnValueOnce(status.promise)
    await advance(5000)
    await act(async () => root.unmount())
    root = createRoot(container)
    await act(async () => status.resolve(authenticated))
    expect(calls('refresh_models')).toHaveLength(0)
  })

  it('does not resurrect an expired backend login or poll it', async () => {
    const expiredLogin = { ...signedOut, login: { ...attempt, expires_at: 1_799_999_999 } }
    rpc.mockResolvedValueOnce(expiredLogin)
    await mount()
    expect(container.textContent).toContain('Sign-in code expired')
    rpc.mockResolvedValueOnce(expiredLogin)
    await advance(30_000)
    expect(calls('login::poll')).toHaveLength(0)
    expect(container.textContent).not.toContain(attempt.user_code)
  })

  it('preserves CLI fallback help and the worker-replaced compatibility placeholder', async () => {
    await mount()
    expect(container.textContent).toContain('codex login')
    expect(container.textContent).toContain('__CODEX_COMPAT_VERSION__')
  })
})
