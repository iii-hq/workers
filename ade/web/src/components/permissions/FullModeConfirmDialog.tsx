import { Button } from '@/components/shadcn/button'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

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
        <Button
          type="button"
          variant="secondary"
          size="lg"
          onClick={onCancel}
          className="min-h-12 sm:min-h-9"
        >
          Cancel
        </Button>
        {/* Full mode drops every safety prompt, so enabling it reads as danger. */}
        <Button
          type="button"
          variant="destructive"
          size="lg"
          onClick={onConfirm}
          className="min-h-12 sm:min-h-9"
        >
          Enable full
        </Button>
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
