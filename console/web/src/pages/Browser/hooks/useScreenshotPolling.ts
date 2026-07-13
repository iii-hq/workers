import { useCallback, useEffect, useState } from 'react'
import { type BrowserScreenshot, takeBrowserScreenshot } from '@/lib/browser'

/**
 * Screenshot polling for the selected session (v1 viewport: no screencast).
 * A self-scheduling loop keeps at most one capture in flight, pauses while
 * the tab is hidden, and resumes with an immediate capture on visibility.
 * `refresh()` short-circuits the wait after an act or navigation so the
 * viewport reflects the interaction right away.
 */

export const SCREENSHOT_POLL_MS = 2_000
/** Faster cadence while pick mode is on, so the hover target stays fresh. */
export const SCREENSHOT_PICK_POLL_MS = 500

export interface ScreenshotPolling {
  shot: BrowserScreenshot | null
  /** No successful capture yet for the current session. */
  loading: boolean
  error: string | null
  refresh: () => void
}

export function useScreenshotPolling(
  sessionId: string | null,
  enabled: boolean,
  intervalMs: number = SCREENSHOT_POLL_MS,
): ScreenshotPolling {
  const [shot, setShot] = useState<BrowserScreenshot | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  // The stale capture never bleeds into a newly selected session.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on session change only
  useEffect(() => {
    setShot(null)
    setError(null)
  }, [sessionId])

  // biome-ignore lint/correctness/useExhaustiveDependencies: token restarts the loop for an immediate capture; the effect body doesn't read it
  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false
    let timer: number | undefined

    const schedule = () => {
      if (cancelled) return
      timer = window.setTimeout(tick, intervalMs)
    }

    const tick = async () => {
      if (cancelled) return
      if (document.hidden) {
        schedule()
        return
      }
      try {
        const next = await takeBrowserScreenshot(sessionId)
        if (cancelled) return
        if (next) {
          setShot(next)
          setError(null)
        }
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
      }
      schedule()
    }

    const onVisibility = () => {
      if (document.hidden || cancelled) return
      window.clearTimeout(timer)
      void tick()
    }

    void tick()
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [enabled, sessionId, token, intervalMs])

  return { shot, loading: shot === null && error === null, error, refresh }
}
