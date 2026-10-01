/* The Git window's Worktrees tab, laid out like WebStorm's. Each worktree
   is a row, its folder over its branch, and the one the IDE is in has a
   check. A wide list adds columns: the last commit, its date and the
   marks, with the row's own actions on hover.

   A click selects a row, and a double click or Enter opens it: the IDE
   and the chat move there. The rail on the left and the row's menu act on
   the selection. The new-worktree and merge forms open above the list,
   never inside it: a listbox holds options only. */

import { EmptyState, Skeleton } from '@iii-dev/console-ui'
import {
  ArrowDown,
  ArrowUp,
  Check,
  Copy,
  FolderGit2,
  FolderInput,
  GitBranch,
  GitGraph,
  GitMerge,
  Plus,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import { useId, useState } from 'react'
import { ActionRail, type GitAction, menuItems } from './ActionRail'
import { glyphColor, glyphOf } from './CommitGraph'
import { useContextMenu } from './ContextMenu'
import { basename } from './paths'
import { speedMarks, useRowNav } from './use-row-nav'
import type { WorktreeOps } from './use-worktree-ops'
import { type MergeDraft, MergeForm, mergeable, NewWorktreeForm, worktreeMarks } from './WorktreeForms'
import { branchOf, type Worktree, worktreeAt } from './worktrees'

const day = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })
const full = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })
/** The actions a wide row shows on hover and when selected. */
const ROW_ACTIONS = new Set(['open', 'merge', 'remove'])

function Marked({ text, query }: { text: string; query: string }) {
  return (
    <>
      {speedMarks(text, query).map((part, index) =>
        part.hit ? (
          <mark key={index} className="shui-git-hit">
            {part.text}
          </mark>
        ) : (
          part.text
        ),
      )}
    </>
  )
}

