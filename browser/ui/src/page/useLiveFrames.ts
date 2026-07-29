import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  BROWSER_FRAMES_STREAM,
  extractStreamFrame,
  readBrowserFrame,
  startBrowserScreencast,
  stopBrowserScreencast,
  takeBrowserScreenshot,
} from '../lib/browser'
import { useBrowserStream } from '../lib/events'

/**
 * Live view for the selected session, fed by the worker's screencast stream:
 * `screencast::start` on mount, then the worker pushes each frame onto the
 * `browser:frames` stream (group = session id) and this hook appends the
 * pushed frames, the same engine-pushes / client-appends pattern the Traces
 * view uses. No polling. One `browser::frame` seed read paints the current
 * frame immediately (the stream only delivers frames produced after the
 * subscription); a `browser::screenshot` is the last-resort first paint if
 * the screencast surface is unavailable (older worker). `screencast::stop`
 * runs on unmount and session switch (idempotent).
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
  // session switch.
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
  }, [host, enabled, sessionId])

  useBrowserStream({
    host,
    enabled: enabled && !!sessionId,
    streamName: BROWSER_FRAMES_STREAM,
    groupId: sessionId,
    fnId: 'iii::browser-ui::frames',
    onFrame: (payload) => {
      const f = extractStreamFrame(payload)
      if (!f || f.frame_seq <= lastSeqRef.current) return
      lastSeqRef.current = f.frame_seq
      setFrame({
        dataUrl: `data:image/jpeg;base64,${f.frame}`,
        width: f.width,
        height: f.height,
      })
      setError(null)
    },
  })

  return { frame, loading: frame === null && error === null, error }
}
