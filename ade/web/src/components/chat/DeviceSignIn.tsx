import {
  Copy,
  ExternalLink,
  LoaderCircle,
  RefreshCw,
  RotateCw,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Chip } from '@/components/ui/Chip'
import { copyTextToClipboard } from '@/lib/clipboard'
import { readableError } from '@/lib/onboarding/api'
import type { DeviceProvider } from '@/lib/onboarding/catalog'
import {
  createLoginPoller,
  type DeviceCode,
  type DevicePollStatus,
  type LoginPoller,
  POLL_TRIES,
  pollDeviceLogin,
  startDeviceLogin,
} from '@/lib/onboarding/device-login'

type Phase = 'idle' | 'fetching' | 'code' | 'waiting' | 'connected' | 'stopped'

const MESSAGES: Partial<Record<DevicePollStatus, string>> = {
  pending: 'Waiting for you to enter the code on GitHub.',
  slow_down: 'Waiting for you to enter the code on GitHub.',
  expired: 'The code expired. Restart Authentication for a new one.',
  denied: 'GitHub sign-in was denied. Restart Authentication to try again.',
}

/** A code GitHub has not used yet is replaced this long before it expires. */
const RENEW_MARGIN_MS = 30_000

/**
 * The code each provider handed out, kept across remounts: a wizard that
 * re-renders must not swap the code the person is typing for a new one.
 */
const issued = new Map<string, { code: DeviceCode; expiresAt: number }>()

function liveCode(providerId: string): DeviceCode | null {
  const entry = issued.get(providerId)
  return entry && entry.expiresAt - Date.now() > RENEW_MARGIN_MS
    ? entry.code
    : null
}

/**
 * The device-flow sign-in (GitHub Copilot). The one-time code shows first —
 * at once when the worker runs, after Get code (which adds the worker) when
 * it does not — so the person has it before GitHub's page opens. The code is
 * kept until it expires, and an unused one is replaced before that.
 * Authenticate copies it, opens the page, and the ADE checks for the
 * sign-in 8 s later and then every 5 s, five tries; a return to this tab
 * starts a new round of five, 5 s apart. A spinner turns while checks are
 * due, and the status counts the tries that failed. Restart Authentication
 * starts over: a new code, copied, and GitHub's page opened again. The
 * reload icon beside the code gets a new code at any time.
 */
