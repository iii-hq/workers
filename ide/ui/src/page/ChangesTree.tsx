/* The Commit panel's tree: groups, folders and files, each with a tick box
   that includes it in the next commit (a group or folder ticks everything
   under it and shows a dash when only part is ticked). The Rollback dialog
   renders the same rows to pick what to undo; a comparison (Show Diff with
   Working Tree) renders them without the tick boxes. */

import { Checkbox, IconButton } from '@iii-dev/console-ui'
import { Archive, ChevronDown, ChevronRight, Folder, FolderOpen, Undo2 } from 'lucide-react'
import type { ReactNode } from 'react'
import type { ChangeRow, TreeEntry } from './commit-tree'
import { FileTypeIcon } from './file-type-icon'
import type { GitComparisonEntry } from './git'
import { statusLetter, statusTitle } from './git-actions'
import { basename } from './paths'

type Tick = 'on' | 'off' | 'mixed'

export function tickState<T extends TreeEntry>(entries: readonly T[], isIncluded: (entry: T) => boolean): Tick {
  const ticked = entries.filter(isIncluded).length
  return ticked === 0 ? 'off' : ticked === entries.length ? 'on' : 'mixed'
}

interface ChangesTreeProps<T extends TreeEntry> {
  rows: readonly ChangeRow<T>[]
  /** With both, each row has a tick box. */
  isIncluded?: (entry: T) => boolean
  onInclude?: (entries: readonly T[], included: boolean) => void
  onToggleOpen: (key: string) => void
  /** The file whose diff is in front. */
  activePath?: string | null
  /** Click previews the file's diff; double click keeps the tab. */
  onOpen?: (entry: T, pin: boolean) => void
  /** Hover action on tracked rows. */
  onRollback?: (entries: readonly T[]) => void
  /** Hover action: stash these files. */
  onStash?: (entries: readonly T[]) => void
  busy?: boolean
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`
}

export function ChangesTree<T extends TreeEntry = GitComparisonEntry>({
  rows,
  isIncluded,
  onInclude,
  onToggleOpen,
  activePath = null,
  onOpen,
  onRollback,
  onStash,
  busy = false,
}: ChangesTreeProps<T>) {
  const ticks = isIncluded !== undefined && onInclude !== undefined
  return (
    <div className="shui-ctree">
      {rows.map((row) => {
        const indent = { paddingLeft: 4 + row.depth * 14 }
        if (row.kind === 'file') {
          const { entry } = row
          const name = basename(entry.path)
          const rollback = onRollback && entry.status !== 'untracked' ? () => onRollback([entry]) : null
          return (
            <Row
              key={row.key}
              kind="file"
              style={indent}
              selected={activePath === entry.path}
              status={entry.status}
              tick={ticks ? (isIncluded(entry) ? 'on' : 'off') : null}
              tickLabel={`Include ${name}`}
              onTick={(on) => onInclude?.([entry], on)}
              busy={busy}
              caret={<span className="shui-ctree-caret" aria-hidden />}
              main={
                <button
                  type="button"
                  className="shui-ctree-main"
                  title={entry.renameFrom ? `${entry.renameFrom} → ${entry.path}` : entry.path}
                  aria-current={activePath === entry.path || undefined}
                  onClick={() => onOpen?.(entry, false)}
                  onDoubleClick={() => onOpen?.(entry, true)}
                >
                  <FileTypeIcon path={entry.path} className="file-icon" />
                  <span className="name">{name}</span>
                  {row.dir ? <span className="meta">{row.dir}</span> : null}
                  {entry.renameFrom ? <span className="meta">from {basename(entry.renameFrom)}</span> : null}
                </button>
              }
              actions={
                rollback || onStash ? (
                  <>
                    {onStash ? (
                      <IconButton label={`Stash ${name}`} disabled={busy} onClick={() => onStash([entry])}>
                        <Archive aria-hidden />
                      </IconButton>
                    ) : null}
                    {rollback ? (
                      <IconButton label={`Rollback ${name}`} disabled={busy} onClick={rollback}>
                        <Undo2 aria-hidden />
                      </IconButton>
                    ) : null}
                  </>
                ) : null
              }
              letter={
                <span className="shui-ctree-status" data-status={entry.status} title={statusTitle(entry.status)}>
                  {statusLetter(entry.status)}
                </span>
              }
            />
          )
        }
        const Chevron = row.open ? ChevronDown : ChevronRight
        const FolderIcon = row.open ? FolderOpen : Folder
        const tracked = row.entries.filter((entry) => entry.status !== 'untracked')
        const rollback = onRollback && tracked.length > 0 && row.group === 'changes' ? () => onRollback(tracked) : null
        const toggle = () => onToggleOpen(row.key)
        return (
          <Row
            key={row.key}
            kind={row.kind}
            style={indent}
            tick={ticks ? tickState(row.entries, isIncluded) : null}
            tickLabel={`Include ${row.label}`}
            onTick={(on) => onInclude?.(row.entries, on)}
            busy={busy}
            caret={
              <button
                type="button"
                className="shui-ctree-caret"
                aria-label={`${row.open ? 'Collapse' : 'Expand'} ${row.label}`}
                aria-expanded={row.open}
                onClick={toggle}
              >
                <Chevron aria-hidden />
              </button>
            }
            main={
              <button
                type="button"
                className="shui-ctree-main"
                title={row.kind === 'folder' ? row.path : row.label}
                onClick={toggle}
              >
                {row.kind === 'folder' ? <FolderIcon aria-hidden className="folder-icon" /> : null}
                <span className="name">{row.label}</span>
                <span className="meta">{plural(row.entries.length, 'file', 'files')}</span>
              </button>
            }
            actions={
              rollback || onStash ? (
                <>
                  {onStash ? (
                    <IconButton
                      label={`Stash ${plural(row.entries.length, 'file', 'files')} in ${row.label}`}
                      disabled={busy}
                      onClick={() => onStash(row.entries)}
                    >
                      <Archive aria-hidden />
                    </IconButton>
                  ) : null}
                  {rollback ? (
                    <IconButton
                      label={`Rollback ${plural(tracked.length, 'file', 'files')} in ${row.label}`}
                      disabled={busy}
                      onClick={rollback}
                    >
                      <Undo2 aria-hidden />
                    </IconButton>
                  ) : null}
                </>
              ) : null
            }
          />
        )
      })}
    </div>
  )
}

function Row({
  kind,
  style,
  selected = false,
  status,
  tick,
  tickLabel,
  onTick,
  busy,
  caret,
  main,
  actions,
  letter,
}: {
  kind: ChangeRow<TreeEntry>['kind']
  style: React.CSSProperties
  selected?: boolean
  status?: GitComparisonEntry['status']
  /** null: no tick box. */
  tick: Tick | null
  tickLabel: string
  onTick: (on: boolean) => void
  busy: boolean
  caret: ReactNode
  main: ReactNode
  actions?: ReactNode
  letter?: ReactNode
}) {
  return (
    <div
      className="shui-ctree-row"
      data-kind={kind}
      data-selected={selected || undefined}
      data-status={status}
      style={style}
    >
      {caret}
      {tick === null ? null : (
        <Checkbox
          className="shui-ctree-tick"
          checked={tick === 'on'}
          indeterminate={tick === 'mixed'}
          disabled={busy}
          aria-label={tickLabel}
          onChange={() => onTick(tick !== 'on')}
        />
      )}
      {main}
      {actions ? <span className="shui-ctree-actions">{actions}</span> : null}
      {letter}
    </div>
  )
}
