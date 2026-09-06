import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  BROWSER_FRAME_EVENT_TRIGGER,
  parseFrameEvent,
  readBrowserFrame,
  startBrowserScreencast,
  stopBrowserScreencast,
  takeBrowserScreenshot,
} from '../lib/browser'
import { useBrowserSessionEvent } from '../lib/events'

/**
 * Live view for the selected tab, fed by the worker's screencast:
 * `screencast::start` on mount, then every frame arrives on the
 * `browser::frame-event` trigger (bound with this tab's `session_id`, the
 * same path the console and network feeds use) and this hook paints it. No
 * polling. One `browser::frame` seed read paints the current frame
 * immediately (the trigger only delivers frames produced after the binding);
 * a `browser::screenshot` is the last-resort first paint if the screencast
 * surface is unavailable (older worker). `screencast::stop` runs on unmount
 * and tab switch (idempotent).
 */

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
  host: Host,
  sessionId: string | null,
  enabled: boolean,
  /** Bump to re-read the current frame once (e.g. after a resize), so the
   * new size paints even if the screencast stream is momentarily quiet. */
  reseedToken = 0,
  /** Bump when the tab's page was reopened under this viewer (it slept and
   * woke, the browser data was cleared): the new page has no screencast
   * until we start one again. */
  wakeToken = 0,
): LiveViewState {
  const [frame, setFrame] = useState<LiveFrame | null>(null)
  const [error, setError] = useState<string | null>(null)
  // Newest applied frame seq, so an out-of-order stream push is ignored.
  const lastSeqRef = useRef(0)

  // The stale image never bleeds into a newly selected session.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on session change only
  useEffect(() => {
    setFrame(null)
    setError(null)
    lastSeqRef.current = 0
  }, [sessionId])

  // Start the screencast, seed the current frame, and clean up on unmount /
  // session switch / wake.
  // biome-ignore lint/correctness/useExhaustiveDependencies: wakeToken re-runs the effect on purpose
  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false

    // Retain the start so teardown can wait for it to settle before stopping;
    // otherwise a late start could reactivate the screencast after cleanup.
    const started = startBrowserScreencast(host.iii, sessionId)

    void (async () => {
      try {
        await started
      } catch {
        // Older worker without the screencast surface: one screenshot so the
        // viewport is not blank.
        const shot = await takeBrowserScreenshot(host.iii, sessionId).catch(
          () => null,
        )
        if (!cancelled && shot?.dataUrl) {
          setFrame({
            dataUrl: shot.dataUrl,
            width: shot.width,
            height: shot.height,
          })
        }
        return
      }
      if (cancelled) return
      // Immediate first paint: the stream only delivers frames produced after
      // the subscription, so read the current one once.
      const seed = await readBrowserFrame(host.iii, sessionId).catch(() => null)
      if (cancelled || !seed?.frame) return
      if (seed.frame_seq > lastSeqRef.current) {
        lastSeqRef.current = seed.frame_seq
        setFrame({
          dataUrl: `data:image/jpeg;base64,${seed.frame}`,
          width: seed.width,
          height: seed.height,
        })
        setError(null)
      }
    })()

    return () => {
      cancelled = true
      // Stop only after the start has settled, so the stop can never be
      // overtaken by an in-flight start reactivating the screencast.
      void started
        .catch(() => {})
        .then(() => stopBrowserScreencast(host.iii, sessionId))
        .catch(() => {})
    }
  }, [host, enabled, sessionId, wakeToken])

  useBrowserSessionEvent({
    host,
    enabled: enabled && !!sessionId,
    triggerType: BROWSER_FRAME_EVENT_TRIGGER,
    sessionId,
    fnId: 'iii::browser-ui::frames',
    onEvent: (payload) => {
      const f = parseFrameEvent(payload)
      if (!f || f.session_id !== sessionId || f.frame_seq <= lastSeqRef.current) return
      lastSeqRef.current = f.frame_seq
      setFrame({
        dataUrl: `data:image/jpeg;base64,${f.frame}`,
        width: f.width,
        height: f.height,
      })
      setError(null)
    },
  })

  // Re-seed on demand: read the current frame once and apply it if newer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: fires on the token, reads live refs
  useEffect(() => {
    if (!enabled || !sessionId || reseedToken === 0) return
    let cancelled = false
    void readBrowserFrame(host.iii, sessionId)
      .then((seed) => {
        if (cancelled || !seed?.frame || seed.frame_seq <= lastSeqRef.current)
          return
        lastSeqRef.current = seed.frame_seq
        setFrame({
          dataUrl: `data:image/jpeg;base64,${seed.frame}`,
          width: seed.width,
          height: seed.height,
        })
        setError(null)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [reseedToken])

  return { frame, loading: frame === null && error === null, error }
}
