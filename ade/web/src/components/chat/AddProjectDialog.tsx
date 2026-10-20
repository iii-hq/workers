import { useCallback, useState } from 'react'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
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
        setError(`can't use ${dir} — ${res.error}`)
        setValidating(false)
        return
      }
      try {
        await upsertHarnessProject(res.path)
      } catch (err) {
        setError(`can't save this project — ${errMsg(err)}`)
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
        aria-describedby={undefined}
        className="flex h-[min(40rem,85vh)] w-[min(60rem,calc(100vw-2rem))] max-w-none flex-col overflow-hidden p-0"
        onOpenAutoFocus={(event) => {
          // No keyboard to type with: don't raise the on-screen one.
          if (!pointerFine) event.preventDefault()
        }}
      >
        <DialogTitle className="px-4 pt-4 pb-3">Add project</DialogTitle>
        <FolderBrowser
          wide={!phone}
          keyboard={pointerFine}
          busy={validating}
          error={error}
          onUse={(dir) => void use(dir)}
        />
      </DialogContent>
    </Dialog>
  )
}
