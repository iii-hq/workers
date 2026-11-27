import { useEffect, useState } from 'react'
import type { TurnRest } from '@/lib/backend/harness-send'

interface TurnRestRowProps {
  sessionId: string
  /** The session reads `done`, its transcript is loaded, nothing streams here. */
  atRest: boolean
  turnRest: (sessionId: string) => Promise<TurnRest | null>
}

/**
 * Tells a turn that waits from one that ended: a turn that stopped short
 * reads the same as a finished answer.
 */
export function TurnRestRow({ sessionId, atRest, turnRest }: TurnRestRowProps) {
  const [rest, setRest] = useState<TurnRest | null>(null)
  useEffect(() => {
    if (!atRest) return
    let alive = true
    void turnRest(sessionId)
      .then((next) => {
        if (alive) setRest(next)
      })
      .catch(() => {})
    return () => {
      alive = false
      setRest(null)
    }
  }, [sessionId, atRest, turnRest])
  if (!atRest || !rest) return null
  return (
    <p role="status" className="mb-2 px-1 text-sm text-ink-faint">
      {rest === 'waiting' ? 'Waiting for a notification.' : 'Turn ended.'}
    </p>
  )
}
