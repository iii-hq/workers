/**
 * What the main pane shows when nothing is open: the wordmark, the folder
 * this pane looks at (the chat composer's own picker, so changing it is
 * one click), one door per surface of the page with its key and what is
 * behind it right now (how many changes, how many turns, the last turn's
 * name), and the files this pane opened last. An empty editor is a dead
 * end; this is the whole page in one glance.
 */

import { DirectoryPicker, Wordmark } from '@iii-dev/console-ui'
import { FileSearch, FolderOpen, GitCompareArrows, History, Search, SquareTerminal } from 'lucide-react'
import type { ComponentType } from 'react'
import { FileTypeIcon } from './file-type-icon'
import type { GitState } from './git'
import { basename, dirname } from './paths'
import { type SessionTurnSummary, turnTitle } from './turns'

interface LauncherCard {
  id: string
  title: string
  detail: string
  /** The key that does the same while the pane has the focus. */
  shortcut?: string
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  onSelect: () => void
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
  onOpenTimeline: () => void
  onOpenTerminal: () => void
  onOpenFiles: () => void
}

function plural(count: number, noun: string): string {
  return `${count} ${count === 1 ? noun : `${noun}s`}`
}

/** One line on the state of the index, for the Source control card. */
export function sourceControlDetail(git: GitState | null): string {
  if (git === null) return 'reading the repository'
  if (git.kind === 'not-a-repo') return 'not a git repository'
  if (git.kind === 'error') return 'git is unavailable here'
  if (git.changes.length === 0) return 'working tree clean'
  const staged = git.changes.filter((change) => change.staged).length
  return staged === 0 ? `${plural(git.changes.length, 'change')}, nothing staged` : `${plural(git.changes.length, 'change')}, ${staged} staged`
}

/** One line on the chat's turns, for the Timeline card. */
export function timelineDetail(turns: readonly SessionTurnSummary[], hasSession: boolean, running: boolean): string {
  if (!hasSession) return 'open beside a chat to follow its turns'
  if (turns.length === 0) return running ? 'a turn is running' : 'no turn has changed files yet'
  const last = turnTitle(turns[0], turns.length)
  return `${plural(turns.length, 'turn')}${running ? ', one running' : ''}. Last: ${last}`
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
  onOpenTimeline,
  onOpenTerminal,
  onOpenFiles,
}: ShellLauncherProps) {
  const folder = root?.split('/').filter(Boolean).at(-1) ?? 'this folder'
  const cards: LauncherCard[] = [
    {
      id: 'open',
      title: 'Open a file',
      detail: 'Find any file by name',
      shortcut: 'P',
      Icon: FileSearch,
      onSelect: onQuickOpen,
    },
    {
      id: 'search',
      title: 'Search in files',
      detail: 'Text across the folder, with context around each hit',
      shortcut: 'F',
      Icon: Search,
      onSelect: onSearch,
    },
    {
      id: 'browse',
      title: 'Browse the folder',
      detail: 'The tree, with a context menu on every file and folder',
      shortcut: 'E',
      Icon: FolderOpen,
      onSelect: onOpenFiles,
    },
    {
      id: 'scm',
      title: 'Source control',
      detail: sourceControlDetail(git),
      shortcut: 'S',
      Icon: GitCompareArrows,
      onSelect: onOpenChanges,
    },
    {
      id: 'timeline',
      title: 'Timeline',
      detail: timelineDetail(turns, hasSession, turnRunning),
      shortcut: 'H',
      Icon: History,
      onSelect: onOpenTimeline,
    },
    {
      id: 'terminal',
      title: 'Terminal',
      detail: `A shell in ${folder}`,
      shortcut: '`',
      Icon: SquareTerminal,
      onSelect: onOpenTerminal,
    },
  ]

  return (
    <div className="shui-main-empty">
      <div className="shui-launcher">
        <Wordmark appearance="inset" className="shui-launcher-mark" />
        <h2 className="shui-launcher-title">
          What are we working on in{' '}
          <DirectoryPicker
            value={pendingRoot ?? root}
            onChange={onChangeRoot}
            defaultDir={defaultRoot}
            externalError={rootError}
            triggerAppearance="inline"
            emptyLabel="a folder"
            className="shui-launcher-picker"
          />
          ?
        </h2>
        {root ? (
          <p className="shui-launcher-path" title={pendingRoot ?? root}>
            {pendingRoot !== null ? `opening ${pendingRoot}` : root}
          </p>
        ) : null}
        <div className="shui-launcher-grid">
          {cards.map(({ id, title, detail, shortcut, Icon, onSelect }) => (
            <button key={id} type="button" className="shui-launcher-card" onClick={onSelect}>
              <span className="shui-launcher-card-head">
                <Icon aria-hidden className="shui-launcher-icon" />
                {shortcut ? (
                  <kbd className="shui-launcher-key" title={`Press ${shortcut} while this pane has the focus`}>
                    {shortcut}
                  </kbd>
                ) : null}
              </span>
              <span className="shui-launcher-card-title">{title}</span>
              <span className="shui-launcher-card-detail">{detail}</span>
            </button>
          ))}
        </div>
        {recent.length > 0 ? (
          <section className="shui-launcher-recent" aria-label="Recently opened">
            <h3 className="shui-launcher-recent-title">Recently opened</h3>
            <div className="shui-launcher-recent-list">
              {recent.map((path) => (
                <button key={path} type="button" className="shui-launcher-recent-row" title={path} onClick={() => onOpenFile(path)}>
                  <FileTypeIcon path={path} className="file-icon" />
                  <span className="name">{basename(path)}</span>
                  {dirname(path) ? <span className="dir">{dirname(path)}</span> : null}
                </button>
              ))}
            </div>
          </section>
        ) : null}
        <p className="shui-launcher-hint">
          Keys work while this pane has the focus. <kbd className="shui-launcher-key">Shift+Alt+←</kbd>{' '}
          <kbd className="shui-launcher-key">Shift+Alt+→</kbd> walk the files you visited, <kbd className="shui-launcher-key">W</kbd>{' '}
          closes a tab.
        </p>
      </div>
    </div>
  )
}
