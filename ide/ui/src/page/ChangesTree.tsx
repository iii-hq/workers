/* The Commit panel's tree: groups, folders and files, each with a tick box
   that includes it in the next commit (a group or folder ticks everything
   under it and shows a dash when only part is ticked). The Rollback dialog
   renders the same rows to pick what to undo; a comparison (Show Diff with
   Working Tree) renders them without the tick boxes. */

import { Checkbox, IconButton } from '@iii-dev/console-ui'
import { Archive, ChevronDown, ChevronRight, Folder, FolderOpen, Undo2 } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { anchorFromEvent, type ContextMenuAnchor } from './ContextMenu'
import type { ChangeRow, TreeEntry } from './commit-tree'
import { FileTypeIcon } from './file-type-icon'
import type { GitComparisonEntry } from './git'
import { statusLetter, statusTitle } from './git-actions'
import { basename } from './paths'
import { VirtualList } from './VirtualList'

type Tick = 'on' | 'off' | 'mixed'

/** `.shui-ctree-row`'s height in styles.css. */
export const ROW_HEIGHT = 28

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
  /** Right click (or the menu key) on a row. */
  onMenu?: (row: ChangeRow<T>, anchor: ContextMenuAnchor) => void
  /** The row whose menu is open: it and the rows it acts on stand out. */
  menuTarget?: string | null
  busy?: boolean
}

/** Whether `key` is a row under the menu's target row: everything in a
    group (`changes`), or below a folder (`changes:src/`). */
export function inMenuScope(key: string, target: string | null): boolean {
  if (target === null || key === target) return false
  if (target.endsWith('/')) return key.startsWith(target)
  return !target.includes(':') && key.startsWith(`${target}:`)
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
  onMenu,
  menuTarget = null,
  busy = false,
}: ChangesTreeProps<T>) {
  const ticks = isIncluded !== undefined && onInclude !== undefined
  // Windowed: thousands of unversioned files mount only the rows in view.
  // The row holding focus stays mounted when it scrolls out, so the keys
  // keep scrolling and focus does not drop to the page.
  const [focusedKey, setFocusedKey] = useState<string | null>(null)
  const focusedIndex = focusedKey === null ? null : rows.findIndex((row) => row.key === focusedKey)
  return (
    <VirtualList
      rows={rows}
      rowHeight={ROW_HEIGHT}
      rowKey={(row) => row.key}
      className="shui-ctree"
      keepIndex={focusedIndex}
      renderRow={(row) => {
        const indent = { paddingLeft: 4 + row.depth * 14 }
        // Under the row, at the pointer's x: the menu leaves the row it marks in sight.
        const menu = onMenu
          ? (event: React.MouseEvent) => {
              event.preventDefault()
              const { x } = anchorFromEvent(event)
              onMenu(row, { x, y: event.currentTarget.getBoundingClientRect().bottom })
            }
          : undefined
        const menuState = row.key === menuTarget ? 'target' : inMenuScope(row.key, menuTarget) ? 'scope' : undefined
        if (row.kind === 'file') {
          const { entry } = row
          const name = basename(entry.path)
          const rollback = onRollback && entry.status !== 'untracked' ? () => onRollback([entry]) : null
          return (
            <Row
              onFocusChange={(on) => setFocusedKey(on ? row.key : null)}
              onContextMenu={menu}
              menuState={menuState}
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
            onFocusChange={(on) => setFocusedKey(on ? row.key : null)}
            onContextMenu={menu}
            menuState={menuState}
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
      }}
    />
  )
}

function Row({
  onFocusChange,
  onContextMenu,
  menuState,
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
  onFocusChange: (focused: boolean) => void
  onContextMenu?: (event: React.MouseEvent) => void
  menuState?: 'target' | 'scope'
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
    // biome-ignore lint/a11y/noStaticElementInteractions: focus/blur bubble up from the row's own controls
    <div
      className="shui-ctree-row"
      data-kind={kind}
      data-selected={selected || undefined}
      data-menu={menuState}
      data-status={status}
      style={style}
      onContextMenu={onContextMenu}
      onFocus={() => onFocusChange(true)}
      onBlur={(event) => {
        // The browser window losing focus blurs the row but leaves focus in
        // it: keep it mounted for when the window comes back.
        const row = event.currentTarget
        if (!row.contains(event.relatedTarget) && !row.contains(document.activeElement)) onFocusChange(false)
      }}
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
