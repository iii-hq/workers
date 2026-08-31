import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { Caret } from '@/components/ui/Caret'
import { useMediaQuery } from '@/hooks/use-media-query'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'
import './thinking-motion.css'

export const THOUGHT_SETTLE_DURATION_MS = 150

interface ThoughtMessageProps {
  message: ThoughtMessageType
  /** Keep a completed thought around just long enough to crossfade its successor. */
  settling?: boolean
  /** Lets a parent presence manager remove the retained row after its exit. */
  onSettled?: () => void
}

export function ThoughtMessage({
  message,
  settling = false,
  onSettled,
}: ThoughtMessageProps) {
  const [retained, setRetained] = useState(() => message.streaming || settling)
  const [expanded, setExpanded] = useState(false)
  const [hasOverflow, setHasOverflow] = useState(false)
  const viewportRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const wasStreamingRef = useRef(message.streaming)
  const onSettledRef = useRef(onSettled)
  const contentId = useId()
  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const explicitLineCount = message.content.split('\n').length
  onSettledRef.current = onSettled

  useEffect(() => {
    const transitionedToSettling = wasStreamingRef.current && !message.streaming
    wasStreamingRef.current = message.streaming

    if (message.streaming) {
      setRetained(true)
      return
    }

    if (!settling && !transitionedToSettling) {
      setRetained(false)
      return
    }

    setRetained(true)
    const delay = reducedMotion ? 0 : THOUGHT_SETTLE_DURATION_MS
    const timer = window.setTimeout(() => {
      setRetained(false)
      onSettledRef.current?.()
    }, delay)

    return () => window.clearTimeout(timer)
  }, [message.streaming, reducedMotion, settling])

  useLayoutEffect(() => {
    const viewport = viewportRef.current
    const content = contentRef.current
    if (!viewport || !content) return
    let frame: number | null = null

    const updateOverflow = () => {
      const lineHeight = Number.parseFloat(getComputedStyle(content).lineHeight)
      const fiveLines = Number.isFinite(lineHeight) ? lineHeight * 5 : 0
      viewport.style.setProperty(
        '--chat-thought-content-height',
        `${content.scrollHeight}px`,
      )
      setHasOverflow(
        fiveLines > 0
          ? content.scrollHeight > fiveLines + 1
          : explicitLineCount > 5 ||
              content.scrollHeight > viewport.clientHeight + 1,
      )
    }
    const scheduleOverflowUpdate = () => {
      if (frame !== null) cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        frame = null
        updateOverflow()
      })
    }

    updateOverflow()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(scheduleOverflowUpdate)
    observer.observe(viewport)
    observer.observe(content)
    return () => {
      observer.disconnect()
      if (frame !== null) cancelAnimationFrame(frame)
    }
  }, [explicitLineCount])

  if (!retained) return null

  const state = message.streaming ? 'streaming' : 'settling'

  return (
    <div
      aria-hidden={state === 'settling'}
      className="chat-thought"
      data-state={state}
      inert={state === 'settling'}
    >
      <div className="chat-thought__presence-inner">
        <details className="iii-details group/thought" open>
          <summary className="inline-flex items-center gap-2 font-sans text-[12px] text-ink-faint hover:text-ink transition-colors select-none">
            <span
              aria-hidden
              className="iii-chev text-ink-ghost w-[8px] inline-block"
            >
              ▸
            </span>
            <span className="flex items-center gap-1.5">
              <span className="thinking-shimmer">Thought…</span>
              {message.streaming ? (
                <Caret className="h-[10px] w-[5px]" />
              ) : null}
            </span>
          </summary>
          <div className="mt-2 ml-2 pl-3 border-l border-rule-2 font-sans text-[13px] text-ink-faint italic whitespace-pre-wrap break-words">
            <div
              ref={viewportRef}
              id={contentId}
              className="chat-thought__viewport"
              data-expanded={expanded}
              data-overflow={hasOverflow}
            >
              <div ref={contentRef} className="chat-thought__content">
                {message.content || (
                  <span className="text-ink-ghost not-italic">
                    No content yet…
                  </span>
                )}
              </div>
            </div>
            {hasOverflow ? (
              <button
                type="button"
                className="chat-thought__disclosure"
                aria-controls={contentId}
                aria-expanded={expanded}
                onClick={() => setExpanded((value) => !value)}
              >
                {expanded ? 'Show latest 5 lines' : 'Show full thinking'}
              </button>
            ) : null}
          </div>
        </details>
      </div>
    </div>
  )
}
