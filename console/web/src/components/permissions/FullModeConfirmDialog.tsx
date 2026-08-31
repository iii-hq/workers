import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
import { cn } from '@/lib/utils'

interface FullModeConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => void
  /**
   * Context-aware copy. The global Settings flow phrases it as
   * "for every new conversation"; the in-chat picker phrases it as
   * "for this conversation". The default works for the in-chat case.
   */
  scope?: 'conversation' | 'default'
}

interface FullModeConfirmContentProps {
  onCancel: () => void
  onConfirm: () => void
  scope?: 'conversation' | 'default'
  className?: string
}

/** Shared warning body for modal and in-sheet confirmation flows. */
export function FullModeConfirmContent({
  onCancel,
  onConfirm,
  scope = 'conversation',
  className,
}: FullModeConfirmContentProps) {
  const target =
    scope === 'default' ? 'every new conversation' : 'this conversation'
  return (
    <div className={cn('font-sans', className)}>
      <p className="text-base leading-relaxed text-ink">
        Full permissions let the agent run any function in {target} without
        asking — including writing files, executing shell commands, sending
        messages, and reading secrets.
      </p>
      <p className="mt-2 text-base leading-relaxed text-ink-faint">
        You can revert from the banner at the top of the chat at any time.
      </p>
      <div className="mt-6 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <button
          type="button"
          onClick={onCancel}
          className="min-h-12 rounded-sm px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:min-h-9 sm:font-mono sm:text-[12px]"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={onConfirm}
          className="min-h-12 rounded-sm bg-ink px-3 font-sans text-base text-bg hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:min-h-9 sm:font-mono sm:text-[12px]"
        >
          Enable full
        </button>
      </div>
    </div>
  )
}

export function FullModeConfirmDialog({
  open,
  onOpenChange,
  onConfirm,
  scope = 'conversation',
}: FullModeConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogTitle className="text-[14px]">
          enable full permissions
        </DialogTitle>
        <FullModeConfirmContent
          scope={scope}
          className="mt-3"
          onCancel={() => onOpenChange(false)}
          onConfirm={() => {
            onConfirm()
            onOpenChange(false)
          }}
        />
      </DialogContent>
    </Dialog>
  )
}
