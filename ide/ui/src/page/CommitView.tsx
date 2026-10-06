/* The Commit tab: a toolbar (refresh, rollback, stash, view options,
   expand/collapse), the change tree with a tick per change, and the commit
   box. Rolling back several files goes through one dialog that lists them.
   Every row has a right-click menu (scm-menus). */

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  ConfirmDialog,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  EmptyState,
  IconButton,
  Skeleton,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { Archive, ChevronsDownUp, ChevronsUpDown, CircleAlert, Eye, RefreshCw, Undo2 } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { ChangesTree } from './ChangesTree'
import { CommitBox } from './CommitBox'
import { useContextMenu } from './ContextMenu'
import { type ChangeGroup, changeRows, changeSummary, expandableKeys } from './commit-tree'
import type { GitComparisonEntry } from './git'
import { basename } from './paths'
import { RollbackDialog } from './RollbackDialog'
import { changeMenu } from './scm-menus'
import { readScmShowUnversioned, readScmViewMode, writeScmShowUnversioned, writeScmViewMode } from './scm-view'
import { TextDialog } from './TextDialog'
import type { SourceControlState } from './use-source-control'
import { useSpin } from './use-spin'

interface CommitViewProps {
  host: Host
  root: string | null
  conversationId?: string | null
  scm: SourceControlState
  /** The change whose diff is in front. */
  activePath: string | null
  onOpenChange: (entry: GitComparisonEntry, pin: boolean) => void
  /** The working copy, in an editor tab. */
  onOpenFile: (path: string) => void
  /** "Compare with…": the file beside a revision picked in the tab. */
  onCompare: (path: string) => void
  /** The Git window's log, narrowed to these root-relative paths. */
  onShowHistory: (paths: string[]) => void
  /** Deletes the file the way the Explorer does: tabs on it close, the tree updates. */
  onDeleteFile: (path: string) => Promise<void>
}

