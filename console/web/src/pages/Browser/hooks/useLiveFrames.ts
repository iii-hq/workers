import { useEffect, useState } from 'react'
import {
  readBrowserFrame,
  startBrowserScreencast,
  stopBrowserScreencast,
  takeBrowserScreenshot,
} from '@/lib/browser'

/**
 * Live view for the selected session, fed by the worker's screencast
 * pipeline: `screencast::start` on mount, then a fast `browser::frame`
 * cursor poll (a memory read on the worker; Chromium pushes the frames, so
 * there is no capture round-trip). A response without `frame` means
 * nothing changed and nothing re-renders. One `browser::screenshot` paints
 * the viewport while the first frame is still in flight; if the screencast
 * surface is unavailable (older worker) the hook degrades to plain
 * screenshot polling. Polling pauses while the tab is hidden and resumes
 * with an immediate read; `screencast::stop` runs on unmount and session
 * switch (idempotent).
 */

export const FRAME_POLL_MS = 150
const FALLBACK_POLL_MS = 2_000

export interface LiveFrame {
  dataUrl: string
  /** Page-viewport size the image maps to (input coordinate space). */
  width: number
  height: number
}

export interface LiveViewState {
  frame: LiveFrame | null
  /** No image yet for the current session. */
  loading: boolean
  error: string | null
}

export function useLiveFrames(
  sessionId: string | null,
  enabled: boolean,
): LiveViewState {
  const [frame, setFrame] = useState<LiveFrame | null>(null)
  const [error, setError] = useState<string | null>(null)

  // The stale image never bleeds into a newly selected session.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on session change only
  useEffect(() => {
    setFrame(null)
    setError(null)
  }, [sessionId])

  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false
    let timer: number | undefined
    let lastSeq = 0
    let gotFrame = false
    // Flips off when the screencast surface is unavailable; the loop then
    // degrades to plain screenshot polling.
    let streaming = true
    // One re-start per outage: the worker restarting drops the screencast
    // (active=false) without ending the session.
    let restartAttempted = false

    const schedule = () => {
      if (cancelled) return
      timer = window.setTimeout(
        tick,
        streaming ? FRAME_POLL_MS : FALLBACK_POLL_MS,
      )
    }

    const paintScreenshot = async () => {
      const shot = await takeBrowserScreenshot(sessionId)
      if (cancelled || gotFrame || !shot?.dataUrl) return
      setFrame({
        dataUrl: shot.dataUrl,
        width: shot.width,
        height: shot.height,
      })
      setError(null)
    }

    const tick = async () => {
      if (cancelled) return
      if (document.hidden) {
        schedule()
        return
      }
      try {
        if (streaming) {
          const res = await readBrowserFrame(
            sessionId,
            lastSeq > 0 ? lastSeq : undefined,
          )
          if (cancelled) return
          if (res) {
            if (!res.active && !restartAttempted) {
              restartAttempted = true
              await startBrowserScreencast(sessionId).catch(() => {})
            } else if (res.active) {
              restartAttempted = false
            }
            if (res.frame) {
              lastSeq = res.frame_seq
              gotFrame = true
              setFrame({
                dataUrl: `data:image/jpeg;base64,${res.frame}`,
                width: res.width,
                height: res.height,
              })
              setError(null)
            } else if (res.frame_seq > lastSeq) {
              lastSeq = res.frame_seq
            }
          }
        } else {
          const shot = await takeBrowserScreenshot(sessionId)
          if (cancelled) return
          if (shot?.dataUrl) {
            setFrame({
              dataUrl: shot.dataUrl,
              width: shot.width,
              height: shot.height,
            })
            setError(null)
          }
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

    void (async () => {
      try {
        await startBrowserScreencast(sessionId)
      } catch {
        streaming = false
      }
      if (cancelled) return
      void tick()
      if (streaming) {
        void paintScreenshot().catch(() => {})
      }
    })()

    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
      void stopBrowserScreencast(sessionId).catch(() => {})
    }
  }, [enabled, sessionId])

  return { frame, loading: frame === null && error === null, error }
}
