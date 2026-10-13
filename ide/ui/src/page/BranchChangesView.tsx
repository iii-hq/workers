/* "Show Diff with Working Tree" as a sidebar view: the files
   that differ between a branch (a tag) and the working tree, staged or not,
   as a tree under the repository with a count on every folder. Swap
   branches turns every diff the other way. A click opens a file's diff, a
   double click keeps its tab. */

import type { Host } from '@iii-dev/console-ui'
import { EmptyState, IconButton, Skeleton } from '@iii-dev/console-ui'
import { ChevronsDownUp, ChevronsUpDown, RefreshCw, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ChangesTree } from './ChangesTree'
import { type ChangeGroup, changeRows, expandableKeys, rowKey } from './commit-tree'
import type { DiffSource } from './diff-source'
import { git } from './git-actions'
import { type CommitFile, shortRef } from './git-log-window'
import { basename } from './paths'
import { useWorkingDiff } from './use-git-log'

/** Past this many files, folders start closed. */
const OPEN_UP_TO = 200

/** A file as the tree knows it: by its path in the repository. */
interface ComparedFile {
  path: string
  status: CommitFile['status']
  renameFrom?: string
  file: CommitFile
}

// Swapped, what the branch added the working tree lacks, and the other way.
const SWAPPED: Partial<Record<CommitFile['status'], CommitFile['status']>> = { added: 'deleted', deleted: 'added' }

/** The IDE's folder below the repository's top, the top, and the branch. */
function useWhere(host: Host, root: string, refreshKey: unknown) {
  const [where, setWhere] = useState<{ prefix: string; top: string; branch: string | null } | null>(null)
  useEffect(() => {
    let live = true
    void git(host, root, ['rev-parse', '--show-prefix', '--show-toplevel', '--abbrev-ref', 'HEAD']).then((out) => {
      const [prefix = '', top = '', branch = 'HEAD'] = out.stdout.split('\n')
      if (live && out.exit_code === 0) setWhere({ prefix, top, branch: branch === 'HEAD' ? null : branch })
    })
    return () => {
      live = false
    }
  }, [host, root, refreshKey])
  return where
}

export function BranchChangesView({
  host,
  root,
  refName,
  refreshKey,
  activeView,
  onOpenDiff,
  onClose,
}: {
  host: Host
  root: string
  /** The full ref compared with the working tree. */
  refName: string
  refreshKey: unknown
  /** The open diff's path, as the page names it, to mark its row. */
  activeView: string | null
  onOpenDiff(view: string, source: DiffSource, pin: boolean): void
  onClose(): void
}) {
  const [reloads, setReloads] = useState(0)
  const where = useWhere(host, root, refreshKey)
  const state = useWorkingDiff(host, root, where?.prefix ?? null, refName, `${String(refreshKey)}:${reloads}`)
  const [swapped, setSwapped] = useState(false)
  const [open, setOpen] = useState<ReadonlyMap<string, boolean>>(() => new Map())
  const label = shortRef(refName)
  const repo = where === null ? '' : basename(where.top)

  const files = useMemo<ComparedFile[]>(
    () =>
      (state.files ?? []).map((file) => ({
        path: file.path,
        status: swapped ? (SWAPPED[file.status] ?? file.status) : file.status,
        renameFrom: file.from,
        file,
      })),
    [state.files, swapped],
  )
  const groups = useMemo<ChangeGroup<ComparedFile>[]>(
    () => [{ id: 'changes', label: repo, entries: files }],
    [repo, files],
  )
  const many = files.length > OPEN_UP_TO
  const rows = useMemo(
    () =>
      changeRows(groups, {
        byDirectory: true,
        isOpen: (key, fallback) => open.get(key) ?? (key === rowKey('changes') || (!many && fallback)),
      }),
    [groups, open, many],
  )
  const setAllOpen = (value: boolean) => setOpen(new Map(expandableKeys(groups).map((key) => [key, value])))
  const activePath = files.find((entry) => entry.file.view === activeView)?.path ?? null
  const sourceOf = (file: CommitFile): DiffSource => ({
    type: 'compare',
    ref: refName,
    ...(file.from ? { from: file.from } : {}),
    ...(swapped ? { reverse: true as const } : {}),
  })
  const current = where?.branch ?? 'HEAD'

  return (
    <section className="shui-bchanges" aria-label={`Changes between ${label} and the working tree`}>
      <div className="shui-bchanges-head">
        <span className="shui-bchanges-title" title={`Changes between ${label} and the working tree`}>
          Changes Between {label} and Working Tree
        </span>
        <IconButton label="Close" variant="ghost" onClick={onClose}>
          <X size={16} aria-hidden />
        </IconButton>
      </div>
      <p className="shui-bchanges-desc">
        {swapped ? (
          <>
            Difference between files in <code>{label}</code> and current working tree on <code>{current}</code>:
          </>
        ) : (
          <>
            Difference between current working tree on <code>{current}</code> and files in <code>{label}</code>:
          </>
        )}{' '}
        <button type="button" className="shui-bchanges-swap" onClick={() => setSwapped((value) => !value)}>
          Swap branches
        </button>
      </p>
      <div className="shui-bchanges-tools">
        <IconButton label="Refresh" variant="ghost" onClick={() => setReloads((count) => count + 1)}>
          <RefreshCw size={16} aria-hidden />
        </IconButton>
        <span className="shui-bchanges-spacer" />
        <IconButton label="Expand all" variant="ghost" disabled={files.length === 0} onClick={() => setAllOpen(true)}>
          <ChevronsUpDown size={16} aria-hidden />
        </IconButton>
        <IconButton
          label="Collapse all"
          variant="ghost"
          disabled={files.length === 0}
          onClick={() => setAllOpen(false)}
        >
          <ChevronsDownUp size={16} aria-hidden />
        </IconButton>
      </div>
      <div className="shui-bchanges-body">
        {state.error !== null ? (
          <p className="shui-side-note">{state.error}</p>
        ) : state.files === null || where === null ? (
          [0, 1, 2].map((row) => <Skeleton key={row} className="shui-git-skeleton" />)
        ) : files.length === 0 ? (
          <EmptyState compact title="No differences" description={`The working tree matches ${label}.`} />
        ) : (
          <>
            <ChangesTree<ComparedFile>
              rows={rows}
              activePath={activePath}
              onToggleOpen={(key) =>
                setOpen((previous) => {
                  const row = rows.find((candidate) => candidate.key === key)
                  const wasOpen = row !== undefined && row.kind !== 'file' ? row.open : false
                  return new Map(previous).set(key, !wasOpen)
                })
              }
              onOpen={(entry, pin) => onOpenDiff(entry.file.view, sourceOf(entry.file), pin)}
            />
            {state.truncated ? <p className="shui-side-note">More files differ than the shell output holds.</p> : null}
          </>
        )}
      </div>
    </section>
  )
}
