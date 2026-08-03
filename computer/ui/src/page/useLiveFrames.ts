import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  extractStreamFrame,
  FRAMES_STREAM,
  readFrame,
  startScreencast,
  stopScreencast,
  takeScreenshot,
} from '../lib/computer'
import { useComputerStream } from '../lib/events'

/**
 * Live desktop for the selected session, fed by the worker's screencast
 * stream: `screencast::start` on mount, then the worker pushes each frame
 * onto the `computer:frames` stream (group = session id) and this hook
 * appends what arrives — the same engine-pushes / client-appends pattern the
 * Traces view uses. No polling. One `computer::frame` seed read paints the
 * current frame immediately (the stream only delivers frames produced after
 * the subscription); a `computer::screenshot` is the last-resort first paint.
 * `screencast::stop` runs on unmount and session switch (idempotent).
 */

export interface LiveFrame {
  dataUrl: string
  /** Desktop pixel size the image maps to (the `act` coordinate space). */
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

  // The stale image never bleeds into a newly selected session: this resets
  // on session change only.
  useEffect(() => {
    setFrame(null)
    setError(null)
    lastSeqRef.current = 0
  }, [sessionId])

  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false

    // Retain the start so teardown can wait for it to settle before stopping;
    // otherwise a late start could reactivate the screencast after cleanup.
    const started = startScreencast(host.iii, sessionId)

    void (async () => {
      try {
        await started
      } catch (e) {
        // Screencast unavailable (permission refused, driver down): one
        // screenshot so the viewport is not blank, and surface why.
        const shot = await takeScreenshot(host.iii, sessionId).catch(() => null)
        if (cancelled) return
        if (shot?.dataUrl) {
          setFrame({
            dataUrl: shot.dataUrl,
            width: shot.width,
            height: shot.height,
          })
        } else {
          setError(e instanceof Error ? e.message : String(e))
        }
        return
      }
      if (cancelled) return
      // Immediate first paint: the stream only delivers frames produced after
      // the subscription, so read the current one once.
      const seed = await readFrame(host.iii, sessionId).catch(() => null)
      if (cancelled || !seed?.frame) return
      if (seed.frame_seq > lastSeqRef.current) {
        lastSeqRef.current = seed.frame_seq
        setFrame({
          dataUrl: `data:${seed.mime};base64,${seed.frame}`,
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
        .then(() => stopScreencast(host.iii, sessionId))
        .catch(() => {})
    }
  }, [host, enabled, sessionId])

  useComputerStream({
    host,
    enabled: enabled && !!sessionId,
    streamName: FRAMES_STREAM,
    groupId: sessionId,
    fnId: 'iii::computer-ui::frames',
    onFrame: (payload) => {
      const f = extractStreamFrame(payload)
      if (!f || f.frame_seq <= lastSeqRef.current) return
      lastSeqRef.current = f.frame_seq
      setFrame({
        dataUrl: `data:${f.mime};base64,${f.data}`,
        width: f.width,
        height: f.height,
      })
      setError(null)
    },
  })

  return { frame, loading: frame === null && error === null, error }
}
