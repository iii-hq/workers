/* Rollback: undo the working-tree changes of several files at once. The
   dialog lists what the toolbar or a row asked for, every file ticked;
   untick to keep one. "Delete local copies of added files" decides whether
   an added file is removed or only un-added (left on disk, unversioned). */

import { Button, Checkbox, Dialog, DialogContent, DialogDescription, DialogTitle } from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import { ChangesTree, ROW_HEIGHT } from './ChangesTree'
import { changeRows, changeSummary } from './commit-tree'
import type { GitComparisonEntry } from './git'

interface RollbackDialogProps {
  /** The files offered; null keeps the dialog closed. */
  entries: readonly GitComparisonEntry[] | null
  busy: boolean
  onCancel: () => void
  onConfirm: (entries: readonly GitComparisonEntry[], options: { keepAdded: boolean }) => void
}

export function RollbackDialog({ entries, busy, onCancel, onConfirm }: RollbackDialogProps) {
  const [unticked, setUnticked] = useState<ReadonlySet<string>>(new Set())
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set())
  const [deleteAdded, setDeleteAdded] = useState(true)
  // Each opening starts with everything ticked and unfolded.
  useEffect(() => {
    if (entries === null) return
    setUnticked(new Set())
    setClosed(new Set())
  }, [entries])

  const offered = entries ?? []
  const picked = useMemo(() => offered.filter((entry) => !unticked.has(entry.path)), [offered, unticked])
  const rows = useMemo(
    () =>
      changeRows([{ id: 'changes', label: 'Changes', entries: offered }], {
        byDirectory: true,
        isOpen: (key) => !closed.has(key),
      }),
    [offered, closed],
  )
  const hasAdded = picked.some((entry) => entry.status === 'added')
  const count = picked.length

  return (
    <Dialog open={entries !== null} onOpenChange={(open) => (open ? undefined : onCancel())}>
      <DialogContent className="shui-rollback-dialog">
        <DialogTitle>
          Rollback {offered.length} {offered.length === 1 ? 'change' : 'changes'}?
        </DialogTitle>
        <DialogDescription>Working-tree changes to the ticked files are lost. This can't be undone.</DialogDescription>
        {/* A windowed tree has no height of its own: up to 260px of rows, plus the 4px padding. */}
        <div className="shui-rollback-tree" style={{ height: Math.min(260, rows.length * ROW_HEIGHT) + 8 }}>
          <ChangesTree
            rows={rows}
            isIncluded={(entry) => !unticked.has(entry.path)}
            onInclude={(changed, on) =>
              setUnticked((current) => {
                const next = new Set(current)
                for (const entry of changed) {
                  if (on) next.delete(entry.path)
                  else next.add(entry.path)
                }
                return next
              })
            }
            onToggleOpen={(key) =>
              setClosed((current) => {
                const next = new Set(current)
                if (next.has(key)) next.delete(key)
                else next.add(key)
                return next
              })
            }
          />
        </div>
        <div className="shui-rollback-options">
          <Checkbox
            label="Delete local copies of added files"
            checked={hasAdded && deleteAdded}
            disabled={!hasAdded}
            onChange={(event) => setDeleteAdded(event.currentTarget.checked)}
          />
          <span className="shui-rollback-summary">{changeSummary(picked) || 'Nothing selected'}</span>
        </div>
        <div className="shui-rollback-actions">
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            type="button"
            size="sm"
            className="shui-danger-button"
            disabled={busy || count === 0}
            onClick={() => onConfirm(picked, { keepAdded: !deleteAdded })}
          >
            {count > 1 ? `Rollback ${count} files` : 'Rollback'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
