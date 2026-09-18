import * as React from 'react'
import {
  addActiveTime,
  dismiss,
  type EmailPromptRecord,
  loadRecord,
  saveRecord,
  shouldPrompt,
  snooze,
  subscribed,
} from '@/lib/email-prompt'

/** How often active time is banked while the tab is on screen. */
const TICK_MS = 15_000

/**
 * Counts time this tab spends on screen and opens the email prompt once it has
 * earned one.
 *
 * `document.hidden` is the whole definition of active: a console left open on
 * another desktop is not usage. The record is banked every tick rather than on
 * unload, so a closed laptop keeps the minutes it actually earned.
 */
export function useEmailPrompt(): {
  open: boolean
  onSnooze: () => void
  onDismiss: () => void
  onSubscribed: () => void
} {
  const [record, setRecord] = React.useState<EmailPromptRecord>(loadRecord)
  const [open, setOpen] = React.useState(false)

  const update = React.useCallback(
    (next: (current: EmailPromptRecord) => EmailPromptRecord) => {
      setRecord((current) => {
        const updated = next(current)
        if (updated !== current) saveRecord(updated)
        return updated
      })
    },
    [],
  )

  React.useEffect(() => {
    if (record.status !== 'pending') return
    let last = Date.now()
    const tick = () => {
      const now = Date.now()
      const elapsed = now - last
      last = now
      if (document.hidden) return
      update((current) => addActiveTime(current, elapsed, TICK_MS))
    }
    const timer = window.setInterval(tick, TICK_MS)
    // A tab coming back on screen must not bank the time it was away for.
    const onVisibility = () => {
      last = Date.now()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [record.status, update])

  React.useEffect(() => {
    // Opening is one-way here: once shown, only a decision closes it, so a
    // later tick cannot reopen the dialog under the operator.
    if (!open && shouldPrompt(record, Date.now())) setOpen(true)
  }, [open, record])

  return {
    open,
    onSnooze: () => {
      setOpen(false)
      update((current) => snooze(current, Date.now()))
    },
    onDismiss: () => {
      setOpen(false)
      update(dismiss)
    },
    onSubscribed: () => {
      setOpen(false)
      update(subscribed)
    },
  }
}
