import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useId, useRef, useState } from 'react'
import { FRAME_EVENT, frameEvent, type SimApi, type TouchPoint } from '../lib/api'

export interface Screen {
  src: string
  /** Device framebuffer pixels: the coordinate space of every touch. */
  deviceWidth: number
  deviceHeight: number
}

/** Renew the watch lease this often; the worker holds it for 15s. */
const RENEW_MS = 5_000

/**
 * The live screen of one booted simulator. `watch` starts (and every few
 * seconds renews) the worker's capture; frames then arrive as
 * `ios-simulator::frame-event` triggers bound to this udid — the worker
 * pushes them straight to this tab, no polling and no other worker in
 * between. One `frame` read paints immediately, since the binding only
 * carries frames produced after it. Stops by itself when the lease lapses.
 */
export function useLiveScreen(host: Host, api: SimApi, udid: string | null, enabled: boolean) {
  const [screen, setScreen] = useState<Screen | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [fps, setFps] = useState(0)
  const lastSeq = useRef(0)
  const arrivals = useRef<number[]>([])
  const size = useRef({ w: 0, h: 0 })
  const instance = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    setScreen(null)
    setError(null)
    lastSeq.current = 0
  }, [udid])

  const apply = (data: string, seq: number, w: number, h: number) => {
    if (seq <= lastSeq.current) return
    lastSeq.current = seq
    if (w && h) size.current = { w, h }
    const now = performance.now()
    arrivals.current = [...arrivals.current.filter((t) => now - t < 1000), now]
    setScreen({
      src: `data:image/jpeg;base64,${data}`,
      deviceWidth: size.current.w,
      deviceHeight: size.current.h,
    })
    setError(null)
  }

  useEffect(() => {
    if (!enabled || !udid) return
    let cancelled = false
    const renew = async (first: boolean) => {
      try {
        const lease = await api.watch(udid)
        size.current = { w: lease.device_width, h: lease.device_height }
        if (first) {
          const seed = await api.frame(udid)
          if (!cancelled && seed.frame) apply(seed.frame, seed.seq, seed.device_width, seed.device_height)
        }
      } catch (err) {
        if (!cancelled) setError(errorMessage(err))
      }
    }
    void renew(true)
    const timer = window.setInterval(() => void renew(false), RENEW_MS)
    const fpsTimer = window.setInterval(() => {
      const now = performance.now()
      setFps(arrivals.current.filter((t) => now - t < 1000).length)
    }, 1000)

    const handler = `iii::ios-simulator-ui::frames::${instance}`
    const offs: Array<() => void> = []
    try {
      offs.push(
        host.iii.on(handler, (payload: unknown) => {
          const f = frameEvent(payload)
          if (f && f.udid === udid && !cancelled) apply(f.data, f.seq, f.device_width, f.device_height)
        }),
      )
      offs.push(
        host.iii.registerTrigger({
          type: FRAME_EVENT,
          function_id: `${handler}::${host.iii.browserId}`,
          config: { tenant: api.tenant, udid },
        }),
      )
    } catch (err) {
      setError(`live view unavailable: ${errorMessage(err)}`)
    }
    return () => {
      cancelled = true
      window.clearInterval(timer)
      window.clearInterval(fpsTimer)
      for (const off of offs) off()
    }
    // biome-ignore lint/correctness/useExhaustiveDependencies: apply only touches refs and setters
  }, [host, api, udid, enabled, instance])

  return { screen, error, fps }
}

type Job = { kind: 'move'; point: TouchPoint } | { kind: 'call'; run: () => Promise<unknown> }

/**
 * Every input to one simulator goes through one queue, one call in flight
 * at a time, so a touch-up can never overtake its last move and typing
 * stays in order. Pending moves coalesce (the newest wins); nothing else
 * is ever dropped.
 */
export function createInputQueue(api: SimApi, udid: string, onError: (message: string) => void) {
  const jobs: Job[] = []
  let busy = false

  const pump = async () => {
    if (busy) return
    busy = true
    while (jobs.length > 0) {
      const job = jobs.shift() as Job
      try {
        await (job.kind === 'move' ? api.touch(udid, job.point) : job.run())
      } catch (err) {
        onError(errorMessage(err))
      }
    }
    busy = false
  }

  return {
    touch(point: TouchPoint) {
      const last = jobs[jobs.length - 1]
      if (point.phase === 'move' && last?.kind === 'move') last.point = point
      else if (point.phase === 'move') jobs.push({ kind: 'move', point })
      else jobs.push({ kind: 'call', run: () => api.touch(udid, point) })
      void pump()
    },
    run(run: () => Promise<unknown>) {
      jobs.push({ kind: 'call', run })
      void pump()
    },
  }
}

export type InputQueue = ReturnType<typeof createInputQueue>
