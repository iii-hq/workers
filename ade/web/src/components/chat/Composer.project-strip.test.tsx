import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  getExtComposerControls,
  type RegisteredComposerControl,
  registerExtComposerControl,
} from '@/lib/ui-slots'
import type { ComposerControlProps } from '@/types/injectable-ui'
import { Composer, composerControlNodes } from './Composer'

function control(
  id: string,
  placement?: RegisteredComposerControl['placement'],
): RegisteredComposerControl {
  return {
    id,
    placement,
    path: `${id}/page.js`,
    scope: id,
    render: ({ workingDir }: ComposerControlProps) => (
      <span data-control={id} data-dir={workingDir ?? ''} />
    ),
  }
}

function renderComposer(
  controls: { footer: ReactNode; project: ReactNode },
  showWorkingDir = true,
) {
  return renderToStaticMarkup(
    <Composer
      model={null}
      modelOptions={[]}
      permissionMode="manual"
      thinkingLevel="default"
      showWorkingDir={showWorkingDir}
      workingDir="/repo"
      onWorkingDirChange={vi.fn()}
      onModelChange={vi.fn()}
      onThinkingLevelChange={vi.fn()}
      onPermissionModeChange={vi.fn()}
      onSubmit={vi.fn()}
      composerControls={controls.footer}
      projectControls={controls.project}
    />,
  )
}

describe('composer controls placement', () => {
  it('keeps project controls beside the folder inside the composer', () => {
    const offs = [
      registerExtComposerControl(control('ide-worktree', 'project')),
      registerExtComposerControl(control('judge-provider')),
    ]
    const controls = composerControlNodes(getExtComposerControls(), {
      sessionId: 's1',
      isStreaming: false,
      metadata: {},
      setMetadata: vi.fn(),
      workingDir: '/repo',
    })
    for (const off of offs) off()

    const html = renderComposer(controls)
    const card = html.indexOf('composer-card')
    const context = html.indexOf('composer-project-context')
    const toolbar = html.indexOf('composer-mobile-toolbar')
    expect(context).toBeGreaterThan(card)
    const project = html.slice(context, toolbar)
    expect(project).toContain('data-control="ide-worktree" data-dir="/repo"')
    expect(project).not.toContain('judge-provider')
    expect(html.slice(toolbar)).toContain('data-control="judge-provider"')
    const folder = project.indexOf(
      'aria-label="select project folder, current folder: repo"',
    )
    const worktree = project.indexOf('data-control="ide-worktree"')
    expect(folder).toBeGreaterThan(-1)
    expect(worktree).toBeGreaterThan(folder)
    expect(project.indexOf('collapse composer')).toBeGreaterThan(worktree)

    // No strip, no project controls.
    expect(renderComposer(controls, false)).not.toContain('ide-worktree')
  })
})
