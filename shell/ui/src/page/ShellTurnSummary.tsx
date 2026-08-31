import type { SessionTurnSummaryProps } from '@iii-dev/console-ui'
import { ChevronDown, Files } from 'lucide-react'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  emitShellReviewFileSelection,
  useShellReviewSummary,
} from './review-summary-store'

export function ShellTurnSummary({ sessionId }: SessionTurnSummaryProps) {
  const summary = useShellReviewSummary(sessionId)
  const summaryIdentity =
    summary && summary.files.length > 0
      ? `${sessionId}\u0000${summary.sourceId}\u0000${summary.turnId}`
      : null
  const [openIdentity, setOpenIdentity] = useState<string | null>(null)
  const open = summaryIdentity !== null && openIdentity === summaryIdentity
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const focusWithinRef = useRef(false)

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

  const closeAndRestoreFocus = useCallback(() => {
    setOpenIdentity(null)
    triggerRef.current?.focus()
  }, [])

  // The open state belongs to one concrete summary. Clear the old identity
  // before paint so switching away and later returning cannot reopen it.
  useLayoutEffect(() => {
    if (openIdentity !== null && openIdentity !== summaryIdentity) {
      setOpenIdentity(null)
    }
  }, [openIdentity, summaryIdentity])

  // A source/turn replacement closes before paint. If focus belonged to the
  // outgoing popover, return it to the persistent trigger before that subtree
  // becomes inert (or the summary unmounts altogether).
  useLayoutEffect(() => {
    const root = rootRef.current
    const trigger = triggerRef.current
    const slot = root?.closest<HTMLElement>('[data-chat-turn-summary-slot]')
    const composer = slot?.nextElementSibling
    const focusableSelector =
      'textarea:not([disabled]), input:not([disabled]), [contenteditable="true"], button:not([disabled]), [tabindex]:not([tabindex="-1"])'
    const fallback = composer?.matches(focusableSelector)
      ? (composer as HTMLElement)
      : composer?.querySelector<HTMLElement>(focusableSelector)

    return () => {
      const hadFocus =
        focusWithinRef.current ||
        Boolean(root?.contains(document.activeElement))
      if (!hadFocus) return
      focusWithinRef.current = false

      trigger?.focus()
      focusWithinRef.current = Boolean(
        trigger?.isConnected && root?.contains(document.activeElement),
      )
      // When an empty summary removes its trigger too, hand focus to the
      // adjacent composer after React has finished the commit.
      queueMicrotask(() => {
        if (
          !trigger?.isConnected &&
          fallback?.isConnected &&
          document.activeElement === document.body
        ) {
          fallback.focus()
        }
      })
    }
  }, [summaryIdentity])

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current?.contains(event.target as Node)) return
      if (rootRef.current?.contains(document.activeElement)) {
        triggerRef.current?.focus()
      }
      setOpenIdentity(null)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      closeAndRestoreFocus()
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [closeAndRestoreFocus, open])

  if (!summary || summary.files.length === 0) return null

  const fileLabel = summary.files.length === 1 ? 'file' : 'files'

  return (
    <div
      ref={rootRef}
      className="shui-chat-summary"
      onFocusCapture={() => {
        focusWithinRef.current = true
      }}
      onBlurCapture={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
          return
        }
        focusWithinRef.current = false
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        className="shui-chat-summary-pill"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpenIdentity(open ? null : summaryIdentity)}
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

      <div
        className="shui-chat-summary-popover"
        role="dialog"
        aria-label="Last Turn changed files"
        aria-hidden={!open}
        data-open={open}
        inert={open ? undefined : true}
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
                closeAndRestoreFocus()
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
    </div>
  )
}
