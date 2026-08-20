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

export function reviewScopeLabel(scope: ReviewScopeSelection): string {
  switch (scope.kind) {
    case 'last-turn':
      return 'Last Turn'
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
  metadataLoading: boolean
  metadataError: string | null
  onOpen: () => void
  onChange: (scope: ReviewScopeSelection) => void
}) {
  const [open, setOpen] = useState(false)
  const [subMenu, setSubMenu] = useState<'committed' | 'branch' | 'turns' | null>(null)
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

  const primary: readonly ReviewScopeSelection[] = [
    { kind: 'last-turn' },
    { kind: 'uncommitted' },
    { kind: 'unstaged' },
    { kind: 'staged' },
  ]

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
        <span>{reviewScopeLabel(value)}</span>
        <ChevronDown aria-hidden />
      </button>
      {open ? (
        <div className="shui-review-scope-menu" role="menu">
          {subMenu === null ? (
            <>
              {primary.map((scope, index) => (
                <div key={scope.kind}>
                  {index === 1 ? <div className="shui-review-menu-separator" /> : null}
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={scopeMatches(value, scope)}
                    onClick={() => choose(scope)}
                  >
                    {scopeIcon(scope)}
                    <span>{reviewScopeLabel(scope)}</span>
                    {scopeMatches(value, scope) ? <Check aria-hidden /> : <span className="menu-icon-gap" />}
                  </button>
                </div>
              ))}
              <div className="shui-review-menu-separator" />
              {turns.length > 0 ? (
                <button type="button" role="menuitem" onClick={() => setSubMenu('turns')}>
                  <MessagesSquare aria-hidden className="menu-icon" />
                  <span>Turns</span>
                  <ChevronRight aria-hidden />
                </button>
              ) : null}
              <button type="button" role="menuitem" onClick={() => setSubMenu('committed')}>
                <GitCommitHorizontal aria-hidden className="menu-icon" />
                <span>Committed</span>
                <ChevronRight aria-hidden />
              </button>
              <button type="button" role="menuitem" onClick={() => setSubMenu('branch')}>
                <GitBranch aria-hidden className="menu-icon" />
                <span>Branch</span>
                <ChevronRight aria-hidden />
              </button>
            </>
          ) : (
            <>
              <button type="button" className="submenu-back" onClick={() => setSubMenu(null)}>
                <ChevronLeft aria-hidden />
                <span>
                  {subMenu === 'committed' ? 'Committed' : subMenu === 'branch' ? 'Branch' : 'Turns'}
                </span>
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
                      onClick={() => choose({ kind: 'turn', turnId: turn.turnId, label: turn.label })}
                    >
                      <MessagesSquare aria-hidden />
                      <span className="scope-detail-main">
                        <span>{turn.label}</span>
                        <small>{turn.active ? 'running' : turn.turnId.slice(0, 10)}</small>
                      </span>
                      {value.kind === 'turn' && value.turnId === turn.turnId ? <Check aria-hidden /> : null}
                    </button>
                  ))
                : null}
              {subMenu !== 'turns' && metadataLoading ? <div className="shui-review-scope-note">loading…</div> : null}
              {subMenu !== 'turns' && !metadataLoading && metadataError ? (
                <div className="shui-review-scope-note warn">{metadataError}</div>
              ) : null}
              {!metadataLoading && metadataError === null && subMenu === 'committed' && commits.length === 0 ? (
                <div className="shui-review-scope-note">no commits</div>
              ) : null}
              {!metadataLoading && metadataError === null && subMenu === 'branch' && branches.length === 0 ? (
                <div className="shui-review-scope-note">no branches</div>
              ) : null}
              {subMenu === 'committed'
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
                : branches.map((branch) => (
                    <button
                      key={branch.ref}
                      type="button"
                      className="scope-detail"
                      role="menuitemradio"
                      aria-checked={value.kind === 'branch' && value.ref === branch.ref}
                      title={branch.ref}
                      onClick={() => choose({ kind: 'branch', ref: branch.ref, name: branch.name })}
                    >
                      <GitBranch aria-hidden />
                      <span className="scope-detail-main">
                        <span>{branch.name}</span>
                        {branch.current ? <small>current</small> : null}
                      </span>
                      {value.kind === 'branch' && value.ref === branch.ref ? <Check aria-hidden /> : null}
                    </button>
                  ))}
            </>
          )}
        </div>
      ) : null}
    </div>
  )
}
