import { useEffect, useState } from 'react'
import { Wordmark } from '@/components/ui/Wordmark'

export function formatModelWaitElapsed(elapsedMs: number): string {
  const totalSeconds = Math.floor(elapsedMs / 100) / 10
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`
  return `${Math.floor(totalSeconds / 60)}m ${(totalSeconds % 60).toFixed(1)}s`
}

function useModelWaitElapsed(): string {
  const [elapsedMs, setElapsedMs] = useState(0)

  useEffect(() => {
    const startedAt = performance.now()
    const timer = window.setInterval(() => {
      setElapsedMs(performance.now() - startedAt)
    }, 100)

    return () => window.clearInterval(timer)
  }, [])

  return formatModelWaitElapsed(elapsedMs)
}

export function ModelWaitingIndicator({
  label = 'thinking…',
}: {
  label?: string
}) {
  const elapsed = useModelWaitElapsed()
  const resolvedLabel = label.trim() || 'thinking…'

  return (
    <div
      role="status"
      aria-label={resolvedLabel}
      data-model-waiting=""
      className="flex max-w-full min-w-0 items-center gap-2.5"
    >
      <Wordmark appearance="loading" className="size-4" />
      <div
        aria-hidden="true"
        className="thinking-shimmer min-w-0 truncate font-sans text-base font-medium sm:text-[0.8125rem]"
      >
        {resolvedLabel}
      </div>
      <div
        aria-hidden="true"
        className="min-w-[7ch] shrink-0 text-right font-mono text-base text-ink-ghost tabular-nums sm:text-[0.75rem]"
      >
        {elapsed}
      </div>
    </div>
  )
}
