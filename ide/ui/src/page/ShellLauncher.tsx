/**
 * What the main pane shows when nothing is open. One centered composition
 * on the canvas: the mark, a short heading, the folder this pane looks at
 * (the chat composer's own picker, so changing it is one click) with its
 * path, then every surface of the page as a row in a framed list — its
 * name, what is behind it right now (how many changes, how many turns, the
 * last turn's name) — and the files this pane
 * opened last. An empty editor is a dead end; this is the whole page in
 * one glance.
 */

import { DirectoryPicker, Wordmark } from '@iii-dev/console-ui'
import { FileSearch, FolderOpen, GitCompareArrows, GitGraph, History, Search, SquareTerminal } from 'lucide-react'
import type { CSSProperties, ComponentType } from 'react'
import { FileTypeIcon } from './file-type-icon'
import type { GitState } from './git'
import { basename, dirname } from './paths'
import { quickOpenKeyLabel, quickOpenPlatform } from './quick-open'
import { type SessionTurnSummary, turnTitle } from './turns'

interface LauncherRow {
  id: string
  title: string
  detail: string
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  onSelect: () => void
}

interface LauncherGroup {
  id: string
  label: string
  rows: LauncherRow[]
}

interface ShellLauncherProps {
  root: string | null
  /** The folder being validated after a pick, shown until it resolves. */
  pendingRoot: string | null
  defaultRoot: string | null
  rootError: string | null
  onChangeRoot: (dir: string) => void
  git: GitState | null
  turns: readonly SessionTurnSummary[]
  hasSession: boolean
  turnRunning: boolean
  /** Root-relative paths this pane opened, most recent first. */
  recent: readonly string[]
  onOpenFile: (relPath: string) => void
  onQuickOpen: () => void
  onSearch: () => void
  onOpenChanges: () => void
  onOpenGit: () => void
  onOpenTimeline: () => void
  onOpenTerminal: () => void
  onOpenFiles: () => void
}

function plural(count: number, noun: string): string {
  return `${count} ${count === 1 ? noun : `${noun}s`}`
}

/** One line on the state of the index, for the Source control row. */
export function sourceControlDetail(git: GitState | null): string {
  if (git === null) return 'Reading the repository'
  if (git.kind === 'not-a-repo') return 'Not a git repository'
  if (git.kind === 'error') return 'Git is unavailable here'
  if (git.changes.length === 0) return 'Working tree clean'
  const staged = git.changes.filter((change) => change.staged).length
  return staged === 0 ? `${plural(git.changes.length, 'change')}, nothing staged` : `${plural(git.changes.length, 'change')}, ${staged} staged`
}

/** One line on the chat's turns, for the Timeline row. */
export function timelineDetail(turns: readonly SessionTurnSummary[], hasSession: boolean, running: boolean): string {
  if (!hasSession) return 'Open beside a chat to follow its turns'
  if (turns.length === 0) return running ? 'A turn is running' : 'No turn has changed files yet'
  const last = turnTitle(turns[0], turns.length)
  return `${plural(turns.length, 'turn')}${running ? ', one running' : ''}. Last: ${last}`
}

/** The row's place in the entrance stagger, read by the stylesheet. */
function staggerStyle(index: number): CSSProperties {
  return { '--shui-i': index } as CSSProperties
}

