import { Copy, ExternalLink, LoaderCircle, RotateCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Chip } from '@/components/ui/Chip'
import { copyTextToClipboard } from '@/lib/clipboard'
import { readableError } from '@/lib/onboarding/api'
import type { DeviceProvider } from '@/lib/onboarding/catalog'
import {
  createLoginPoller,
  type DevicePollStatus,
  type LoginPoller,
  pollDeviceLogin,
  startDeviceLogin,
} from '@/lib/onboarding/device-login'

type Phase = 'idle' | 'starting' | 'waiting' | 'connected' | 'stopped'

const MESSAGES: Partial<Record<DevicePollStatus, string>> = {
  pending: 'Waiting for you to enter the code on GitHub.',
  slow_down: 'Waiting for you to enter the code on GitHub.',
  expired: 'The code expired. Retry for a new one.',
  denied: 'GitHub sign-in was denied. Retry to start again.',
}

/**
 * The device-flow sign-in (GitHub Copilot): Authenticate adds the worker if
 * it is not running, gets a code, copies it and opens the provider's page.
 * The ADE then checks for the sign-in when this tab gets focus again, and
 * 4, 8, 16 and 32 seconds after the code or the focus. Retry, shown from the
 * first click until the sign-in lands, starts over with a new code.
 */
export function DeviceSignIn({
  provider,
  onConnected,
}: {
  provider: DeviceProvider
  /** The sign-in landed; the provider's catalog fills next. */
  onConnected?: () => void
}) {
  const [phase, setPhase] = useState<Phase>('idle')
  const [userCode, setUserCode] = useState<string | null>(null)
  const [pageUrl, setPageUrl] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const poller = useRef<LoginPoller | null>(null)
  const onConnectedRef = useRef(onConnected)
  onConnectedRef.current = onConnected

  useEffect(() => () => poller.current?.stop(), [])

  useEffect(() => {
    if (phase !== 'waiting') return
    const onFocus = () => poller.current?.focus()
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [phase])

  const authenticate = async () => {
    poller.current?.stop()
    // Opened inside the click, so the browser allows it; pointed at the
    // page once the code is back.
    const tab = window.open('', '_blank')
    if (tab) tab.opener = null
    setPhase('starting')
    setUserCode(null)
    setMessage(null)
    try {
      const code = await startDeviceLogin(provider, ({ note }) => {
        if (note) setMessage(note)
      })
      setUserCode(code.user_code)
      setPageUrl(code.verification_uri)
      void copyTextToClipboard(code.user_code)
      if (tab) tab.location.href = code.verification_uri
      else window.open(code.verification_uri, '_blank', 'noopener')
      setMessage('Code copied. Paste it on the GitHub page that opened.')
      setPhase('waiting')
      const next = createLoginPoller({
        poll: () => pollDeviceLogin(provider, code.device_code),
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
    } catch (error) {
      tab?.close()
      setMessage(readableError(error))
      setPhase('stopped')
    }
  }

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
          : `Uses ${provider.plan}, no API key. Authenticate opens GitHub with a one-time code.`}
      </p>
      {userCode && phase !== 'connected' ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="sr-only">One-time code:</span>
          <span className="rounded-sm bg-panel-raised px-2 py-1 font-mono text-[15px] font-medium tracking-[0.12em] text-ink">
            {userCode}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void copyTextToClipboard(userCode)}
          >
            <Copy aria-hidden />
            Copy
          </Button>
          {pageUrl ? (
            <Button variant="ghost" size="sm" asChild>
              <a href={pageUrl} target="_blank" rel="noopener noreferrer">
                <ExternalLink aria-hidden />
                Open GitHub
              </a>
            </Button>
          ) : null}
        </div>
      ) : null}
      {message && phase !== 'connected' ? (
        <p
          className="font-sans text-[12px] text-ink-faint"
          role="status"
          aria-live="polite"
        >
          {message}
        </p>
      ) : null}
      {phase !== 'connected' ? (
        <span className="flex items-center gap-2">
          {phase === 'idle' ? (
            <Button size="sm" onClick={() => void authenticate()}>
              Authenticate
            </Button>
          ) : (
            <Button
              variant="pill"
              size="sm"
              onClick={() => void authenticate()}
              disabled={phase === 'starting'}
            >
              {phase === 'starting' ? (
                <LoaderCircle className="iii-ui-spin" aria-hidden />
              ) : (
                <RotateCw aria-hidden />
              )}
              Retry
            </Button>
          )}
        </span>
      ) : null}
    </section>
  )
}
