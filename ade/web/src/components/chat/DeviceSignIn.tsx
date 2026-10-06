import { Copy, ExternalLink, LoaderCircle, RotateCw } from 'lucide-react'
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
  pollDeviceLogin,
  startDeviceLogin,
} from '@/lib/onboarding/device-login'

type Phase = 'idle' | 'fetching' | 'code' | 'waiting' | 'connected' | 'stopped'

const MESSAGES: Partial<Record<DevicePollStatus, string>> = {
  pending: 'Waiting for you to enter the code on GitHub.',
  slow_down: 'Waiting for you to enter the code on GitHub.',
  expired: 'The code expired. Retry for a new one.',
  denied: 'GitHub sign-in was denied. Retry to start again.',
}

/** A code GitHub has not used yet is replaced this long before it expires. */
const RENEW_MARGIN_MS = 30_000

/**
 * The device-flow sign-in (GitHub Copilot). The one-time code shows first —
 * at once when the worker runs, after Get code (which adds the worker) when
 * it does not — so the person has it before GitHub's page opens. A code left
 * unused is replaced before it expires. Authenticate copies the code, opens
 * the page, and the ADE checks for the sign-in when this tab gets focus
 * again and 4, 8, 16 and 32 seconds after the click or the focus. Retry,
 * shown from Authenticate until the sign-in lands, gets a new code and opens
 * the page again.
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

  // A running worker hands out a code without being asked.
  useEffect(() => {
    if (installed) void fetchCode()
    return () => {
      generation.current++
      poller.current?.stop()
    }
  }, [installed, fetchCode])

  // An unused code is replaced before GitHub expires it.
  useEffect(() => {
    if (phase !== 'code' || !code?.expires_in) return
    const timer = setTimeout(
      () => void fetchCode(),
      Math.max(code.expires_in * 1_000 - RENEW_MARGIN_MS, 1_000),
    )
    return () => clearTimeout(timer)
  }, [phase, code, fetchCode])

  useEffect(() => {
    if (phase !== 'waiting') return
    const onFocus = () => poller.current?.focus()
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [phase])

  const watch = (current: DeviceCode) => {
    setPhase('waiting')
    setMessage(MESSAGES.pending ?? null)
    const next = createLoginPoller({
      poll: () => pollDeviceLogin(provider, current.device_code),
      onResult: (result) => {
        if (result instanceof Error) {
          setMessage(readableError(result))
        } else if (result === 'ok') {
          setPhase('connected')
          setMessage(null)
          onConnectedRef.current?.()
        } else {
          setMessage(MESSAGES[result] ?? null)
          if (result === 'expired' || result === 'denied') setPhase('stopped')
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

  const retry = async () => {
    // Opened inside the click, so the browser allows it; pointed at the
    // page once the new code is back.
    const tab = window.open('', '_blank')
    if (tab) tab.opener = null
    const next = await fetchCode()
    if (!next) {
      tab?.close()
      return
    }
    void copyTextToClipboard(next.user_code)
    if (tab) tab.location.href = next.verification_uri
    else window.open(next.verification_uri, '_blank', 'noopener')
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
          {phase === 'fetching' ? (
            <LoaderCircle className="iii-ui-spin size-3.5" aria-hidden />
          ) : null}
          {message ?? (phase === 'fetching' ? 'Getting a code…' : null)}
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
            <Button variant="pill" size="sm" onClick={() => void retry()}>
              <RotateCw aria-hidden />
              Retry
            </Button>
          ) : null}
        </span>
      ) : null}
    </section>
  )
}
