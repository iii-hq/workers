import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'

export type FilesystemAccessAction = 'once' | 'session' | 'always' | 'deny'

interface FilesystemAccessPromptProps {
  /** Canonical root a filesystem grant would target. */
  requestedRoot: string
  /** The session's working directory, if any — shown as "always allowed". */
  workingDir?: string | null
  onResolve: (action: FilesystemAccessAction) => void | Promise<void>
  /** Footer link to the grants management dialog (§5). */
  onManage?: () => void
  /** Disables every button — set when no resolve handler is wired. */
  disabled?: boolean
}

/**
 * Pending-footer replacement for a `filesystem_access` approval record: the
 * agent tried to touch a path outside the session's allowed folders and the
 * gate parked the call for a human. Offers once / session / always / deny
 * instead of the standard approve/deny/always row. "always allow" gates on
 * a destructive-style confirmation (it edits shell's permanent
 * `fs.host_roots` for every conversation), mirroring `AlwaysAllowButton`.
 */
export function FilesystemAccessPrompt({
  requestedRoot,
  workingDir,
  onResolve,
  onManage,
  disabled,
}: FilesystemAccessPromptProps) {
  const [submitting, setSubmitting] = useState<FilesystemAccessAction | null>(
    null,
  )
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [confirmOpen, setConfirmOpen] = useState(false)

  const run = async (action: FilesystemAccessAction) => {
    if (submitting) return
    setSubmitError(null)
    setSubmitting(action)
    try {
      await onResolve(action)
      // Leave `submitting` set; the card clears itself once
      // fcall-approval-cleared (or the resurrected result) lands, exactly
      // like the standard approve/deny/always row.
    } catch (err) {
      setSubmitting(null)
      setSubmitError(err instanceof Error ? err.message : String(err))
    }
  }

  return (
    <div className="border-t border-rule-2 px-3 py-2 flex flex-col gap-2">
      <div className="flex flex-col gap-1">
        <div
          // dir=rtl keeps the trailing (most-distinguishing) path segment
          // visible when truncated, matching the working-dir footer banner.
          dir="rtl"
          className="truncate text-left font-mono text-[12px] text-ink bg-paper-2 px-2 py-1"
          title={requestedRoot}
        >
          {requestedRoot}
        </div>
        <p className="font-mono text-[11px] text-ink-faint">
          this folder is outside the session's allowed folders.
        </p>
        {workingDir ? (
          <p className="font-mono text-[11px] text-ink-ghost">
            workspace: {workingDir} is always allowed.
          </p>
        ) : null}
      </div>

      <div className="flex items-center gap-2 flex-wrap">
        <Button
          variant="primary"
          size="sm"
          onClick={() => void run('once')}
          disabled={disabled || !!submitting}
        >
          {submitting === 'once' ? 'allowing…' : 'allow once'}
        </Button>
        <Button
          variant="pill"
          size="sm"
          onClick={() => void run('session')}
          disabled={disabled || !!submitting}
        >
          {submitting === 'session' ? 'allowing…' : 'allow this session'}
        </Button>
        <Button
          variant="pill"
          size="sm"
          onClick={() => setConfirmOpen(true)}
          disabled={disabled || !!submitting}
        >
          {submitting === 'always' ? 'allowing…' : 'always allow…'}
        </Button>
        <Button
          variant="pill"
          size="sm"
          onClick={() => void run('deny')}
          disabled={disabled || !!submitting}
        >
          {submitting === 'deny' ? 'denying…' : 'deny'}
        </Button>
        {submitting ? (
          <span className="font-mono text-[12px] text-ink-faint">
            waiting for the agent to resume…
          </span>
        ) : null}
      </div>

      {submitError ? (
        <div className="font-mono text-[12px] text-warn">{submitError}</div>
      ) : null}

      {onManage ? (
        <button
          type="button"
          onClick={onManage}
          className="self-start font-mono text-[11px] text-ink-faint hover:text-ink hover:underline underline-offset-2"
        >
          manage filesystem access…
        </button>
      ) : null}

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent className="max-w-md">
          <DialogTitle className="text-[14px]">
            always allow this folder?
          </DialogTitle>
          <DialogDescription className="mt-3 text-ink">
            this adds{' '}
            <code className="px-1 bg-paper-2 text-accent">{requestedRoot}</code>{' '}
            to the permanent allowed folders (shell fs.host_roots) for every
            conversation.
          </DialogDescription>
          <DialogDescription className="mt-2 text-ink-faint">
            prefer "allow this session" if you only need it for this task.
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
                void run('always')
              }}
              className="font-mono text-[12px] px-3 py-1 border border-alert bg-alert text-bg hover:bg-bg hover:text-alert transition-colors"
            >
              always allow {requestedRoot}
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
