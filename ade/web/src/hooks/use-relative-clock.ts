import { useEffect, useState } from 'react'

/** Refresh compact relative timestamps without rerendering every second. */
export function useRelativeClock(timestamp?: number | null): number {
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    if (timestamp == null) return
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [timestamp])

  return now
}
