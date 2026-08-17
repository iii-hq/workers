import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  GitBranch,
  GitCommitHorizontal,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

export type ReviewScopeSelection =
  | { kind: 'last-turn' }
  | { kind: 'uncommitted' }
  | { kind: 'unstaged' }
  | { kind: 'staged' }
  | { kind: 'commit'; sha: string; subject: string }
  | { kind: 'branch'; ref: string; name: string }

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
  }
}

function scopeMatches(left: ReviewScopeSelection, right: ReviewScopeSelection): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === 'commit' && right.kind === 'commit') return left.sha === right.sha
  if (left.kind === 'branch' && right.kind === 'branch') return left.ref === right.ref
  return true
}

export function ReviewScopePicker({
  value,
  commits,
  branches,
  metadataLoading,
  metadataError,
  onOpen,
  onChange,
}: {
  value: ReviewScopeSelection
  commits: readonly ReviewCommitChoice[]
  branches: readonly ReviewBranchChoice[]
  metadataLoading: boolean
  metadataError: string | null
  onOpen: () => void
  onChange: (scope: ReviewScopeSelection) => void
}) {
  const [open, setOpen] = useState(false)
  const [subMenu, setSubMenu] = useState<'committed' | 'branch' | null>(null)
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
                    <span>{reviewScopeLabel(scope)}</span>
                    {scopeMatches(value, scope) ? <Check aria-hidden /> : <span className="menu-icon-gap" />}
                  </button>
                </div>
              ))}
              <div className="shui-review-menu-separator" />
              <button type="button" role="menuitem" onClick={() => setSubMenu('committed')}>
                <span>Committed</span>
                <ChevronRight aria-hidden />
              </button>
              <button type="button" role="menuitem" onClick={() => setSubMenu('branch')}>
                <span>Branch</span>
                <ChevronRight aria-hidden />
              </button>
            </>
          ) : (
            <>
              <button type="button" className="submenu-back" onClick={() => setSubMenu(null)}>
                <ChevronLeft aria-hidden />
                <span>{subMenu === 'committed' ? 'Committed' : 'Branch'}</span>
              </button>
              <div className="shui-review-menu-separator" />
              {metadataLoading ? <div className="shui-review-scope-note">loading…</div> : null}
              {!metadataLoading && metadataError ? (
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
