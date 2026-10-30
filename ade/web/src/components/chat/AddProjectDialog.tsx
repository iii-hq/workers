import { FolderPlus } from 'lucide-react'
import { useCallback, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { upsertHarnessProject } from '@/lib/backend/projects'
import { errMsg, validateWorkspaceDir } from '@/lib/working-dir'
import { FolderBrowser } from './FolderBrowser'

interface AddProjectDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The worker-echoed canonical path of the folder just added. */
  onAdded: (path: string) => void
}

/**
 * "Add project" on its own, outside the directory picker: the folder
 * browser in a dialog. A chosen folder is validated against the live shell
 * worker and remembered as a project before `onAdded` fires with the
 * canonical path, the same contract the picker's own Add project keeps.
 *
 * The dialog is the shadcn one: a titled header that says what choosing
 * does, the browser as the body, and the browser's own footer (keys on the
 * left, Cancel and Use on the right) as the dialog's footer.
 */
export function AddProjectDialog({
  open,
  onOpenChange,
  onAdded,
}: AddProjectDialogProps) {
  const pointerFine = useMediaQuery(DESKTOP_POINTER_QUERY)
  const phone = useMediaQuery('(max-width: 767px)')
  const [validating, setValidating] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const use = useCallback(
    async (raw: string) => {
      const dir = raw.trim().replace(/\/+$/, '') || '/'
      setError(null)
      setValidating(true)
      const res = await validateWorkspaceDir(dir)
      if (!res.ok) {
        setError(`Can't use ${dir}: ${res.error}`)
        setValidating(false)
        return
      }
      try {
        await upsertHarnessProject(res.path)
      } catch (err) {
        setError(`Can't save this project: ${errMsg(err)}`)
        setValidating(false)
        return
      }
      setValidating(false)
      onOpenChange(false)
      onAdded(res.path)
    },
    [onAdded, onOpenChange],
  )

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setError(null)
        onOpenChange(next)
      }}
    >
      <DialogContent
        className="flex h-[min(38rem,calc(100dvh-2rem))] w-[min(56rem,calc(100vw-2rem))] max-w-none [--dialog-max-width:none] flex-col gap-0 overflow-hidden p-0"
        // No keyboard to type with: don't raise the on-screen one.
        initialFocus={pointerFine ? undefined : () => false}
      >
        <DialogHeader className="flex-row items-start gap-3 border-b border-border px-5 py-4 pr-14">
          <span
            aria-hidden
            className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-foreground"
          >
            <FolderPlus className="size-4" />
          </span>
          <div className="flex min-w-0 flex-col gap-1">
            <DialogTitle>Add project</DialogTitle>
            <DialogDescription>
              Choose a folder for the chat to work in. It is remembered as a
              project, so the next chat can pick it in one click.
            </DialogDescription>
          </div>
        </DialogHeader>
        <FolderBrowser
          wide={!phone}
          keyboard={pointerFine}
          busy={validating}
          error={error}
          onUse={(dir) => void use(dir)}
          onCancel={() => onOpenChange(false)}
        />
      </DialogContent>
    </Dialog>
  )
}
