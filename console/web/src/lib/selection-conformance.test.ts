import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const repoRoot = fileURLToPath(new URL('../../../../', import.meta.url))

function cssRuleBodies(path: string, selector: string): string[] {
  const css = readFileSync(join(repoRoot, path), 'utf8')
  const bodies: string[] = []
  let searchAt = 0
  while (searchAt < css.length) {
    const selectorAt = css.indexOf(selector, searchAt)
    if (selectorAt < 0) break
    const bodyAt = css.indexOf('{', selectorAt)
    const bodyEnd = css.indexOf('}', bodyAt)
    if (bodyAt < 0 || bodyEnd < 0) break
    bodies.push(css.slice(bodyAt + 1, bodyEnd))
    searchAt = bodyEnd + 1
  }
  expect(bodies.length, `${path}: ${selector}`).toBeGreaterThan(0)
  return bodies
}

const neutralSelectionRules: Array<[path: string, selector: string]> = [
  ['console/ui/styles.css', '.console-catalog-row[data-selected="true"]'],
  ['computer/ui/styles.css', '.cp-ui-rail-row.active::before'],
  ['state/ui/styles.css', '.state-ui-nav-row.active::before'],
  ['memory/ui/styles.css', '.mem-ui-nav-row.active::before'],
  ['memory/ui/styles.css', '.mem-ui-tagbtn.active'],
  ['iii-directory/ui/styles.css', '.dir-ui-nav-row.active::before'],
  ['worktree/ui/styles.css', '.wt-ui-edge.selected'],
  ['worktree/ui/styles.css', '.wt-ui-node.selected::before'],
  ['database/ui/styles.css', '.db-tree-row.active::before'],
  ['database/ui/styles.css', '.db-erd-node.selected'],
  ['browser/ui/styles.css', '.br-ui-rail-row.active'],
  ['browser/ui/styles.css', '.br-ui-rail-row.active .br-ui-rail-icon'],
  [
    'console/web/src/styles/ui-recipes.css',
    '.iii-ui-tab[aria-selected="true"]',
  ],
  ['browser/ui/styles.css', '.br-cfg-nav-row.active::before'],
  ['storage/ui/styles.css', '.storage-ui-object-row.active::before'],
  ['storage/ui/styles.css', '.storage-cfg-nav-row.active::before'],
  ['github/ui/styles.css', '.gh-ui-node-selected'],
  ['github/ui/styles.css', '.gh-ui-graph-row.selected'],
  ['eval/ui/styles.css', '.eval-ui-history-row.active'],
  ['eval/ui/styles.css', '.eval-ui-tabs button.active'],
  ['eval/ui/styles.css', '.eval-ui-session-option.selected'],
  ['eval/ui/styles.css', '.eval-ui-run-card.selected'],
  ['eval/ui/styles.css', '.eval-ui-run-row.active'],
  ['editor/ui/styles.css', '.ed-seg button[data-active="true"]'],
  ['editor/ui/styles.css', '.ed-row[data-active="true"]'],
  ['editor/ui/styles.css', '.ed-tab[data-active="true"]'],
  ['shell/ui/styles.css', '.shui-etab.active'],
  ['shell/ui/styles.css', '.shui-terminal-tab.active'],
  [
    'console/ui/styles.css',
    '.console-catalog-header-toggle[aria-pressed="true"]',
  ],
  ['sandbox-code-runner/ui/src/styles/page.css', '.cr-page-toggle.on'],
]

describe('neutral selection contract', () => {
  it.each(neutralSelectionRules)(
    '%s keeps %s free from theme accent colors',
    (path, selector) => {
      for (const body of cssRuleBodies(path, selector)) {
        expect(body).not.toContain('--color-accent')
        expect(body).not.toContain('accent-muted')
        expect(body).not.toContain('accent-border')
      }
    },
  )

  it('keeps selected native Console labels and outlines neutral', () => {
    const sources = [
      'console/web/src/components/chat/DirectoryPicker.tsx',
      'console/web/src/components/workspace/EmptyPane.tsx',
      'console/web/src/components/workspace/MobileWorkspaceMenu.tsx',
      'console/web/src/components/chat/ModelPicker.tsx',
      'console/web/src/components/chat/SessionAddonsPicker.tsx',
      'console/web/src/components/chat/MemoryChip.tsx',
      'console/web/src/pages/Configuration/tabs/WorkersTab/WorkersList.tsx',
      'console/web/src/pages/TracesV2/components/ViewsDropdown.tsx',
      'console/web/src/pages/TracesV2/components/WaterfallChart.tsx',
      'console/web/src/pages/TracesV2/components/timeline/spanVisuals.tsx',
    ].map((path) => readFileSync(join(repoRoot, path), 'utf8'))

    for (const source of sources) {
      expect(source).not.toMatch(
        /(?:isSelected|selected|active)[^\n]{0,80}(?:text|stroke|outline|border|bg)-accent/,
      )
    }
  })

  it('keeps command palette filters and result selection neutral', () => {
    const source = readFileSync(
      join(repoRoot, 'console/web/src/components/CommandPalette.tsx'),
      'utf8',
    )

    expect(source).not.toMatch(
      /filter === option\.id[\s\S]{0,160}(?:text|border|bg)-accent/,
    )
    expect(source).not.toMatch(
      /selected[\s\S]{0,120}\?[\s\S]{0,80}(?:text|border|bg)-accent/,
    )
  })

  it('keeps Monaco selections and selected suggestions neutral', () => {
    const source = readFileSync(
      join(repoRoot, 'console/web/src/lib/monaco.ts'),
      'utf8',
    )
    expect(source).toContain("'editor.selectionBackground': surfaceSelected")
    expect(source).toContain(
      "'editorSuggestWidget.selectedBackground': surfaceSelected",
    )
    expect(source).not.toMatch(/selectedBackground': `\$\{accent\}/)
  })
})
