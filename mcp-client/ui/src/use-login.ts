import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { Api } from './api'
import { signInView } from './presentation'
import type { LoginStart, Server } from './types'

/** One server's sign-in. The outcome arrives through `servers::list`: the `server` prop changes on every `login`
 * and `state` event, so nothing polls. The start reply, which alone carries the authorize URL, is kept in this
 * state and nowhere else, so a reload resumes without it. */
export function useLogin(api: Api, server: Server) {
  const [held, setHeld] = useState<LoginStart | null>(null)
  const [busy, setBusy] = useState<'start' | 'finish' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [now, setNow] = useState(Date.now)
  // Bumped by every action and by a change of server: a reply that returns after either is dropped.
  const revision = useRef(0)
  const { name, login, state } = server
  const pending = login?.pending === true

  useEffect(() => {
    revision.current += 1
    setHeld(null)
    setBusy(null)
    setError(null)
  }, [name])
  // A server that moved on (signed in from another tab, switched to another auth, turned off) leaves no old
  // failure behind. A failed start or finish leaves the state as it was, so a real failure stays.
  useEffect(() => {
    setHeld(null)
    setError(null)
  }, [state])
  // The worker ended the attempt (signed in, cancelled elsewhere, or taken by a finish): its link is spent.
  useEffect(() => {
    if (login === null) setHeld(null)
  }, [login])
  useEffect(() => {
    if (!pending) return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [pending])

  const run = useCallback(
    async <T>(kind: 'start' | 'finish' | null, call: () => Promise<T>, done?: (reply: T) => void) => {
      const mine = ++revision.current
      setBusy(kind)
      setError(null)
      try {
        const reply = await call()
        if (mine === revision.current) done?.(reply)
      } catch (err) {
        if (mine !== revision.current) return
        setHeld(null)
        setError(errorMessage(err))
      } finally {
        if (mine === revision.current) setBusy(null)
      }
    },
    [],
  )
  const start = useCallback(() => {
    setHeld(null)
    void run('start', () => api.loginStart(name), setHeld)
  }, [api, name, run])
  const finish = useCallback(
    (callbackUrl: string) => void run('finish', () => api.loginComplete(callbackUrl.trim())),
    [api, run],
  )
  const cancel = useCallback(() => {
    setHeld(null)
    void run(null, () => api.loginCancel(name))
  }, [api, name, run])

  return {
    ...signInView({ login, held, busy, error, now }),
    redirectUri: held?.redirect_uri ?? null,
    expiresAt: login?.expires_at ?? held?.expires_at ?? null,
    error,
    start,
    finish,
    cancel,
  }
}
