import {
  Check,
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
import { cn } from '@/lib/utils'

type Phase = 'idle' | 'fetching' | 'code' | 'waiting' | 'connected' | 'stopped'

const MESSAGES: Partial<Record<DevicePollStatus, string>> = {
  pending: 'Waiting for you to enter the code on GitHub.',
  slow_down: 'Waiting for you to enter the code on GitHub.',
  expired: 'The code expired. Retry for a new one.',
  denied: 'GitHub sign-in was denied. Retry to start again.',
}

/** How long a copy shows as done. */
const COPIED_MS = 2_000

/** `https://github.com/login/device` as `github.com/login/device`. */
function pageLabel(uri: string): string {
  try {
    const url = new URL(uri)
    return `${url.host}${url.pathname.replace(/\/$/, '')}`
  } catch {
    return uri
  }
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
 * sign-in 8 s later and then every 5 s, five tries; a return to this tab or
 * Retry starts a new round of five, 5 s apart. A spinner turns while checks
 * are due, and the status counts the tries that failed. Only an expired or
 * denied code gets a new one from Retry. The reload icon beside the code
 * gets a new code at any time.
 */
export function DeviceSignIn({
  provider,
  installed = true,
  heading = true,
  onConnected,
}: {
  provider: DeviceProvider
  /** The provider's worker runs, so a code can be fetched without asking. */
  installed?: boolean
  /**
   * Its own title and plan line. Off where the provider's row above already
   * says both (the setup wizard).
   */
  heading?: boolean
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
  /** The code was just copied: the copy control shows it for a moment. */
  const [copied, setCopied] = useState(false)
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
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

  useEffect(
    () => () => {
      if (copiedTimer.current) clearTimeout(copiedTimer.current)
    },
    [],
  )

  const copy = async (current: DeviceCode) => {
    const ok = await copyTextToClipboard(current.user_code)
    if (!ok) return
    if (copiedTimer.current) clearTimeout(copiedTimer.current)
    setCopied(true)
    copiedTimer.current = setTimeout(() => setCopied(false), COPIED_MS)
  }

  const authenticate = (current: DeviceCode) => {
    void copy(current)
    window.open(current.verification_uri, '_blank', 'noopener')
    watch(current)
  }

  /**
   * Check again at once with the same code. A code GitHub expired or
   * refused cannot be checked again: that one is replaced, and Authenticate
   * opens the page with the new one.
   */
  const retry = () => {
    if (phase === 'waiting') checkNow()
    else void fetchCode()
  }

  const started = phase === 'waiting' || phase === 'stopped'
  const showCode =
    code !== null && phase !== 'connected' && phase !== 'fetching'
  const intro =
    phase === 'idle'
      ? `Uses ${provider.plan}, no API key. Get code adds ${provider.worker} and shows a one-time code for GitHub.`
      : phase === 'connected'
        ? `Signed in — uses ${provider.plan}. Its models appear in a few seconds.`
        : heading
          ? `Uses ${provider.plan}, no API key.`
          : null
  return (
    <section
      aria-label={`${provider.title} sign-in`}
      className="flex flex-col gap-3 rounded-lg border border-edge bg-surface p-3"
    >
      {heading || phase === 'connected' ? (
        <header className="flex items-center justify-between gap-2">
          <span className="font-sans text-[13px] font-medium text-ink">
            Sign in with GitHub
          </span>
          {phase === 'connected' ? <Chip tone="success">Signed in</Chip> : null}
        </header>
      ) : null}
      {intro ? (
        <p className="font-sans text-[13px] leading-relaxed text-ink-faint">
          {intro}
        </p>
      ) : null}
      {showCode && code ? (
        <div className="flex flex-col gap-1.5">
          <span className="font-sans text-[12px] font-medium text-ink-faint">
            Your one-time code
          </span>
          <button
            type="button"
            onClick={() => void copy(code)}
            aria-label={
              copied
                ? `Code ${code.user_code} copied`
                : `Copy code ${code.user_code}`
            }
            className={cn(
              'group flex h-12 items-center justify-between gap-3 rounded-md border bg-panel-raised pr-2 pl-4 text-left transition-[border-color,background-color] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
              copied
                ? 'border-ok/50'
                : 'border-edge hover:border-neutral-400 dark:hover:border-neutral-600',
            )}
          >
            <span className="select-all font-mono text-[18px] font-semibold tracking-[0.18em] text-ink">
              {code.user_code}
            </span>
            <span
              className={cn(
                'flex h-8 items-center gap-1.5 rounded-sm px-2.5 font-sans text-[13px] font-medium transition-colors duration-150',
                copied
                  ? 'text-ok'
                  : 'text-ink-faint group-hover:bg-surface-hover group-hover:text-ink',
              )}
              aria-hidden
            >
              {copied ? (
                <Check className="size-4 shrink-0" />
              ) : (
                <Copy className="size-4 shrink-0" />
              )}
              {copied ? 'Copied' : 'Copy'}
            </span>
          </button>
          <span className="font-sans text-[12px] text-ink-faint">
            Paste it at{' '}
            <a
              href={code.verification_uri}
              target="_blank"
              rel="noopener noreferrer"
              className="font-medium text-ink underline decoration-neutral-400 underline-offset-2 hover:decoration-ink dark:decoration-neutral-600"
            >
              {pageLabel(code.verification_uri)}
            </a>
            .
          </span>
          <span className="sr-only" role="status" aria-live="polite">
            {copied ? 'Code copied to the clipboard.' : ''}
          </span>
        </div>
      ) : null}
      {phase === 'fetching' || (message && phase !== 'connected') ? (
        <p
          className="flex items-center gap-1.5 font-sans text-[13px] text-ink-faint"
          role="status"
          aria-live="polite"
        >
          {phase === 'fetching' || (phase === 'waiting' && checking) ? (
            <LoaderCircle className="iii-ui-spin size-4 shrink-0" aria-hidden />
          ) : null}
          <span>
            {message ?? (phase === 'fetching' ? 'Getting a code…' : null)}
            {phase === 'waiting' && failed > 0
              ? ` ${failed}/${POLL_TRIES} retries failed.`
              : null}
            {phase === 'waiting' && !checking
              ? ' Come back to this tab or click Retry to check again.'
              : null}
          </span>
        </p>
      ) : null}
      {phase !== 'connected' && phase !== 'fetching' ? (
        <span className="flex flex-wrap items-center gap-2">
          {phase === 'idle' ? (
            <Button size="sm" onClick={() => void fetchCode()}>
              Get code
            </Button>
          ) : null}
          {phase === 'code' && code ? (
            <Button size="sm" onClick={() => authenticate(code)}>
              Copy code &amp; open GitHub
              <ExternalLink aria-hidden />
            </Button>
          ) : null}
          {started && code ? (
            <Button variant="pill" size="sm" asChild>
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
          {started ? (
            <Button variant="pill" size="sm" onClick={retry}>
              <RotateCw aria-hidden />
              Retry
            </Button>
          ) : null}
          {showCode ? (
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              onClick={() => void fetchCode()}
            >
              <RefreshCw aria-hidden />
              New code
            </Button>
          ) : null}
        </span>
      ) : null}
    </section>
  )
}
