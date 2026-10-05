import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { CommitView } from '../CommitView'
import type { SourceControlPhase, SourceControlState } from '../use-source-control'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <>{children}</>
  return {
    Button: Pass,
    Checkbox: () => <input type="checkbox" />,
    ConfirmDialog: () => null,
    DropdownMenu: Pass,
    DropdownMenuCheckboxItem: Pass,
    DropdownMenuContent: Pass,
    DropdownMenuLabel: Pass,
    DropdownMenuTrigger: Pass,
    IconButton: Pass,
    Skeleton: () => null,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    StatusPanel: ({ headline }: { headline: string }) => <p>{headline}</p>,
    uiClasses: {},
  }
})
vi.mock('../CommitBox', () => ({ CommitBox: () => <textarea aria-label="Commit message" /> }))
vi.mock('../RollbackDialog', () => ({ RollbackDialog: () => null }))
vi.mock('../TextDialog', () => ({ TextDialog: () => null }))

function render(phase: SourceControlPhase, unversioned: { path: string; status: 'untracked' }[] = []) {
  const scm = {
    phase,
    branch: 'main',
    changes: [],
    unversioned,
    included: [],
    isIncluded: () => true,
    error: 'boom',
  }
  return renderToStaticMarkup(
    <CommitView
      host={{} as never}
      root="/repo"
      scm={scm as unknown as SourceControlState}
      activePath={null}
      onOpenChange={() => {}}
      onOpenFile={() => {}}
      onCompare={() => {}}
      onShowHistory={() => {}}
    />,
  )
}

describe('the Commit tab', () => {
  it('keeps the commit box in place, hidden, while the status cannot be read', () => {
    // The box sits in the same element in every phase, so React keeps it,
    // and the message being typed, across a failed read.
    expect(render('ready')).toMatch(/^<div class="shui-commit"><div class="shui-commit-toolbar".*<textarea/)
    expect(render('error')).toMatch(
      /^<div class="shui-commit-error">.*<\/div><div class="shui-commit" hidden="">.*<textarea/,
    )
    expect(render('not-a-repo')).toMatch(
      /^<div class="shui-side-empty">.*<\/div><div class="shui-commit" hidden="">.*<textarea/,
    )
  })

  it('mounts only the rows in view, however many files changed', () => {
    const unversioned = Array.from({ length: 3242 }, (_, index) => ({
      // At the root: the tree view folds unversioned folders.
      path: `file-${index}.ts`,
      status: 'untracked' as const,
    }))
    const html = render('ready', unversioned)
    expect(html).toContain('3242 files')
    // Before measuring its viewport the window holds 1 row plus the overscan.
    expect(html.match(/class="shui-ctree-row"/g)?.length).toBeLessThanOrEqual(10)
  })
})