export function GitWorktreesTab({
  root,
  ops,
  narrow = false,
  onShowInLog,
}: {
  root: string
  ops: WorktreeOps
  /** A narrow pane: the actions sit in a bar along the bottom. */
  narrow?: boolean
  /** Moves to the Log tab with `branch` selected there. */
  onShowInLog?(branch: string): void
}) {
  const { list, busy } = ops
  const target = list?.defaultBranch ?? null
  const items = list?.worktrees ?? []
  const here = list === null ? null : worktreeAt(list.worktrees, root)
  const [picked, setPicked] = useState<string | null>(null)
  // Nothing picked yet: the worktree the IDE is in.
  const selected = picked ?? here?.path ?? null
  const [creating, setCreating] = useState(false)
  const [merging, setMerging] = useState<MergeDraft | null>(null)
  const menu = useContextMenu()
  const domId = useId()
  const mergingWorktree = merging === null ? null : (items.find((wt) => wt.path === merging.path) ?? null)

  const openMerge = (wt: Worktree) => {
    setMerging({ path: wt.path, squash: true, message: '' })
    void ops
      .mergeMessage(wt)
      .then((message) =>
        setMerging((draft) => (draft?.path === wt.path && draft.message === '' ? { ...draft, message } : draft)),
      )
  }

  const actionsFor = (wt: Worktree | null): GitAction[] => {
    const busyWhy = busy ? 'another worktree operation is running' : null
    const none = wt === null ? 'select a worktree first' : null
    const branch = wt === null ? null : branchOf(wt)
    return [
      {
        id: 'new',
        label: `New worktree from ${target ?? 'HEAD'}`,
        short: 'New',
        icon: <Plus aria-hidden />,
        blocked: list === null ? 'reading the worktrees' : busyWhy,
        run: () => {
          setMerging(null)
          setCreating(true)
        },
      },
      {
        id: 'open',
        label: 'Open',
        icon: <FolderInput aria-hidden />,
        shortcut: 'Enter',
        blocked:
          none ??
          (wt?.path === here?.path
            ? 'the IDE is in it'
            : wt?.prunable
              ? 'its folder is gone'
              : wt?.bare
                ? 'a bare repository has no files'
                : busyWhy),
        run: () => {
          if (wt !== null) ops.switchTo(wt)
        },
      },
      {
        id: 'merge',
        label: `Merge into ${target ?? 'the default branch'}`,
        short: 'Merge',
        icon: <GitMerge aria-hidden />,
        blocked:
          none ??
          (wt !== null && !mergeable(wt, target)
            ? branch === null
              ? 'it has no branch'
              : branch === target
                ? `it is ${target}`
                : 'the main worktree has nothing to merge'
            : busyWhy),
        run: () => {
          if (wt === null) return
          setCreating(false)
          openMerge(wt)
        },
      },
      {
        id: 'log',
        label: 'Show in Log',
        icon: <GitGraph aria-hidden />,
        in: 'menu',
        blocked: none ?? (branch === null ? 'it has no branch' : onShowInLog ? null : 'no log here'),
        run: () => {
          if (branch !== null) onShowInLog?.(branch)
        },
      },
      {
        id: 'copy',
        label: 'Copy path',
        icon: <Copy aria-hidden />,
        in: 'menu',
        blocked: none,
        run: () => {
          if (wt !== null) void navigator.clipboard?.writeText(wt.path)
        },
      },
      {
        id: 'remove',
        label: wt?.prunable ? 'Prune' : 'Remove',
        icon: <Trash2 aria-hidden />,
        shortcut: 'Delete',
        danger: true,
        blocked: none ?? (wt?.main || wt?.bare ? 'the main worktree stays' : busyWhy),
        run: () => {
          if (wt !== null) ops.askRemove(wt)
        },
      },
      {
        id: 'refresh',
        label: 'Refresh',
        icon: <RefreshCw aria-hidden />,
        in: 'rail',
        end: true,
        blocked: busyWhy,
        run: ops.reload,
      },
    ]
  }

  const run = (wt: Worktree, id: string) => {
    const action = actionsFor(wt).find((candidate) => candidate.id === id)
    if (action && (action.blocked ?? null) === null) action.run()
  }
  const nav = useRowNav<Worktree>({
    items,
    idOf: (wt) => wt.path,
    labelOf: (wt) => `${basename(wt.path)} ${branchOf(wt) ?? ''}`,
    domId,
    selected,
    onSelect: setPicked,
    onAct: (wt) => run(wt, 'open'),
    onDelete: (wt) => run(wt, 'remove'),
    onMenu: (wt, anchor) => menu.open(anchor, menuItems(actionsFor(wt))),
  })
  const current = items.find((wt) => wt.path === selected) ?? null
  const repo = basename(list?.worktrees[0]?.path ?? root)

  return (
    <div className="shui-git-pane" data-pane="worktrees">
      {narrow ? null : <ActionRail label="Worktree actions" actions={actionsFor(current)} />}
      <div className="shui-git-column">
        {creating && list !== null ? (
          <div className="shui-git-form">
            <NewWorktreeForm
              target={target}
              busy={busy}
              onCreate={(branch) => ops.create(branch, () => setCreating(false))}
              onCancel={() => setCreating(false)}
            />
          </div>
        ) : null}
        {merging !== null && mergingWorktree !== null && target !== null ? (
          <div className="shui-git-form">
            <p className="shui-git-form-title">
              Merge {branchOf(mergingWorktree)} into {target}
            </p>
            <MergeForm
              draft={merging}
              target={target}
              busy={busy}
              onChange={setMerging}
              onMerge={() => ops.merge(mergingWorktree, merging, () => setMerging(null))}
              onCancel={() => setMerging(null)}
            />
          </div>
        ) : null}
        {ops.error !== null ? (
          ops.error.toLowerCase().includes('not a git repository') ? (
            <EmptyState compact title="No repository" description="This folder is not inside a Git repository." />
          ) : (
            <p className="shui-git-note-line warn">{ops.error}</p>
          )
        ) : list === null ? (
          <div className="shui-git-list" aria-busy="true">
            {[0, 1, 2].map((row) => (
              <Skeleton key={row} className="shui-git-skeleton" />
            ))}
          </div>
        ) : (
          <>
            <div className="shui-git-wt shui-git-wt-head" aria-hidden>
              <span />
              <span />
              <span>Worktree</span>
              <span>Last commit</span>
              <span>Date</span>
              <span>Status</span>
              <span />
            </div>
            <div
              role="listbox"
              aria-label={`Worktrees of ${repo}`}
              className="shui-git-list"
              data-git-focus=""
              {...nav.listProps}
            >
              {items.map((wt, index) => {
                const isHere = wt.path === here?.path
                const branch = branchOf(wt)
                const { marks, titles } = worktreeMarks(wt, target)
                const quiet = marks.filter((mark) => mark !== '●')
                const status =
                  quiet.length === 0 ? null : (
                    <span className="shui-git-marks">
                      {quiet.map((mark) => {
                        // ↑n / ↓n: ahead of / behind the default branch
                        const Arrow = mark.startsWith('↑') ? ArrowUp : mark.startsWith('↓') ? ArrowDown : null
                        return Arrow === null ? (
                          <span key={mark}>{mark}</span>
                        ) : (
                          <span key={mark} className="shui-git-mark">
                            <Arrow aria-hidden />
                            {mark.slice(1)}
                            <span className="shui-sr-only">
                              {mark.startsWith('↑') ? ' ahead of ' : ' behind '}
                              {target}
                            </span>
                          </span>
                        )
                      })}
                    </span>
                  )
                return (
                  // biome-ignore lint/a11y/useFocusableInteractive: the listbox holds focus and names this row through aria-activedescendant
                  <div
                    key={wt.path}
                    role="option"
                    className="shui-git-wt"
                    aria-current={isHere ? 'true' : undefined}
                    data-gone={wt.prunable || undefined}
                    data-held={wt.held ? '' : undefined}
                    title={titles.join('\n')}
                    {...nav.rowProps(index)}
                  >
                    <span className="shui-git-wt-check">{isHere ? <Check aria-hidden /> : null}</span>
                    <FolderGit2
                      aria-hidden
                      className="shui-git-wt-icon"
                      style={{ color: glyphColor(glyphOf(branch, target)) }}
                    />
                    <span className="shui-git-wt-text">
                      <span className="shui-git-wt-folder">
                        <Marked text={basename(wt.path)} query={nav.query} />
                      </span>
                      <span className="shui-git-wt-branch">
                        <GitBranch aria-hidden />
                        <span className="shui-git-wt-name">
                          {branch !== null ? (
                            <Marked text={branch} query={nav.query} />
                          ) : wt.bare ? (
                            'bare'
                          ) : (
                            `detached${wt.head ? ` · ${wt.head.slice(0, 7)}` : ''}`
                          )}
                        </span>
                        {wt.dirty ? <span className="shui-wt-row-dirty" /> : null}
                        {status}
                      </span>
                    </span>
                    {/* the columns a wide list adds; a narrow one keeps the two lines */}
                    <span className="shui-git-wt-subject">{wt.tip?.subject}</span>
                    <span className="shui-git-wt-date" title={wt.tip ? full.format(wt.tip.date * 1000) : undefined}>
                      {wt.tip ? day.format(wt.tip.date * 1000) : null}
                    </span>
                    <span className="shui-git-wt-status">
                      {wt.dirty ? (
                        <span className="shui-git-wt-edited">
                          <span className="shui-wt-row-dirty" />
                          edited
                        </span>
                      ) : null}
                      {status ?? (wt.dirty || wt.ahead === undefined ? null : 'up to date')}
                    </span>
                    {/* the row's actions for a mouse; the rail, keys and menu hold them all */}
                    <span className="shui-git-wt-actions" aria-hidden>
                      {actionsFor(wt)
                        .filter((action) => ROW_ACTIONS.has(action.id) && (action.blocked ?? null) === null)
                        .map((action) => (
                          <button
                            key={action.id}
                            type="button"
                            tabIndex={-1}
                            title={action.label}
                            className="shui-git-rail-action"
                            data-tone={action.danger ? 'alert' : undefined}
                            onClick={action.run}
                            onDoubleClick={(event) => event.stopPropagation()}
                          >
                            {action.icon}
                          </button>
                        ))}
                    </span>
                  </div>
                )
              })}
            </div>
          </>
        )}
        {nav.query !== '' ? <span className="shui-git-speed">{nav.query}</span> : null}
      </div>
      {narrow ? (
        <ActionRail
          label="Worktree actions"
          actions={actionsFor(current)}
          bar
          onMore={(anchor, rest) => menu.open(anchor, menuItems(rest, true))}
        />
      ) : null}
      {menu.element}
    </div>
  )
}
