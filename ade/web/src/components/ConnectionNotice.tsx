import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { getDefaultBackend } from '@/lib/backend'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'

export const CONNECTION_NOTICE_TIMEOUT_MS = 10_000

/** Independent of worker presence: a failed probe must not look connected. */
export function ConnectionNotice({
  enabled = getDefaultBackend().id === 'real',
}: {
  enabled?: boolean
}) {
  const [connection, setConnection] = useState<IIIConnectionState>('connecting')
  const [offline, setOffline] = useState(
    () => typeof navigator !== 'undefined' && navigator.onLine === false,
  )
  const [slow, setSlow] = useState(false)

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let off: (() => void) | undefined
    void getIiiClient()
      .then((client) => {
        if (!cancelled) off = client.addConnectionStateListener(setConnection)
      })
      .catch(() => {
        if (!cancelled) setConnection('failed')
      })
    const updateOnline = () =>
      setOffline(typeof navigator !== 'undefined' && navigator.onLine === false)
    window.addEventListener('online', updateOnline)
    window.addEventListener('offline', updateOnline)
    return () => {
      cancelled = true
      off?.()
      window.removeEventListener('online', updateOnline)
      window.removeEventListener('offline', updateOnline)
    }
  }, [enabled])

  const connected = connection === 'connected'
  useEffect(() => {
    if (!enabled || connected) {
      setSlow(false)
      return
    }
    // Do not reset on connecting -> reconnecting: repeated attempts must
    // not leave the user with an indefinite spinner.
    const timer = window.setTimeout(
      () => setSlow(true),
      CONNECTION_NOTICE_TIMEOUT_MS,
    )
    return () => window.clearTimeout(timer)
  }, [enabled, connected])

  if (!enabled || (connected && !offline)) return null
  const failed = offline || slow || connection === 'failed'
  const title = offline
    ? 'You are offline'
    : failed
      ? 'Unable to connect'
      : connection === 'connecting'
        ? 'Connecting…'
        : 'Connection lost. Reconnecting…'
  const detail = offline
    ? 'Check your internet connection. Chats cannot sync while you are offline.'
    : failed
      ? 'The server is not responding. Check your connection and that the server is running.'
      : 'Waiting for the server. Chats may not be up to date.'

  return (
    <div
      role={failed ? 'alert' : 'status'}
      aria-live={failed ? 'assertive' : 'polite'}
      className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-edge bg-panel px-4 py-3 font-sans text-sm text-ink"
    >
      <div className="min-w-0 flex-1">
        <p className="font-medium">{title}</p>
        <p className="text-ink-faint">{detail}</p>
      </div>
      <Button
        type="button"
        variant="pill"
        className="min-h-11"
        onClick={() => window.location.reload()}
      >
        Reload app
      </Button>
    </div>
  )
}
