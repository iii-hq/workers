import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import { EMPTY_FEED, feedForSession, mergeEvents, pairingIsCurrent, type EventFeed, type EventPage, type PairingCode, type Session } from './model'

/** Serial cursor polling: events are best-effort, so a live binding cannot replace reconciliation. */
export function useSession(host: Host, sessionId: string, paused: boolean) {
  const [session, setSession] = useState<Session | null>(null)
  const [qr, setQr] = useState<PairingCode | null>(null)
  const [feed, setFeed] = useState<EventFeed>(EMPTY_FEED)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [revision, setRevision] = useState(0)
  const cursor = useRef(EMPTY_FEED)
  const identity = useRef<Session | null>(null)
  const refresh = useCallback(() => setRevision((value) => value + 1), [])

  useEffect(() => {
    let active = true
    let busy = false
    setQr(null)
    if (paused) return
    const poll = async () => {
      if (busy || document.hidden) return
      busy = true
      try {
        const next = await host.iii.trigger<Session>('whatsapp::sessions::get', { session_id: sessionId })
        if (!active) return
        const previous = feedForSession(cursor.current, identity.current, next)
        const page = await host.iii.trigger<EventPage>('whatsapp::events::poll', { session_id: sessionId, after_sequence: previous.cursor, limit: 100 })
        if (!active) return
        identity.current = next
        setSession(next)
        cursor.current = mergeEvents(previous, page, sessionId)
        setFeed(cursor.current)
        if (next.status !== 'pairing') setQr(null)
        const pairing = next.status === 'pairing'
          ? await host.iii.trigger<PairingCode | null>('whatsapp::sessions::qr', { session_id: sessionId })
          : null
        if (!active) return
        setQr(pairingIsCurrent(pairing, next.status, Date.now()) ? pairing : null)
        setError(null)
      } catch (cause) {
        if (!active) return
        setQr(null)
        setError(errorMessage(cause))
      } finally {
        busy = false
        if (active) setLoading(false)
      }
    }
    void poll()
    const timer = window.setInterval(() => void poll(), 2500)
    const visible = () => { if (!document.hidden) void poll() }
    document.addEventListener('visibilitychange', visible)
    return () => {
      active = false
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', visible)
    }
  }, [host, sessionId, paused, revision])
  return { session, qr, feed, error, loading, refresh }
}
