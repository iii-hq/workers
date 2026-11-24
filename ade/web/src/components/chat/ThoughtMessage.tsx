import { ChevronRight } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { Caret } from '@/components/ui/Caret'
import { formatTurnDuration } from '@/lib/turn-summary'
import { cn } from '@/lib/utils'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'
import './thinking-motion.css'

interface ThoughtMessageProps {
  message: ThoughtMessageType
}

/** The fold's label: live while the model thinks, a record once it is done. */
export function thoughtLabel(message: ThoughtMessageType): string {
  if (message.streaming) return 'Thinking'
  return message.durationMs > 0
    ? `Thought for ${formatTurnDuration(message.durationMs)}`
    : 'Thought'
}

/**
 * The model's reasoning as a row of the transcript. While it streams the
 * fold is open and shows the latest five lines with a live caret; when it
 * finishes it folds to its label and stays in the chat, so the thinking
 * behind an answer can be read back at any time (and after a reload).
 */
export function ThoughtMessage({ message }: ThoughtMessageProps) {
  const streaming = message.streaming === true
  // Open while live; a finished thought starts folded (one line), and the
  // reader unfolds it. The fold is the person's once they touch it.
  const [open, setOpen] = useState(streaming)
  const [expanded, setExpanded] = useState(false)
  const [hasOverflow, setHasOverflow] = useState(false)
  const viewportRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const wasStreamingRef = useRef(streaming)
  const contentId = useId()
  const explicitLineCount = message.content.split('\n').length

  // Streaming ended: fold the thought so the answer below it has the room.
  // A run of thoughts shares one fold, so the next thought joining it opens
  // it again while that one streams.
  useEffect(() => {
    if (wasStreamingRef.current && !streaming) setOpen(false)
    if (!wasStreamingRef.current && streaming) setOpen(true)
    wasStreamingRef.current = streaming
  }, [streaming])

  // The fold's content has no box while it is closed, so the measure runs
  // again when the reader opens it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `open` re-measures a fold that had no layout while closed
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
  }, [explicitLineCount, open])

  const state = streaming ? 'streaming' : 'done'

  return (
    <div className="chat-thought" data-state={state}>
      <div className="chat-thought__presence-inner">
        <details
          className="iii-details group/thought"
          open={open}
          onToggle={(event) => setOpen(event.currentTarget.open)}
        >
          <summary className="chat-thought__summary inline-flex items-center gap-1.5 select-none">
            <ChevronRight
              aria-hidden
              className="iii-chev size-4 shrink-0 text-ink-ghost"
            />
            <span className="flex items-center gap-1.5">
              <span
                className={cn(
                  streaming ? 'thinking-shimmer' : 'text-ink-faint',
                )}
              >
                {thoughtLabel(message)}
              </span>
              {streaming ? <Caret className="h-[10px] w-[5px]" /> : null}
            </span>
          </summary>
          <div className="chat-thought__body whitespace-pre-wrap break-words">
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
