import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'

interface FullModeConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => void
  /**
   * Context-aware copy. The Configuration screen flow phrases it as
   * "for every new conversation"; the in-chat picker phrases it as
   * "for this conversation". The default works for the in-chat case.
   */
  scope?: 'conversation' | 'default'
}

export function FullModeConfirmDialog({
  open,
  onOpenChange,
  onConfirm,
  scope = 'conversation',
}: FullModeConfirmDialogProps) {
  const target =
    scope === 'default' ? 'every new conversation' : 'this conversation'
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogTitle className="text-[14px]">
          enable full permissions
        </DialogTitle>
        <DialogDescription className="mt-3 text-ink">
          full permissions let the agent run any function in {target} without
          asking — including writing files, executing shell commands, sending
          messages, and reading secrets.
        </DialogDescription>
        <DialogDescription className="mt-2 text-ink-faint">
          you can revert from the banner at the top of the chat at any time.
        </DialogDescription>
        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="font-mono text-[12px] px-3 py-1 border border-rule text-ink-faint hover:text-ink hover:border-ink transition-colors"
          >
            cancel
          </button>
          <button
            type="button"
            onClick={() => {
              onConfirm()
              onOpenChange(false)
            }}
            className="font-mono text-[12px] px-3 py-1 border border-ink bg-ink text-bg hover:bg-bg hover:text-ink transition-colors"
          >
            enable full
          </button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
