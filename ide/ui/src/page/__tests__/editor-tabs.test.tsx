import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { EditorTabs } from '../EditorTabs'
import type { OpenTab } from '../tabs'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: ReactNode }) => <>{children}</>,
}))
vi.mock('../ContextMenu', () => ({
  anchorFromEvent: () => null,
  useContextMenu: () => ({ open: () => {}, element: null }),
}))
vi.mock('../file-type-icon', () => ({ FileTypeIcon: () => null }))

const noop = () => {}

function render(tabs: OpenTab[]) {
  return renderToStaticMarkup(
    <EditorTabs
      tabs={{ tabs, active: null }}
      dirtyPaths={new Set()}
      tabVisible
      gitStatus={new Map([['a.ts', 'modified']])}
      turnTitles={new Map()}
      terminal={null}
      onActivate={noop}
      onClose={noop}
      onPin={noop}
      onCloseOthers={noop}
      onCloseRight={noop}
      onCloseSaved={noop}
      onCloseAll={noop}
      onReveal={noop}
      onCopyPath={noop}
      onCompare={noop}
      onOpenFile={noop}
    />,
  )
}

describe('the editor tabs', () => {
  it("colours a file by its Git status, never a commit's version of it", () => {
    const html = render([
      { id: 'file:a.ts', target: { kind: 'file', path: 'a.ts' }, pinned: true },
      { id: 'revision:abc1234def:a.ts', target: { kind: 'revision', path: 'a.ts', sha: 'abc1234def' }, pinned: true },
    ])
    expect(html).toMatch(/data-tab-id="file:a.ts" data-status="modified"/)
    expect(html).toMatch(/data-tab-id="revision:abc1234def:a.ts" data-kind="revision"/)
    expect(html).toContain('a.ts @ abc1234')
  })
})
