import type { SessionTurnSummaryProps } from '@iii-dev/console-ui'
import { ChevronDown, Files } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import {
  emitShellReviewFileSelection,
  useShellReviewSummary,
} from './review-summary-store'

export function ShellTurnSummary({ sessionId }: SessionTurnSummaryProps) {
  const summary = useShellReviewSummary(sessionId)
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const totals = useMemo(
    () =>
      summary?.files.reduce(
        (total, file) => ({
          add: total.add + (file.add ?? 0),
          del: total.del + (file.del ?? 0),
          ready: total.ready + (file.state === 'ready' ? 1 : 0),
          pending: total.pending + (file.state === 'pending' ? 1 : 0),
          unavailable:
            total.unavailable + (file.state === 'unavailable' ? 1 : 0),
        }),
        { add: 0, del: 0, ready: 0, pending: 0, unavailable: 0 },
      ) ?? { add: 0, del: 0, ready: 0, pending: 0, unavailable: 0 },
    [summary],
  )

  useEffect(() => setOpen(false), [sessionId, summary?.sourceId, summary?.turnId])

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      setOpen(false)
      triggerRef.current?.focus()
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  if (!summary || summary.files.length === 0) return null

  const fileLabel = summary.files.length === 1 ? 'file' : 'files'

  return (
    <div ref={rootRef} className="shui-chat-summary">
      <button
        ref={triggerRef}
        type="button"
        className="shui-chat-summary-pill"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((value) => !value)}
      >
        <Files aria-hidden className="files-icon" />
        <span>
          {summary.files.length} {fileLabel} changed
        </span>
        {totals.ready > 0 ? (
          <>
            <span className="add">+{totals.add}</span>
            <span className="del">−{totals.del}</span>
          </>
        ) : null}
        {totals.pending > 0 || totals.unavailable > 0 ? (
          <span
            role="status"
            title={`${totals.pending} pending, ${totals.unavailable} unavailable`}
            aria-label={`${totals.pending} change totals pending, ${totals.unavailable} unavailable`}
          >
            …
          </span>
        ) : null}
        <ChevronDown aria-hidden className={`chevron${open ? ' open' : ''}`} />
      </button>

      {open ? (
        <div
          className="shui-chat-summary-popover"
          role="dialog"
          aria-label="Last Turn changed files"
        >
          <div className="shui-chat-summary-head">
            <span>Last Turn</span>
            <span>
              {summary.files.length} {fileLabel}
            </span>
          </div>
          <div className="shui-chat-summary-files">
            {summary.files.map((file) => (
              <button
                key={file.path}
                type="button"
                className="shui-chat-summary-file"
                title={file.path}
                aria-label={
                  file.state === 'ready'
                    ? `${file.path}, ${file.add} additions, ${file.del} deletions`
                    : `${file.path}, change totals ${file.state}`
                }
                onClick={() => {
                  emitShellReviewFileSelection(sessionId, {
                    sourceId: summary.sourceId,
                    path: file.path,
                  })
                  setOpen(false)
                }}
              >
                <span className="path">{file.path}</span>
                {file.state === 'ready' ? (
                  <span className="stats">
                    <span className="add">+{file.add}</span>
                    <span className="del">−{file.del}</span>
                  </span>
                ) : (
                  <span className="stats" title={`change totals ${file.state}`}>
                    {file.state === 'pending' ? '…' : '—'}
                  </span>
                )}
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  )
}
