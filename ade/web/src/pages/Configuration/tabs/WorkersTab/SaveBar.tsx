import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { Button } from '@/components/shadcn/button'
import { settingsEnter, settingsFade } from '../../components/motion'

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
 * in flight / has just failed — operators never see a useless bar on a
 * freshly-loaded form. It rises 8px into place on the first edit and
 * fades out once the draft is clean again; under reduced motion only the
 * opacity runs.
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
  const reduceMotion = useReducedMotion()
  const inFlight = status.kind === 'saving'
  const shouldShow = dirty || inFlight || status.kind === 'error'
  const hidden = reduceMotion
    ? { opacity: 0 }
    : { opacity: 0, transform: 'translateY(8px)' }
  const shown = reduceMotion
    ? { opacity: 1 }
    : { opacity: 1, transform: 'translateY(0px)' }

  return (
    <AnimatePresence initial={false}>
      {shouldShow ? (
        <motion.div
          key="save-bar"
          initial={hidden}
          animate={{ ...shown, transition: settingsEnter }}
          exit={{ opacity: 0, transition: settingsFade }}
          className="sticky inset-x-0 bottom-0 flex min-h-12 items-center justify-between gap-4 border-t border-neutral-200 bg-white/95 px-5 py-2 backdrop-blur-sm @3xl:px-14 dark:border-neutral-800 dark:bg-black/95"
        >
          <SaveStatusText status={status} dirty={dirty} />
          <div className="flex shrink-0 items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={onReset}
              disabled={inFlight}
            >
              Reset
            </Button>
            <Button
              type="button"
              onClick={onSave}
              disabled={inFlight || saveDisabled || !dirty}
            >
              {inFlight ? 'Saving…' : 'Save changes'}
            </Button>
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>
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
      <p className="truncate text-sm leading-5 text-destructive" role="alert">
        Error: {status.message}
      </p>
    )
  }
  if (status.kind === 'saving') {
    return <p className="text-sm leading-5 text-muted-foreground">Saving…</p>
  }
  if (status.kind === 'saved' && !dirty) {
    return <p className="text-sm leading-5 text-muted-foreground">Saved</p>
  }
  if (dirty) {
    return (
      <p className="text-sm leading-5 text-muted-foreground">Unsaved changes</p>
    )
  }
  return null
}