export function ShellLauncher({
  root,
  pendingRoot,
  defaultRoot,
  rootError,
  onChangeRoot,
  git,
  turns,
  hasSession,
  turnRunning,
  recent,
  onOpenFile,
  onQuickOpen,
  onSearch,
  onOpenChanges,
  onOpenGit,
  onOpenTimeline,
  onOpenTerminal,
  onOpenFiles,
}: ShellLauncherProps) {
  const folder = root?.split('/').filter(Boolean).at(-1) ?? 'this folder'
  const quickOpenKey = quickOpenKeyLabel(quickOpenPlatform())
  const groups: LauncherGroup[] = [
    {
      id: 'find',
      label: 'Find',
      rows: [
        {
          id: 'open',
          title: 'Go to file',
          detail: `Any part of a path, ${quickOpenKey} from anywhere`,
          Icon: FileSearch,
          onSelect: onQuickOpen,
        },
        {
          id: 'search',
          title: 'Search in files',
          detail: 'Text across the folder, in context',
          Icon: Search,
          onSelect: onSearch,
        },
        {
          id: 'browse',
          title: 'Browse the folder',
          detail: 'The tree, with a menu on every entry',
          Icon: FolderOpen,
          onSelect: onOpenFiles,
        },
      ],
    },
    {
      id: 'changes',
      label: 'Changes',
      rows: [
        {
          id: 'scm',
          title: 'Source control',
          detail: sourceControlDetail(git),
          Icon: GitCompareArrows,
          onSelect: onOpenChanges,
        },
        {
          id: 'git',
          title: 'Git',
          detail: 'Log, branches and worktrees',
          Icon: GitGraph,
          onSelect: onOpenGit,
        },
        {
          id: 'timeline',
          title: 'Timeline',
          detail: timelineDetail(turns, hasSession, turnRunning),
          Icon: History,
          onSelect: onOpenTimeline,
        },
      ],
    },
    {
      id: 'run',
      label: 'Run',
      rows: [
        {
          id: 'terminal',
          title: 'Terminal',
          detail: `A shell in ${folder}`,
          Icon: SquareTerminal,
          onSelect: onOpenTerminal,
        },
      ],
    },
  ]
  // Rows count up across the groups so the stagger runs down the page.
  let order = 0

  return (
    <div className="shui-main-empty">
      <div className="shui-launcher">
        <header className="shui-launcher-head">
          <Wordmark appearance="inset" className="shui-launcher-mark" />
          <h2 className="shui-launcher-title">What are we working on?</h2>
          <div className="shui-launcher-folder">
            <DirectoryPicker
              value={pendingRoot ?? root}
              onChange={onChangeRoot}
              defaultDir={defaultRoot}
              externalError={rootError}
              emptyLabel="Choose a folder"
              className="shui-launcher-picker"
            />
            {root ? (
              <span className="shui-launcher-path" title={pendingRoot ?? root}>
                {pendingRoot !== null ? `Opening ${pendingRoot}` : root}
              </span>
            ) : null}
          </div>
        </header>

        <div className="shui-launcher-groups">
          {groups.map(({ id, label, rows }) => (
            <section
              key={id}
              className="shui-launcher-group"
              aria-label={label}
            >
              <h3 className="shui-launcher-group-label">{label}</h3>
              <div className="shui-launcher-list">
                {rows.map(({ id: rowId, title, detail, Icon, onSelect }) => (
                  <button key={rowId} type="button" className="shui-launcher-row" style={staggerStyle(order++)} onClick={onSelect}>
                    <Icon aria-hidden className="shui-launcher-icon" />
                    <span className="shui-launcher-row-text">
                      <span className="shui-launcher-row-title">{title}</span>
                      <span className="shui-launcher-row-detail">{detail}</span>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          ))}

          {recent.length > 0 ? (
            <section className="shui-launcher-group" aria-label="Recently opened">
              <h3 className="shui-launcher-group-label">Recent</h3>
              <div className="shui-launcher-list">
                {recent.map((path) => (
                  <button
                    key={path}
                    type="button"
                    className="shui-launcher-row shui-launcher-recent-row"
                    style={staggerStyle(order++)}
                    title={path}
                    onClick={() => onOpenFile(path)}
                  >
                    <FileTypeIcon path={path} className="file-icon" />
                    <span className="name">{basename(path)}</span>
                    {dirname(path) ? <span className="dir">{dirname(path)}</span> : null}
                  </button>
                ))}
              </div>
            </section>
          ) : null}
        </div>

      </div>
    </div>
  )
}
