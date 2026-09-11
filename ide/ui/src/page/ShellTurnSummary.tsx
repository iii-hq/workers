import type { SessionTurnSummaryProps } from '@iii-dev/console-ui'
import { ChevronDown, Files } from 'lucide-react'
import { useId, useRef } from 'react'
import { emitShellReviewFileSelection, useShellReviewSummary } from './review-summary-store'

const POPOVER_GAP_PX = 6

/* The chat footer's "N files changed" pill. Its file list is a native
   popover: the top layer escapes the footer's overflow clip, and the browser
   owns the open state, light dismiss, Escape and focus return. */
export function ShellTurnSummary({ sessionId }: SessionTurnSummaryProps) {
  const summary = useShellReviewSummary(sessionId)
  const popoverId = useId()
  const popoverRef = useRef<HTMLDivElement>(null)

  if (!summary || summary.files.length === 0) return null

  let add = 0
  let del = 0
  let ready = 0
  for (const file of summary.files) {
    if (file.state !== 'ready') continue
    ready += 1
    add += file.add ?? 0
    del += file.del ?? 0
  }
  const withoutTotals = summary.files.length - ready
  const fileLabel = summary.files.length === 1 ? 'file' : 'files'

  return (
    <div className="shui-chat-summary">
      <button
        type="button"
        className="shui-chat-summary-pill"
        popoverTarget={popoverId}
        onClick={(event) => {
          // A top-layer popover is viewport-positioned; pin it above the
          // pill before the click's default action shows it.
          const rect = event.currentTarget.getBoundingClientRect()
          const style = popoverRef.current?.style
          if (!style) return
          style.right = `${window.innerWidth - rect.right}px`
          style.bottom = `${window.innerHeight - rect.top + POPOVER_GAP_PX}px`
        }}
      >
        <Files aria-hidden className="files-icon" />
        <span>
          {summary.files.length} {fileLabel} changed
        </span>
        {ready > 0 ? (
          <>
            <span className="add">+{add}</span>
            <span className="del">−{del}</span>
          </>
        ) : null}
        {withoutTotals > 0 ? (
          <span
            role="status"
            title={`${withoutTotals} without totals yet`}
            aria-label={`${withoutTotals} without totals yet`}
          >
            …
          </span>
        ) : null}
        <ChevronDown aria-hidden className="chevron" />
      </button>

      <div
        id={popoverId}
        ref={popoverRef}
        popover="auto"
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
                popoverRef.current?.hidePopover()
              }}
            >
              <span className="path">{file.path}</span>
              <span className="stats" title={file.state === 'ready' ? undefined : `change totals ${file.state}`}>
                {file.state === 'ready' ? (
                  <>
                    <span className="add">+{file.add}</span>
                    <span className="del">−{file.del}</span>
                  </>
                ) : file.state === 'pending' ? (
                  '…'
                ) : (
                  '—'
                )}
              </span>
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}
