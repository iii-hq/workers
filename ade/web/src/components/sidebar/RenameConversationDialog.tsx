import type { RefObject } from 'react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'

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
        className="w-[min(26rem,calc(100vw-2rem))] gap-0 bg-dialog-surface p-0 text-foreground ring-1 ring-dialog-edge [--dialog-max-width:26rem]"
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
            <DialogDescription className="text-body text-muted-foreground">
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
            className="h-10 w-full rounded-lg bg-muted px-3 text-sm text-foreground ring-1 ring-transparent outline-none transition-[box-shadow] duration-[var(--motion-duration-control)] selection:bg-primary selection:text-primary-foreground placeholder:text-ink-ghost focus-visible:ring-ring/50"
          />
          <div className="flex items-center justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              size="lg"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" size="lg" disabled={!canSave}>
              Save
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
