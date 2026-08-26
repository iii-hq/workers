import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FilePen,
  GitBranch,
  GitCommitHorizontal,
  History,
  Layers,
  ListChecks,
  MessagesSquare,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

export type ReviewScopeSelection =
  | { kind: 'last-turn' }
  | { kind: 'uncommitted' }
  | { kind: 'unstaged' }
  | { kind: 'staged' }
  | { kind: 'commit'; sha: string; subject: string }
  | { kind: 'branch'; ref: string; name: string }
  | { kind: 'turn'; turnId: string; label: string }

export interface ReviewTurnChoice {
  turnId: string
  label: string
  fileCount: number
  active: boolean
}

export interface ReviewCommitChoice {
  sha: string
  subject: string
}

export interface ReviewBranchChoice {
  ref: string
  name: string
  current: boolean
}

export type ReviewScopeCounts = Partial<Record<'last-turn' | 'uncommitted' | 'unstaged' | 'staged', number>>

export function reviewScopeLabel(scope: ReviewScopeSelection, currentTurn = false): string {
  switch (scope.kind) {
    case 'last-turn':
      return currentTurn ? 'Current Turn' : 'Last Turn'
    case 'uncommitted':
      return 'Uncommitted'
    case 'unstaged':
      return 'Unstaged'
    case 'staged':
      return 'Staged'
    case 'commit':
      return scope.subject || scope.sha.slice(0, 8)
    case 'branch':
      return scope.name
    case 'turn':
      return scope.label
  }
}

function scopeIcon(scope: ReviewScopeSelection) {
  switch (scope.kind) {
    case 'last-turn':
      return <History aria-hidden className="menu-icon" />
    case 'uncommitted':
      return <FilePen aria-hidden className="menu-icon" />
    case 'unstaged':
      return <ListChecks aria-hidden className="menu-icon" />
    case 'staged':
      return <Layers aria-hidden className="menu-icon" />
    case 'commit':
      return <GitCommitHorizontal aria-hidden className="menu-icon" />
    case 'branch':
      return <GitBranch aria-hidden className="menu-icon" />
    case 'turn':
      return <MessagesSquare aria-hidden className="menu-icon" />
  }
}

function scopeMatches(left: ReviewScopeSelection, right: ReviewScopeSelection): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === 'commit' && right.kind === 'commit') return left.sha === right.sha
  if (left.kind === 'branch' && right.kind === 'branch') return left.ref === right.ref
  if (left.kind === 'turn' && right.kind === 'turn') return left.turnId === right.turnId
  return true
}

