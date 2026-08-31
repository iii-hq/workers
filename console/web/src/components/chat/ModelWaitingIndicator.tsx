import { useEffect, useRef, useState } from 'react'
import { Wordmark } from '@/components/ui/Wordmark'
import { useMediaQuery } from '@/hooks/use-media-query'
import './thinking-motion.css'

export const MODEL_WAITING_LABEL_SWAP_MS = 150

export function formatModelWaitElapsed(elapsedMs: number): string {
  const totalSeconds = Math.floor(elapsedMs / 100) / 10
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`
  return `${Math.floor(totalSeconds / 60)}m ${(totalSeconds % 60).toFixed(1)}s`
}

export function modelWaitElapsedMs(
  startedAt: number,
  currentTime = Date.now(),
): number {
  return Math.max(0, currentTime - startedAt)
}

function useModelWaitElapsed(active: boolean, turnKey?: string): string {
  const [elapsedMs, setElapsedMs] = useState(0)
  const accumulatedRef = useRef(0)
  const activeStartedAtRef = useRef<number | null>(null)
  const turnKeyRef = useRef(turnKey)

  const turnChanged = turnKeyRef.current !== turnKey

  useEffect(() => {
    if (turnChanged) {
      turnKeyRef.current = turnKey
      accumulatedRef.current = 0
      setElapsedMs(0)
    }

    if (!active) {
      activeStartedAtRef.current = null
      setElapsedMs(accumulatedRef.current)
      return
    }

    const startedAt = Date.now()
    activeStartedAtRef.current = startedAt
    setElapsedMs(accumulatedRef.current)
    const timer = window.setInterval(() => {
      setElapsedMs(accumulatedRef.current + modelWaitElapsedMs(startedAt))
    }, 100)

    return () => {
      window.clearInterval(timer)
      if (activeStartedAtRef.current === startedAt) {
        accumulatedRef.current += modelWaitElapsedMs(startedAt)
        activeStartedAtRef.current = null
      }
    }
  }, [active, turnChanged, turnKey])

  // Effects reset the timer machinery after commit. Deriving the first paint
  // as zero prevents the previous turn's elapsed time flashing meanwhile.
  return formatModelWaitElapsed(turnChanged ? 0 : elapsedMs)
}

interface WaitingLabelState {
  current: string
  outgoing: string | null
  revision: number
}

export function ModelWaitingIndicator({
  label = 'thinking…',
  active = true,
  turnKey,
}: {
  label?: string
  active?: boolean
  /** Resets the elapsed clock only when a genuinely new turn starts. */
  turnKey?: string
}) {
  const resolvedLabel = label.trim() || 'thinking…'
  const elapsed = useModelWaitElapsed(active, turnKey)
  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const [labelState, setLabelState] = useState<WaitingLabelState>(() => ({
    current: resolvedLabel,
    outgoing: null,
    revision: 0,
  }))
  const [longestLabel, setLongestLabel] = useState(resolvedLabel)
  useEffect(() => {
    setLongestLabel((current) =>
      resolvedLabel.length > current.length ? resolvedLabel : current,
    )
    setLabelState((current) => {
      if (current.current === resolvedLabel) {
        return active ? current : { ...current, outgoing: null }
      }
      if (!active || reducedMotion) {
        return {
          current: resolvedLabel,
          outgoing: null,
          revision: current.revision + 1,
        }
      }
      return {
        current: resolvedLabel,
        outgoing: current.current,
        revision: current.revision + 1,
      }
    })
  }, [active, reducedMotion, resolvedLabel])

  useEffect(() => {
    if (!labelState.outgoing) return
    const revision = labelState.revision
    const timer = window.setTimeout(
      () => {
        setLabelState((current) =>
          current.revision === revision
            ? { ...current, outgoing: null }
            : current,
        )
      },
      reducedMotion ? 0 : MODEL_WAITING_LABEL_SWAP_MS,
    )
    return () => window.clearTimeout(timer)
  }, [labelState.outgoing, labelState.revision, reducedMotion])

  return (
    <div
      role="status"
      aria-hidden={!active}
      aria-label={resolvedLabel}
      data-active={active}
      data-model-waiting=""
      className="chat-model-waiting flex max-w-full min-w-0 items-center gap-2.5"
    >
      <Wordmark appearance="loading" className="size-4" />
      <div
        aria-hidden="true"
        className="chat-model-waiting__label-stack min-w-0 font-sans text-base font-medium sm:text-[0.8125rem]"
      >
        <span className="chat-model-waiting__label-sizer">{longestLabel}</span>
        {labelState.outgoing ? (
          <span
            key={`outgoing-${labelState.revision}`}
            className="chat-model-waiting__label thinking-shimmer"
            data-motion="exiting"
          >
            {labelState.outgoing}
          </span>
        ) : null}
        <span
          key={`current-${labelState.revision}`}
          className="chat-model-waiting__label thinking-shimmer"
          data-motion={labelState.outgoing ? 'entering' : 'idle'}
        >
          {labelState.current}
        </span>
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
