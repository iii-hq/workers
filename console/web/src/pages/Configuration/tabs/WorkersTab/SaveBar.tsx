import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import { wt } from './typography'

export type SaveStatus =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'saved'; savedAtMs: number }
  | { kind: 'error'; message: string }

interface SaveBarProps {
  dirty: boolean
  onSave: () => void
  onReset: () => void
  status: SaveStatus
  /**
   * Disables Save when the form has a structural issue the operator can
   * see (e.g. duplicate dictionary key, empty key). The bar still shows
   * so the operator can Reset out of the bad state without leaving the
   * page.
   */
  saveDisabled?: boolean
}

/**
 * Sticky bottom save bar. Renders only when dirty or while a save is
 * in flight / has just succeeded — operators never see a useless bar on
 * a freshly-loaded form.
 *
 * Status text is intentionally calm: `saving…`, `saved`, `error: …`.
 * No icons; the rest of the console uses typographic affordances and
 * a single accent color, and this bar follows suit.
 *
 * The bar is anchored inside the editor (not portalled to the page
 * shell) so it scrolls with the form on tall configurations. The
 * sticky-bottom positioning sits the bar just above the bottom edge of
 * the editor's scroll viewport.
 */
export function SaveBar({
  dirty,
  onSave,
  onReset,
  status,
  saveDisabled,
}: SaveBarProps) {
  const inFlight = status.kind === 'saving'
  const shouldShow = dirty || inFlight || status.kind === 'error'
  if (!shouldShow) return null

  return (
    <div className="sticky bottom-0 inset-x-0 border-t border-rule bg-bg px-6 py-3 flex items-center justify-between gap-4 shadow-[0_-4px_12px_-8px_rgba(0,0,0,0.15)]">
      <SaveStatusText status={status} dirty={dirty} />
      <div className="flex items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={wt.control}
          onClick={onReset}
          disabled={inFlight}
        >
          reset
        </Button>
        <Button
          type="button"
          variant="primary"
          size="sm"
          className={wt.control}
          onClick={onSave}
          disabled={inFlight || saveDisabled || !dirty}
        >
          {inFlight ? 'saving…' : 'save'}
        </Button>
      </div>
    </div>
  )
}

function SaveStatusText({
  status,
  dirty,
}: {
  status: SaveStatus
  dirty: boolean
}) {
  if (status.kind === 'error') {
    return (
      <p className={cn(wt.bodySm, 'text-alert truncate')} role="alert">
        error: {status.message}
      </p>
    )
  }
  if (status.kind === 'saving') {
    return <p className={cn(wt.bodySm, 'text-ink-faint')}>saving…</p>
  }
  if (status.kind === 'saved' && !dirty) {
    return <p className={cn(wt.bodySm, 'text-ink-faint')}>saved</p>
  }
  if (dirty) {
    return (
      <p className={cn(wt.bodySm, 'text-ink')}>
        <span className="text-accent" aria-hidden>
          ·
        </span>{' '}
        unsaved changes
      </p>
    )
  }
  return null
}
