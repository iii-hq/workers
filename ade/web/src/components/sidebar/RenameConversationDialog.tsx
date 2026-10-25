import type { RefObject } from 'react'
import { useEffect, useRef, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { cn } from '@/lib/utils'

interface RenameConversationDialogProps {
  open: boolean
  /** The current title; the field opens with it selected. */
  title: string
  onOpenChange: (open: boolean) => void
  /** A nonempty title that differs from the current one. */
  onSave: (title: string) => void
  /** Where focus returns on close — the row, not the hover-revealed pencil. */
  returnFocusTo?: RefObject<HTMLElement | null>
}

const pillClassName =
  'inline-flex h-9 min-w-20 items-center justify-center rounded-full px-4 text-[13px] font-medium transition-[background-color,color,transform] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] active:scale-[0.97] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:focus-visible:ring-neutral-400'

/**
 * Renaming in a centred dialog instead of the row's inline field: the
 * sidebar row is too narrow to show a whole title while it is edited, and a
 * dialog gives the full name room, a title, a hint and explicit Save and
 * Cancel. Enter saves, Escape and the ✕ cancel.
 */
export function RenameConversationDialog({
  open,
  title,
  onOpenChange,
  onSave,
  returnFocusTo,
}: RenameConversationDialogProps) {
  const [draft, setDraft] = useState(title)
  const inputRef = useRef<HTMLInputElement>(null)

  // Every open starts from the live title, with the whole of it selected:
  // most renames replace the name, so typing straight away is the fast path.
  useEffect(() => {
    if (!open) return
    setDraft(title)
    const frame = requestAnimationFrame(() => {
      inputRef.current?.focus()
      inputRef.current?.select()
    })
    return () => cancelAnimationFrame(frame)
  }, [open, title])

  const next = draft.trim()
  const canSave = next.length > 0

  const submit = () => {
    if (!canSave) return
    if (next !== title) onSave(next)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="w-[min(26rem,calc(100vw-2rem))] gap-0 bg-white p-0 text-neutral-900 ring-1 ring-neutral-200 [--dialog-max-width:26rem] dark:bg-neutral-900 dark:text-neutral-50 dark:ring-neutral-800"
        initialFocus={inputRef}
        finalFocus={returnFocusTo}
        // The dialog portals to the body but stays inside the row's React
        // tree, so pointer and key events would bubble to the row's open,
        // rename and delete handlers.
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        <form
          className="flex flex-col gap-4 p-5"
          onSubmit={(e) => {
            e.preventDefault()
            submit()
          }}
        >
          <div className="flex flex-col gap-1 pr-8">
            <DialogTitle className="text-[15px] leading-5 font-semibold">
              Rename chat
            </DialogTitle>
            <DialogDescription className="text-[13px] text-neutral-500 dark:text-neutral-400">
              Keep it short and recognizable
            </DialogDescription>
          </div>
          <input
            ref={inputRef}
            name="conversation-title"
            aria-label="Chat name"
            value={draft}
            onChange={(e) => setDraft(e.currentTarget.value)}
            autoComplete="off"
            spellCheck={false}
            className="h-10 w-full rounded-lg bg-neutral-100 px-3 text-[14px] text-neutral-900 ring-1 ring-transparent outline-none transition-[box-shadow] duration-[var(--motion-duration-control)] selection:bg-neutral-900 selection:text-neutral-50 placeholder:text-neutral-400 focus-visible:ring-neutral-400 dark:bg-neutral-800 dark:text-neutral-50 dark:selection:bg-white dark:selection:text-neutral-900 dark:placeholder:text-neutral-500 dark:focus-visible:ring-neutral-500"
          />
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => onOpenChange(false)}
              className={cn(
                pillClassName,
                'bg-neutral-100 text-neutral-900 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-50 dark:hover:bg-neutral-700',
              )}
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!canSave}
              className={cn(
                pillClassName,
                'disabled:cursor-not-allowed',
                canSave
                  ? 'bg-neutral-900 text-neutral-50 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200'
                  : 'bg-neutral-100 text-neutral-400 dark:bg-neutral-800 dark:text-neutral-500',
              )}
            >
              Save
            </button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