export function DeviceSignIn({
  provider,
  installed = true,
  onConnected,
}: {
  provider: DeviceProvider
  /** The provider's worker runs, so a code can be fetched without asking. */
  installed?: boolean
  /** The sign-in landed; the provider's catalog fills next. */
  onConnected?: () => void
}) {
  const [phase, setPhase] = useState<Phase>(installed ? 'fetching' : 'idle')
  const [code, setCode] = useState<DeviceCode | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  /** Checks are scheduled (the spinner turns). */
  const [checking, setChecking] = useState(false)
  /** Checks in the current round that did not return the token. */
  const [failed, setFailed] = useState(0)
  const poller = useRef<LoginPoller | null>(null)
  const generation = useRef(0)
  const onConnectedRef = useRef(onConnected)
  onConnectedRef.current = onConnected

  /** Fetch a code; resolves to it, or null when a newer fetch replaced it. */
  const fetchCode = useCallback(async (): Promise<DeviceCode | null> => {
    poller.current?.stop()
    const mine = ++generation.current
    setPhase('fetching')
    setMessage(null)
    try {
      const next = await startDeviceLogin(provider, ({ note }) => {
        if (note && generation.current === mine) setMessage(note)
      })
      if (generation.current !== mine) return null
      issued.set(provider.providerId, {
        code: next,
        expiresAt: Date.now() + (next.expires_in ?? 900) * 1_000,
      })
      setCode(next)
      setMessage(null)
      setPhase('code')
      return next
    } catch (error) {
      if (generation.current !== mine) return null
      setMessage(readableError(error))
      setPhase('stopped')
      return null
    }
  }, [provider])

  // A running worker hands out a code without being asked; one already
  // handed out (before a remount) is shown again instead.
  useEffect(() => {
    const kept = liveCode(provider.providerId)
    if (kept) {
      setCode(kept)
      setPhase('code')
    } else if (installed) {
      void fetchCode()
    }
    return () => {
      generation.current++
      poller.current?.stop()
    }
  }, [installed, fetchCode, provider.providerId])

  // An unused code is replaced before GitHub expires it.
  useEffect(() => {
    if (phase !== 'code' || !code?.expires_in) return
    const timer = setTimeout(
      () => void fetchCode(),
      Math.max(code.expires_in * 1_000 - RENEW_MARGIN_MS, 1_000),
    )
    return () => clearTimeout(timer)
  }, [phase, code, fetchCode])

  const checkNow = useCallback(() => {
    setChecking(true)
    setFailed(0)
    poller.current?.restart()
  }, [])

  // Back on this tab: switching tabs does not always focus the window, so
  // listen for both.
  useEffect(() => {
    if (phase !== 'waiting') return
    const onReturn = () => {
      if (document.visibilityState === 'visible') checkNow()
    }
    window.addEventListener('focus', onReturn)
    document.addEventListener('visibilitychange', onReturn)
    return () => {
      window.removeEventListener('focus', onReturn)
      document.removeEventListener('visibilitychange', onReturn)
    }
  }, [phase, checkNow])

  const watch = (current: DeviceCode) => {
    poller.current?.stop()
    setPhase('waiting')
    setMessage(MESSAGES.pending ?? null)
    setChecking(true)
    setFailed(0)
    const next = createLoginPoller({
      poll: () => pollDeviceLogin(provider, current.device_code),
      minIntervalMs: (current.interval ?? 5) * 1_000,
      onIdle: () => setChecking(false),
      onResult: (result, attempt) => {
        if (result !== 'ok') setFailed(attempt)
        if (result instanceof Error) {
          setMessage(readableError(result))
        } else if (result === 'ok') {
          issued.delete(provider.providerId)
          setChecking(false)
          setPhase('connected')
          setMessage(null)
          onConnectedRef.current?.()
        } else {
          setMessage(MESSAGES[result] ?? null)
          if (result === 'expired' || result === 'denied') {
            issued.delete(provider.providerId)
            setChecking(false)
            setPhase('stopped')
          }
        }
      },
    })
    poller.current = next
    next.begin()
  }

  const authenticate = (current: DeviceCode) => {
    void copyTextToClipboard(current.user_code)
    window.open(current.verification_uri, '_blank', 'noopener')
    watch(current)
  }

  /** Start over: a new code, copied, with GitHub's page opened for it. */
  const restart = async () => {
    // Opened in the click, before the await: a window opened after it is a
    // popup the browser blocks.
    const page = window.open('', '_blank')
    const next = await fetchCode()
    if (!next) {
      page?.close()
      return
    }
    void copyTextToClipboard(next.user_code)
    if (page) {
      page.opener = null
      page.location.href = next.verification_uri
    }
    watch(next)
  }

  const started = phase === 'waiting' || phase === 'stopped'
  return (
    <section
      aria-label={`${provider.title} sign-in`}
      className="flex flex-col gap-2 rounded-md bg-surface p-3"
    >
      <header className="flex items-center justify-between gap-2">
        <span className="font-sans text-[13px] font-medium text-ink">
          Sign in with GitHub
        </span>
        {phase === 'connected' ? <Chip tone="success">Signed in</Chip> : null}
      </header>
      <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
        {phase === 'connected'
          ? `Signed in — uses ${provider.plan}. Its models appear in a few seconds.`
          : phase === 'idle'
            ? `Uses ${provider.plan}, no API key. Get code adds ${provider.worker} and shows a one-time code for GitHub.`
            : `Uses ${provider.plan}, no API key. Enter this code on GitHub: Authenticate copies it and opens the page.`}
      </p>
      {code && phase !== 'connected' && phase !== 'fetching' ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="sr-only">One-time code:</span>
          <span className="rounded-sm bg-panel-raised px-2 py-1 font-mono text-[15px] font-medium tracking-[0.12em] text-ink">
            {code.user_code}
          </span>
          <Button
            variant="icon"
            size="icon"
            aria-label="New code"
            title="New code"
            onClick={() => void fetchCode()}
          >
            <RefreshCw aria-hidden />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void copyTextToClipboard(code.user_code)}
          >
            <Copy aria-hidden />
            Copy
          </Button>
          {started ? (
            <Button variant="ghost" size="sm" asChild>
              <a
                href={code.verification_uri}
                target="_blank"
                rel="noopener noreferrer"
              >
                <ExternalLink aria-hidden />
                Open GitHub
              </a>
            </Button>
          ) : null}
        </div>
      ) : null}
      {phase === 'fetching' || (message && phase !== 'connected') ? (
        <p
          className="flex items-center gap-1.5 font-sans text-[12px] text-ink-faint"
          role="status"
          aria-live="polite"
        >
          {phase === 'fetching' || (phase === 'waiting' && checking) ? (
            <LoaderCircle
              className="iii-ui-spin size-3.5 shrink-0"
              aria-hidden
            />
          ) : null}
          <span>
            {message ?? (phase === 'fetching' ? 'Getting a code…' : null)}
            {phase === 'waiting' && failed > 0
              ? ` ${failed}/${POLL_TRIES} retries failed.`
              : null}
            {phase === 'waiting' && !checking
              ? ' Come back to this tab to check again.'
              : null}
          </span>
        </p>
      ) : null}
      {phase !== 'connected' ? (
        <span className="flex items-center gap-2">
          {phase === 'idle' ? (
            <Button size="sm" onClick={() => void fetchCode()}>
              Get code
            </Button>
          ) : null}
          {phase === 'code' && code ? (
            <Button size="sm" onClick={() => authenticate(code)}>
              Authenticate
            </Button>
          ) : null}
          {started ? (
            <Button variant="pill" size="sm" onClick={() => void restart()}>
              <RotateCw aria-hidden />
              Restart Authentication
            </Button>
          ) : null}
        </span>
      ) : null}
    </section>
  )
}
