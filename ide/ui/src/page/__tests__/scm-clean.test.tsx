import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { CommitView } from '../CommitView'
import { sameFolder, scmCleanCopy } from '../scm-clean'
import { sourceControlDetail } from '../ShellLauncher'
import type { SourceControlState } from '../use-source-control'

vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <>{children}</>
  return {
    Button: Pass,
    Checkbox: () => <input type="checkbox" />,
    ConfirmDialog: () => null,
    DropdownMenu: Pass,
    DropdownMenuCheckboxItem: Pass,
    DropdownMenuContent: Pass,
    DropdownMenuGroup: Pass,
    DropdownMenuLabel: Pass,
    DropdownMenuTrigger: Pass,
    IconButton: Pass,
    Skeleton: () => null,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    StatusPanel: ({ headline }: { headline: string }) => <p>{headline}</p>,
    uiClasses: {},
    Tooltip: Pass,
  }
})
vi.mock('../CommitBox', () => ({ CommitBox: () => <textarea aria-label="Commit message" /> }))
vi.mock('../RollbackDialog', () => ({ RollbackDialog: () => null }))
vi.mock('../TextDialog', () => ({ TextDialog: () => null }))
vi.mock('../RepoSetup', () => ({ RepoSetup: () => null }))

function emptyCommitView(root: string, worktree: { top: string; dirty: boolean } | null) {
  const scm = {
    phase: 'ready',
    branch: 'feat/ade-design-system',
    changes: [],
    unversioned: [],
    included: [],
    isIncluded: () => true,
    error: null,
  }
  return renderToStaticMarkup(
    <CommitView
      host={{} as never}
      root={root}
      scm={scm as unknown as SourceControlState}
      activePath={null}
      onOpenChange={() => {}}
      onOpenFile={() => {}}
      onCompare={() => {}}
      onShowHistory={() => {}}
      onDeleteFile={async () => {}}
      worktree={worktree}
    />,
  )
}

describe('the empty Commit list names its scope', () => {
  // QA 2026-11-16: opened on `ide/`, the panel said "Everything is committed
  // on feat/ade-design-system" beside the branch chip's orange dirty dot
  // (the changes were outside `ide/`).
  it('a clean subfolder of a dirty worktree does not claim everything is committed', () => {
    const html = emptyCommitView('/repo/ide', { top: '/repo', dirty: true })
    expect(html).toContain('No changes in this folder')
    expect(html).toContain('Other folders in this repository have uncommitted changes')
    expect(html).not.toContain('Everything is committed')
  })

  it('a clean worktree says everything is committed, on its branch', () => {
    const html = emptyCommitView('/repo', { top: '/repo/', dirty: false })
    expect(html).toContain('<span class="title">No changes</span>')
    expect(html).toContain('Everything is committed on <code class="shui-commit-clean-branch">feat/ade-design-system</code>')
  })

  it('a clean subfolder of a clean worktree scopes the sentence to the folder', () => {
    expect(scmCleanCopy({ branch: 'main', inSubfolder: true, worktreeDirty: false })).toEqual({
      title: 'No changes',
      detail: 'Everything in this folder is committed',
      branch: 'main',
    })
  })

  it('compares folders without a trailing slash mattering', () => {
    expect(sameFolder('/repo/', '/repo')).toBe(true)
    expect(sameFolder('/repo/ide', '/repo')).toBe(false)
  })

  it('the launcher says "in this folder" for a subfolder, "working tree clean" at the top', () => {
    const ready = (prefix: string) =>
      ({ kind: 'ready', changes: [], status: { prefix, entries: [], branch: 'main' } }) as never
    expect(sourceControlDetail(ready('ide/'))).toBe('No changes in this folder')
    expect(sourceControlDetail(ready(''))).toBe('Working tree clean')
  })
})
