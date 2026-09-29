import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'

export interface LoginStartResponse {
  login_id: string
  verification_uri: string
  user_code: string
  expires_at: number
  interval: number
}

interface AuthStatus {
  status: 'signed_out' | 'authenticated' | 'expired'
  source: 'managed' | 'vault' | 'local' | null
  account_id: string | null
  login: LoginStartResponse | null
}

interface PollResponse {
  status: 'pending' | 'ok' | 'expired' | 'canceled' | 'error'
  error?: { code: string; message: string }
}

interface View {
  auth: AuthStatus | null
  login: LoginStartResponse | null
  busy: 'start' | 'cancel' | 'logout' | null
  checking: boolean
  refreshing: boolean
  statusError: string | null
  actionError: string | null
  pollError: string | null
  catalogMessage: string | null
  notice: string | null
  retry: boolean
  confirming: boolean
}

const initialView: View = {
  auth: null,
  login: null,
  busy: null,
  checking: true,
  refreshing: false,
  statusError: null,
  actionError: null,
  pollError: null,
  catalogMessage: null,
  notice: null,
  retry: false,
  confirming: false,
}
const prefix = 'provider::openai-codex::'

/** Only this mounted view's requests/timers are owned here; login belongs to the backend. */
export function useCodexAuth(iii: Host['iii']) {
  const [view, setView] = useState(initialView)
  const actions = useRef<{
    start(): Promise<void>
    cancel(): Promise<void>
    logout(): Promise<void>
    status(): Promise<void>
    refresh(): Promise<void>
  } | null>(null)

  useEffect(() => {
    let state = { ...initialView }
    let disposed = false
    let revision = 0
    let statusRequest: object | null = null
    let pollTimer: ReturnType<typeof setTimeout> | undefined
    let expiryTimer: ReturnType<typeof setTimeout> | undefined
    // Do not resurrect an attempt that this view has already seen finish.
    let finishedLogin: string | null = null

    function update(patch: Partial<View>) {
      if (disposed) return
      state = { ...state, ...patch }
      setView(state)
    }
    const current = (version: number) => !disposed && revision === version
    const rpc = <T>(method: string, payload: Record<string, unknown> = {}) => iii.trigger<T>(prefix + method, payload)

    function stopLoginTimers() {
      clearTimeout(pollTimer)
      clearTimeout(expiryTimer)
    }

    function finishLogin(notice: string) {
      finishedLogin = state.login?.login_id ?? null
      stopLoginTimers()
      revision++
      update({ login: null, pollError: null, actionError: null, notice, retry: true })
    }

    function scheduleLogin() {
      stopLoginTimers()
      const login = state.login
      if (!login || state.busy || disposed) return
      const remaining = login.expires_at * 1000 - Date.now()
      if (remaining <= 0) {
        finishLogin('Sign-in code expired. Start a new attempt.')
        return
      }
      const version = revision
      expiryTimer = setTimeout(() => {
        if (current(version)) finishLogin('Sign-in code expired. Start a new attempt.')
      }, remaining)
      // Polls are sequential, including retries after transport/storage errors.
      pollTimer = setTimeout(() => void poll(login, version), Math.max(1, login.interval) * 1000)
    }

    function resume(login: LoginStartResponse) {
      if (login.login_id === finishedLogin) return
      update({ login, notice: null, retry: false, pollError: null })
      scheduleLogin()
    }

    async function status() {
      if (disposed || statusRequest || state.busy || state.login) return
      const request = {}
      statusRequest = request
      const version = revision
      update({ checking: true })
      try {
        const auth = await rpc<AuthStatus>('auth::status')
        if (!current(version)) return
        update({ auth, statusError: null, actionError: null, confirming: false })
        if (auth.login) resume(auth.login)
      } catch {
        if (current(version))
          update({ statusError: 'Could not check account status. Retry or wait for automatic recovery.' })
      } finally {
        if (statusRequest === request) {
          statusRequest = null
          update({ checking: false })
        }
      }
    }

    async function refresh() {
      if (disposed || state.refreshing) return
      const version = revision
      update({ refreshing: true, catalogMessage: null })
      try {
        const result = await rpc<{ count?: number }>('refresh_models')
        if (current(version)) {
          const count = result?.count ?? 0
          update({ catalogMessage: `${count} model${count === 1 ? '' : 's'} available.` })
        }
      } catch {
        if (current(version))
          update({
            catalogMessage: 'Could not refresh models. Your sign-in is unchanged. Try refreshing models again.',
          })
      } finally {
        if (current(version)) update({ refreshing: false })
      }
    }

    async function poll(login: LoginStartResponse, version: number) {
      try {
        const result = await rpc<PollResponse>('login::poll', { login_id: login.login_id })
        if (!current(version)) return
        if (result.status === 'pending') {
          update({ pollError: null })
          scheduleLogin()
        } else if (result.status === 'ok') {
          finishLogin('Sign-in completed.')
          update({ confirming: true, retry: false })
          // Catalog availability is independent of authentication success.
          await status()
          if (!disposed) void refresh()
        } else {
          const notice =
            result.status === 'expired'
              ? 'Sign-in code expired. Start a new attempt.'
              : result.status === 'canceled'
                ? 'Sign-in canceled. You can start a new attempt.'
                : 'Sign-in could not be completed. Start a new attempt.'
          finishLogin(notice)
        }
      } catch {
        if (!current(version)) return
        update({ pollError: 'Could not check sign-in. Retrying automatically.' })
        scheduleLogin()
      }
    }

    function begin(busy: View['busy']) {
      revision++
      stopLoginTimers()
      // An earlier status response must not overwrite a user action.
      statusRequest = null
      update({ busy, checking: false, actionError: null, notice: null, refreshing: false, catalogMessage: null })
      return revision
    }

    async function start() {
      if (disposed || state.busy || state.login || state.confirming || !state.auth) return
      const version = begin('start')
      try {
        const login = await rpc<LoginStartResponse>('login::start')
        if (current(version)) {
          finishedLogin = null
          update({ login, retry: false, pollError: null })
        }
      } catch {
        if (current(version)) update({ actionError: 'Could not start sign-in. Try again.' })
      } finally {
        if (current(version)) {
          update({ busy: null })
          scheduleLogin()
        }
      }
    }

    async function cancel() {
      if (disposed || state.busy || !state.login) return
      const login = state.login
      const version = begin('cancel')
      try {
        const result = await rpc<{ ok: boolean }>('login::cancel', { login_id: login.login_id })
        if (!result.ok) throw new Error('Cancellation was not confirmed')
        if (!current(version)) return
        finishedLogin = login.login_id
        update({ login: null, pollError: null, notice: 'Sign-in canceled.', retry: true })
      } catch {
        if (current(version))
          update({ actionError: 'Could not cancel sign-in. It may still be active. Try canceling again.' })
      } finally {
        if (current(version)) {
          update({ busy: null })
          if (state.login) scheduleLogin()
          else void status()
        }
      }
    }

    async function logout() {
      if (disposed || state.busy || state.login || state.confirming) return
      const version = begin('logout')
      try {
        const result = await rpc<{ ok: boolean }>('auth::logout')
        if (!result.ok) throw new Error('Logout was not confirmed')
        if (current(version))
          update({
            auth: { status: 'signed_out', source: null, account_id: null, login: null },
            retry: false,
            statusError: null,
            notice: 'Logged out.',
          })
      } catch {
        if (current(version)) update({ actionError: 'Could not log out. Your session may still be active. Try again.' })
      } finally {
        if (current(version)) update({ busy: null })
      }
    }

    actions.current = { start, cancel, logout, status, refresh }
    void status()
    const statusTimer = setInterval(() => void status(), 30_000)
    const onFocus = () => void status()
    window.addEventListener('focus', onFocus)
    return () => {
      disposed = true
      revision++
      stopLoginTimers()
      clearInterval(statusTimer)
      window.removeEventListener('focus', onFocus)
      actions.current = null
    }
  }, [iii])

  return {
    ...view,
    start: () => actions.current?.start(),
    cancel: () => actions.current?.cancel(),
    logout: () => actions.current?.logout(),
    checkStatus: () => actions.current?.status(),
    refreshModels: () => actions.current?.refresh(),
  }
}