export function ReviewScopePicker({
  value,
  commits,
  branches,
  turns = [],
  counts = {},
  currentTurn = false,
  metadataLoading,
  metadataError,
  onOpen,
  onChange,
}: {
  value: ReviewScopeSelection
  commits: readonly ReviewCommitChoice[]
  branches: readonly ReviewBranchChoice[]
  /** Stored turns of the chat this page follows, newest first. */
  turns?: readonly ReviewTurnChoice[]
  counts?: ReviewScopeCounts
  currentTurn?: boolean
  metadataLoading: boolean
  metadataError: string | null
  onOpen: () => void
  onChange: (scope: ReviewScopeSelection) => void
}) {
  const [open, setOpen] = useState(false)
  const [subMenu, setSubMenu] = useState<'commits' | 'branches' | 'turns' | null>(null)
  const wrapRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const dismiss = (event: PointerEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) {
        setOpen(false)
        setSubMenu(null)
      }
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      setOpen(false)
      setSubMenu(null)
    }
    window.addEventListener('pointerdown', dismiss)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('pointerdown', dismiss)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  const choose = (scope: ReviewScopeSelection) => {
    onChange(scope)
    setOpen(false)
    setSubMenu(null)
  }

  const workingTreeScopes = [
    { kind: 'uncommitted' },
    { kind: 'unstaged' },
    { kind: 'staged' },
  ] as const satisfies readonly ReviewScopeSelection[]

  return (
    <div ref={wrapRef} className="shui-review-scope-wrap">
      <button
        type="button"
        className="shui-review-scope-button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          const next = !open
          setOpen(next)
          setSubMenu(null)
          if (next) onOpen()
        }}
      >
        {scopeIcon(value)}
        <span>{reviewScopeLabel(value, currentTurn)}</span>
        <ChevronDown aria-hidden />
      </button>
      {open ? (
        <div className="shui-review-scope-menu" role="menu">
          {subMenu === null ? (
            <>
              <div className="shui-review-menu-label">Activity</div>
              {([{ kind: 'last-turn' }] as const).map((scope) => (
                <div key={scope.kind}>
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={scopeMatches(value, scope)}
                    onClick={() => choose(scope)}
                  >
                    {scopeIcon(scope)}
                    <span className="scope-menu-main">
                      <span>{reviewScopeLabel(scope, currentTurn)}</span>
                      {counts[scope.kind] !== undefined ? (
                        <>
                          <small aria-hidden>{counts[scope.kind]}</small>
                          <span className="shui-sr-only">{counts[scope.kind]} files</span>
                        </>
                      ) : null}
                    </span>
                    <span className="scope-selected">{scopeMatches(value, scope) ? <Check aria-hidden /> : null}</span>
                  </button>
                </div>
              ))}
              {turns.length > 0 ? (
                <button type="button" role="menuitem" onClick={() => setSubMenu('turns')}>
                  <MessagesSquare aria-hidden className="menu-icon" />
                  <span>Turns</span>
                  <ChevronRight aria-hidden />
                </button>
              ) : null}
              <div className="shui-review-menu-label">Working tree</div>
              {workingTreeScopes.map((scope) => (
                <div key={scope.kind}>
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={scopeMatches(value, scope)}
                    onClick={() => choose(scope)}
                  >
                    {scopeIcon(scope)}
                    <span className="scope-menu-main">
                      <span>{reviewScopeLabel(scope)}</span>
                      {counts[scope.kind] !== undefined ? (
                        <>
                          <small aria-hidden>{counts[scope.kind]}</small>
                          <span className="shui-sr-only">{counts[scope.kind]} files</span>
                        </>
                      ) : null}
                    </span>
                    <span className="scope-selected">{scopeMatches(value, scope) ? <Check aria-hidden /> : null}</span>
                  </button>
                </div>
              ))}
              <div className="shui-review-menu-label">History</div>
              <button type="button" role="menuitem" onClick={() => setSubMenu('commits')}>
                <GitCommitHorizontal aria-hidden className="menu-icon" />
                <span>Commits</span>
                <ChevronRight aria-hidden />
              </button>
              <button type="button" role="menuitem" onClick={() => setSubMenu('branches')}>
                <GitBranch aria-hidden className="menu-icon" />
                <span>Branches</span>
                <ChevronRight aria-hidden />
              </button>
            </>
          ) : (
            <>
              <button type="button" className="submenu-back" onClick={() => setSubMenu(null)}>
                <ChevronLeft aria-hidden />
                <span>{subMenu === 'commits' ? 'Commits' : subMenu === 'branches' ? 'Branches' : 'Turns'}</span>
              </button>
              <div className="shui-review-menu-separator" />
              {subMenu === 'turns'
                ? turns.map((turn) => (
                    <button
                      key={turn.turnId}
                      type="button"
                      className="scope-detail"
                      role="menuitemradio"
                      aria-checked={value.kind === 'turn' && value.turnId === turn.turnId}
                      title={turn.turnId}
                      onClick={() =>
                        choose({
                          kind: 'turn',
                          turnId: turn.turnId,
                          label: turn.label,
                        })
                      }
                    >
                      <MessagesSquare aria-hidden />
                      <span className="scope-detail-main">
                        <span>{turn.label}</span>
                        <small>
                          {turn.active ? 'running' : turn.turnId.slice(0, 10)} · {turn.fileCount}{' '}
                          {turn.fileCount === 1 ? 'file' : 'files'}
                        </small>
                      </span>
                      {value.kind === 'turn' && value.turnId === turn.turnId ? <Check aria-hidden /> : null}
                    </button>
                  ))
                : null}
              {subMenu !== 'turns' && metadataLoading ? <div className="shui-review-scope-note">loading…</div> : null}
              {subMenu !== 'turns' && !metadataLoading && metadataError ? (
                <div className="shui-review-scope-note warn">{metadataError}</div>
              ) : null}
              {!metadataLoading && metadataError === null && subMenu === 'commits' && commits.length === 0 ? (
                <div className="shui-review-scope-note">no commits</div>
              ) : null}
              {!metadataLoading && metadataError === null && subMenu === 'branches' && branches.length === 0 ? (
                <div className="shui-review-scope-note">no branches</div>
              ) : null}
              {subMenu === 'commits'
                ? commits.map((commit) => (
                    <button
                      key={commit.sha}
                      type="button"
                      className="scope-detail"
                      role="menuitemradio"
                      aria-checked={value.kind === 'commit' && value.sha === commit.sha}
                      title={`${commit.sha} ${commit.subject}`}
                      onClick={() => choose({ kind: 'commit', ...commit })}
                    >
                      <GitCommitHorizontal aria-hidden />
                      <span className="scope-detail-main">
                        <span>{commit.subject || 'untitled commit'}</span>
                        <code>{commit.sha.slice(0, 8)}</code>
                      </span>
                      {value.kind === 'commit' && value.sha === commit.sha ? <Check aria-hidden /> : null}
                    </button>
                  ))
                : null}
              {subMenu === 'branches'
                ? branches.map((branch) => (
                    <button
                      key={branch.ref}
                      type="button"
                      className="scope-detail"
                      role="menuitemradio"
                      aria-checked={value.kind === 'branch' && value.ref === branch.ref}
                      title={branch.ref}
                      onClick={() =>
                        choose({
                          kind: 'branch',
                          ref: branch.ref,
                          name: branch.name,
                        })
                      }
                    >
                      <GitBranch aria-hidden />
                      <span className="scope-detail-main">
                        <span>{branch.name}</span>
                        {branch.current ? <small>current</small> : null}
                      </span>
                      {value.kind === 'branch' && value.ref === branch.ref ? <Check aria-hidden /> : null}
                    </button>
                  ))
                : null}
            </>
          )}
        </div>
      ) : null}
    </div>
  )
}
