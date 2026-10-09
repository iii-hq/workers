import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useState } from 'react'
import {
  FRAME_CHANGED_TRIGGER,
  readFrame,
  startScreencast,
  stopScreencast,
  takeScreenshot,
} from '../lib/computer'
import { createFrameFollower, parseFrameChange } from '../lib/frameFollower'

/**
 * Live desktop for the selected session, notify-then-fetch: the worker keeps
 * only the newest frame per session and fires `computer::frame-changed`
 * (`{ session_id }` filter, small image-free payload) after each one; this
 * hook reads the frame with `computer::frame`. No polling.
 *
 * Order on open: bind first, `screencast::start`, then one `computer::frame`
 * read (the initial paint; notifications only cover frames produced after
 * the binding). A `computer::screenshot` is the last-resort first paint when
 * the screencast cannot start. Each notification marks the view dirty; the
 * follower runs at most one read at a time and skips intermediate frames, so
 * a slow tab never queues frames (see lib/frameFollower). After a
 * reconnect it reads again (notifications missed while away are gone, the
 * stored frame is not). `screencast::stop` runs on unmount and session
 * switch (idempotent).
 *
 * The handler id carries the `iii::` prefix so per-frame invocations stay
 * span-suppressed and out of the trace feed; the per-mount instance id keeps
 * two hook instances from colliding.
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
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  // The stale image never bleeds into a newly selected session: this resets
  // on session change only.
  useEffect(() => {
    setFrame(null)
    setError(null)
  }, [sessionId])

  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false

    const follower = createFrameFollower({
      sessionId,
      read: (since) => readFrame(host.iii, sessionId, since),
      apply: (f) => {
        if (cancelled) return
        setFrame({
          dataUrl: `data:${f.mime};base64,${f.data}`,
          width: f.width,
          height: f.height,
        })
        setError(null)
      },
    })

    // 1. Bind first, so no frame stored after the initial read is missed.
    const offs: Array<() => void> = []
    const localFnId = `iii::computer-ui::frames::${instanceId}`
    try {
      offs.push(
        host.iii.on(localFnId, (payload: unknown) => {
          const note = parseFrameChange(payload)
          if (note) follower.notify(note)
        }),
      )
      offs.push(
        host.iii.registerTrigger({
          type: FRAME_CHANGED_TRIGGER,
          function_id: `${localFnId}::${host.iii.browserId}`,
          config: { session_id: sessionId },
        }),
      )
    } catch (err) {
      // Trigger type unavailable (worker restarting): the initial read is the
      // only paint. Say so; a silent catch looks like a frozen desktop.
      console.warn('[computer-ui] frame-changed binding failed', err)
    }

    // Recovery: notifications fired while disconnected are gone, the stored
    // frame is not, so read it again once the connection is back.
    let wasConnected = true
    offs.push(
      host.iii.addConnectionStateListener((state) => {
        const connected = state === 'connected'
        if (connected && !wasConnected) void follower.resync()
        wasConnected = connected
      }),
    )

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
      // 2. Initial read: the frame stored right now.
      await follower.resync()
    })()

    return () => {
      cancelled = true
      follower.stop()
      for (const off of offs) {
        try {
          off()
        } catch {
          // already gone
        }
      }
      // Stop only after the start has settled, so the stop can never be
      // overtaken by an in-flight start reactivating the screencast.
      void started
        .catch(() => {})
        .then(() => stopScreencast(host.iii, sessionId))
        .catch(() => {})
    }
  }, [host, enabled, sessionId, instanceId])

  return { frame, loading: frame === null && error === null, error }
}
