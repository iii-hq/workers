import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { isAutoAcceptable } from '@/lib/backend/auto-accept-policy'

const isDestructiveFunction = (id: string) => !isAutoAcceptable(id)

interface AlwaysAllowButtonProps {
  functionId: string
  /** Engine namespace of the held call; the grant covers only that namespace. */
  namespace?: string
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
  namespace,
  onConfirm,
  disabled,
  submitting,
}: AlwaysAllowButtonProps) {
  const [confirmOpen, setConfirmOpen] = useState(false)
  const destructive = isDestructiveFunction(functionId)
  const target = namespace
    ? `${functionId} in namespace ${namespace}`
    : functionId
  const scope = namespace ? ` in namespace ${namespace}` : ''

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
        data-message-action="always-allow"
        onClick={handleClick}
        disabled={disabled || submitting}
        title={
          destructive
            ? 'potentially destructive function — confirmation required'
            : `approve and stop asking for this function${scope} this session`
        }
      >
        {submitting
          ? 'saving…'
          : destructive
            ? 'approve always (potentially destructive)'
            : 'approve always'}
      </Button>
      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title="Always approve this function?"
        description={`The agent can run ${target} for the rest of this conversation without asking. It matches the destructive-action policy (write, delete, exec, send, credentials), so approve once if you only need it now.`}
        confirmLabel="Approve always"
        tone="danger"
        onConfirm={() => {
          void onConfirm()
        }}
      />
    </>
  )
}