/** Hands `text` to the browser as a file download. */
function download(name: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/x-patch' }))
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.click()
  // Some browsers read the blob after click() returns: free it later.
  setTimeout(() => URL.revokeObjectURL(url), 30_000)
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`
}

export function CommitView({
  host,
  root,
  conversationId,
  scm,
  activePath,
  onOpenChange,
  onOpenFile,
  onCompare,
  onShowHistory,
  onDeleteFile,
}: CommitViewProps) {
  // Grouped by directory until told otherwise.
  const [byDirectory, setByDirectory] = useState(() => readScmViewMode(undefined, 'tree') === 'tree')
  const [showUnversioned, setShowUnversioned] = useState(readScmShowUnversioned)
  const [open, setOpen] = useState<ReadonlyMap<string, boolean>>(new Map())
  const [rollbackEntries, setRollbackEntries] = useState<readonly GitComparisonEntry[] | null>(null)
  const spinning = useSpin(scm.refreshing)
  const [stashEntries, setStashEntries] = useState<readonly GitComparisonEntry[] | null>(null)
  const [deleteEntry, setDeleteEntry] = useState<GitComparisonEntry | null>(null)
  const messageRef = useRef<HTMLTextAreaElement>(null)
  const menu = useContextMenu()
  // The row the open menu acts on, marked in the tree while it is open.
  const [menuRow, setMenuRow] = useState<string | null>(null)

  const groups = useMemo<ChangeGroup[]>(
    () => [
      { id: 'changes', label: 'Changes', entries: scm.changes },
      ...(showUnversioned
        ? [{ id: 'unversioned' as const, label: 'Unversioned files', entries: scm.unversioned }]
        : []),
    ],
    [scm.changes, scm.unversioned, showUnversioned],
  )
  const rows = useMemo(
    () => changeRows(groups, { byDirectory, isOpen: (key, fallback) => open.get(key) ?? fallback }),
    [groups, byDirectory, open],
  )
  const setAllOpen = (value: boolean) => setOpen(new Map(expandableKeys(groups).map((key) => [key, value])))

  // The toolbar's Rollback acts on the ticked tracked changes, or on the
  // change whose diff is open when nothing is ticked.
  const tickedTracked = scm.changes.filter(scm.isIncluded)
  const rollbackTargets =
    tickedTracked.length > 0 ? tickedTracked : scm.changes.filter((entry) => entry.path === activePath)
  // Stash sets aside the ticked changes (unversioned ones included), or the
  // change whose diff is open when nothing is ticked.
  const stashTargets =
    scm.included.length > 0
      ? scm.included
      : [...scm.changes, ...scm.unversioned].filter((entry) => entry.path === activePath)

  const total = scm.changes.length + scm.unversioned.length

  // Without a status to show, the view is hidden rather than unmounted:
  // the commit box keeps the message being typed for when one reads again.
  return (
    <>
      {scm.phase === 'not-a-repo' ? (
        <div className="shui-side-empty">
          <EmptyState title="No repository" description="This folder is not inside a Git repository." />
        </div>
      ) : scm.phase === 'error' ? (
        <div className="shui-commit-error">
          <StatusPanel
            variant="alert"
            icon={<CircleAlert aria-hidden />}
            headline="Couldn't read the changes"
            detail={scm.error}
            action={
              <Button type="button" variant="pill" size="sm" onClick={scm.reload}>
                Retry
              </Button>
            }
          />
        </div>
      ) : null}
      <div className="shui-commit" hidden={scm.phase === 'not-a-repo' || scm.phase === 'error'}>
        <div className="shui-commit-toolbar" role="toolbar" aria-label="Changes">
          <IconButton label="Refresh" disabled={scm.busy} onClick={scm.reload} aria-busy={spinning}>
            <RefreshCw aria-hidden className={spinning ? uiClasses.spin : undefined} />
          </IconButton>
          <IconButton
            label={
              rollbackTargets.length > 0
                ? `Rollback ${plural(rollbackTargets.length, 'file', 'files')}…`
                : 'Tick the changes to roll back'
            }
            disabled={scm.busy || rollbackTargets.length === 0}
            onClick={() => setRollbackEntries(rollbackTargets)}
          >
            <Undo2 aria-hidden />
          </IconButton>
          <IconButton
            label={
              stashTargets.length > 0
                ? `Stash ${plural(stashTargets.length, 'file', 'files')}…`
                : 'Tick the changes to stash'
            }
            disabled={scm.busy || stashTargets.length === 0}
            onClick={() => setStashEntries(stashTargets)}
          >
            <Archive aria-hidden />
          </IconButton>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <IconButton label="View options">
                <Eye aria-hidden />
              </IconButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuLabel>Group by</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={byDirectory}
                onSelect={(event) => event.preventDefault()}
                onCheckedChange={(checked) => {
                  setByDirectory(checked)
                  writeScmViewMode(checked ? 'tree' : 'list')
                }}
              >
                Directory
              </DropdownMenuCheckboxItem>
              <DropdownMenuLabel>Show</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={showUnversioned}
                onSelect={(event) => event.preventDefault()}
                onCheckedChange={(checked) => {
                  setShowUnversioned(checked)
                  writeScmShowUnversioned(checked)
                }}
              >
                Unversioned files
              </DropdownMenuCheckboxItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <span className="spacer" />
          <IconButton label="Expand all" disabled={!byDirectory && rows.length === 0} onClick={() => setAllOpen(true)}>
            <ChevronsUpDown aria-hidden />
          </IconButton>
          <IconButton label="Collapse all" onClick={() => setAllOpen(false)}>
            <ChevronsDownUp aria-hidden />
          </IconButton>
        </div>

        <div className="shui-commit-tree">
          {scm.phase === 'loading' || scm.phase === 'idle' ? (
            <div className="shui-commit-skeleton" role="status" aria-label="Reading git status">
              <Skeleton className="shui-commit-skeleton-row" />
              <Skeleton className="shui-commit-skeleton-row" />
              <Skeleton className="shui-commit-skeleton-row" />
            </div>
          ) : total === 0 ? (
            <div className="shui-commit-clean">
              <span className="title">No changes</span>
              <span className="detail">The working tree matches {scm.branch ? `HEAD on ${scm.branch}` : 'HEAD'}.</span>
            </div>
          ) : (
            <ChangesTree
              rows={rows}
              isIncluded={scm.isIncluded}
              onInclude={scm.setIncluded}
              onToggleOpen={(key) =>
                setOpen((current) => {
                  const row = rows.find((candidate) => candidate.key === key)
                  const next = new Map(current)
                  next.set(key, !(row && row.kind !== 'file' ? row.open : false))
                  return next
                })
              }
              activePath={activePath}
              onOpen={onOpenChange}
              onRollback={setRollbackEntries}
              onStash={setStashEntries}
              menuTarget={menu.isOpen ? menuRow : null}
              onMenu={(row, anchor) => {
                setMenuRow(row.key)
                menu.open(
                  anchor,
                  changeMenu(row, {
                    root: root ?? '',
                    busy: scm.busy,
                    commit: (entries) => {
                      scm.setIncluded([...scm.changes, ...scm.unversioned], false)
                      scm.setIncluded(entries, true)
                      messageRef.current?.focus()
                    },
                    rollback: setRollbackEntries,
                    stash: setStashEntries,
                    open: onOpenChange,
                    jump: onOpenFile,
                    copy: (text) => void copyText(text),
                    remove: setDeleteEntry,
                    add: (entries) => void scm.add(entries),
                    ignore: (paths) => void scm.ignore(paths),
                    savePatch: (entries) =>
                      void scm.patch(entries, async (patch) => {
                        download('changes.patch', patch)
                        return `saved ${changeSummary(entries)} as changes.patch`
                      }),
                    copyPatch: (entries) =>
                      void scm.patch(entries, async (patch) => {
                        if (!(await copyText(patch))) throw new Error('the clipboard refused it')
                        return `copied ${changeSummary(entries)} as a patch`
                      }),
                    refresh: scm.reload,
                    compare: onCompare,
                    history: onShowHistory,
                  }),
                )
              }}
              busy={scm.busy}
            />
          )}
        </div>

        <CommitBox host={host} root={root} conversationId={conversationId} scm={scm} messageRef={messageRef} />
        {menu.element}

        <RollbackDialog
          entries={rollbackEntries}
          busy={scm.busy}
          onCancel={() => setRollbackEntries(null)}
          onConfirm={(entries, options) => {
            setRollbackEntries(null)
            void scm.rollback(entries, options)
          }}
        />
        <ConfirmDialog
          open={deleteEntry !== null}
          onOpenChange={(open) => (open ? undefined : setDeleteEntry(null))}
          title={deleteEntry ? `Delete ${basename(deleteEntry.path)}?` : 'Delete file?'}
          description={
            deleteEntry?.status === 'untracked'
              ? "It was never committed: this can't be undone."
              : 'The deletion shows as a change until it is committed or rolled back.'
          }
          details={deleteEntry ? [deleteEntry.path] : undefined}
          confirmLabel="Delete"
          tone="danger"
          onCancel={() => setDeleteEntry(null)}
          onConfirm={() => {
            const entry = deleteEntry
            setDeleteEntry(null)
            if (!entry) return
            void scm.run('delete', async () => {
              await onDeleteFile(entry.path)
              return `deleted ${basename(entry.path)}`
            })
          }}
        />
        <TextDialog
          open={stashEntries !== null}
          title={stashEntries ? `Stash ${plural(stashEntries.length, 'file', 'files')}` : 'Stash files'}
          description={
            stashEntries
              ? `Sets ${changeSummary(stashEntries)} aside in a new stash; the rest of the working tree stays as it is.`
              : undefined
          }
          label="Message"
          placeholder="What these changes are"
          allowEmpty
          confirmLabel="Stash"
          onCancel={() => setStashEntries(null)}
          onConfirm={(message) => {
            const entries = stashEntries
            setStashEntries(null)
            if (entries) void scm.stash(entries, message)
          }}
        />
      </div>
    </>
  )
}
