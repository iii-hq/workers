import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { isAutoAcceptable } from '@/lib/backend/auto-accept-policy'

const isDestructiveFunction = (id: string) => !isAutoAcceptable(id)

interface AlwaysAllowButtonProps {
  functionId: string
  onConfirm: () => void | Promise<void>
  disabled?: boolean
  submitting?: boolean
}

/**
 * Third button on a pending approval prompt: "approve always". Approves
 * the current call AND remembers the decision for the rest of this
 * session (per-conversation `approved_always`, honored in every mode),
 * so the same function stops prompting. For functions whose id matches
 * the potentially-destructive verb policy (write / delete / exec / send /
 * credential / …) we gate on a confirmation dialog that shows the
 * canonical `function_id` so the user can't misclick into a standing
 * grant.
 */
export function AlwaysAllowButton({
  functionId,
  onConfirm,
  disabled,
  submitting,
}: AlwaysAllowButtonProps) {
  const [confirmOpen, setConfirmOpen] = useState(false)
  const destructive = isDestructiveFunction(functionId)

  function handleClick() {
    if (destructive) {
      setConfirmOpen(true)
      return
    }
    void onConfirm()
  }

  return (
    <>
      <Button
        variant="pill"
        size="sm"
        onClick={handleClick}
        disabled={disabled || submitting}
        title={
          destructive
            ? 'potentially destructive function — confirmation required'
            : 'approve and stop asking for this function this session'
        }
      >
        {submitting
          ? 'saving…'
          : destructive
            ? 'approve always (potentially destructive)'
            : 'approve always'}
      </Button>
      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent className="max-w-md">
          <DialogTitle className="text-[14px]">
            approve always for this potentially destructive function?
          </DialogTitle>
          <DialogDescription className="mt-3 text-ink">
            you're about to let the agent trigger{' '}
            <code className="px-1 bg-paper-2 text-accent">{functionId}</code>{' '}
            for the rest of this conversation with no further prompts.
          </DialogDescription>
          <DialogDescription className="mt-2 text-ink-faint">
            this function matched the potentially-destructive verb policy (write
            / delete / exec / send / credential / …). approve once if you only
            want it this time.
          </DialogDescription>
          <div className="mt-6 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setConfirmOpen(false)}
              className="font-mono text-[12px] px-3 py-1 border border-rule text-ink-faint hover:text-ink hover:border-ink transition-colors"
            >
              cancel
            </button>
            <button
              type="button"
              onClick={() => {
                setConfirmOpen(false)
                void onConfirm()
              }}
              className="font-mono text-[12px] px-3 py-1 border border-alert bg-alert text-bg hover:bg-bg hover:text-alert transition-colors"
            >
              approve always {functionId}
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
