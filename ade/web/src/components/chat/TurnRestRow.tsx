import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import type { TurnRest } from '@/lib/backend/harness-send'

/** What the Continue button sends, as the person's next message. */
export const CONTINUE_MESSAGE = 'Continue'

interface TurnRestRowProps {
  sessionId: string
  /** The session reads `done`, its transcript is loaded, nothing streams here. */
  atRest: boolean
  turnRest: (sessionId: string) => Promise<TurnRest | null>
  onContinue: () => void
}

/**
 * Tells a turn that waits from one that ended, and offers Continue on the
 * latter: a turn that stopped short reads the same as a finished answer.
 */
export function TurnRestRow({
  sessionId,
  atRest,
  turnRest,
  onContinue,
}: TurnRestRowProps) {
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
    <div className="mb-2 flex items-center gap-2 px-1 text-sm text-ink-faint">
      <p role="status">
        {rest === 'waiting' ? 'Waiting for a notification.' : 'Turn ended.'}
      </p>
      {rest === 'ended' ? (
        <Button
          variant="pill"
          size="sm"
          onClick={() => {
            // The chat stays at rest until the send lands: a second click
            // would send again.
            setRest(null)
            onContinue()
          }}
        >
          Continue
        </Button>
      ) : null}
    </div>
  )
}
