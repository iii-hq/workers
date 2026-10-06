import { Fragment, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ChangesTree } from '../ChangesTree'
import { changeRows } from '../commit-tree'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({ Checkbox: () => null, IconButton: () => null }))
// Every row mounts: server rendering measures no viewport.
vi.mock('../VirtualList', () => ({
  VirtualList: <T,>(props: {
    rows: readonly T[]
    rowKey(row: T, at: number): string
    renderRow(row: T, at: number): ReactNode
  }) => props.rows.map((row, at) => <Fragment key={props.rowKey(row, at)}>{props.renderRow(row, at)}</Fragment>),
}))

/** styles.css as its innermost rules: selector and declarations, comments dropped. */
async function rules(): Promise<Array<{ selector: string; body: string }>> {
  // Typed by hand: the worker UI tsconfig carries no Node types.
  const fs: { readFileSync(path: URL, encoding: 'utf8'): string } = await import('node:fs' as string)
  const css = fs.readFileSync(new URL('../../../styles.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '')
  return [...css.matchAll(/([^{};]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body }))
}

describe('the menu styles in styles.css', () => {
  it('keep every IDE menu raised over the panel it opens from', async () => {
    const menu = (await rules()).filter((rule) => rule.selector === '[role="menu"]')
    const shadow = menu.map((rule) => /box-shadow:([^;]*)/.exec(rule.body)?.[1] ?? '').join(' ')
    expect(shadow).toContain('var(--shadow-raised)')
    expect(shadow).toContain('var(--shadow-floating)')
  })

  it("style each mark ChangesTree sets on a menu's rows, the target apart from its scope", async () => {
    const files = ['src/a.ts', 'src/lib/b.ts', 'README.md'].map((path) => ({ path, status: 'modified' as const }))
    const rows = changeRows([{ id: 'changes', label: 'Changes', entries: files }], {
      byDirectory: true,
      isOpen: (_key, fallback) => fallback,
    })
    const folder = rows.find((row) => row.kind === 'folder')?.key ?? null
    const html = renderToStaticMarkup(<ChangesTree rows={rows} onToggleOpen={() => {}} menuTarget={folder} />)
    const marks = [...new Set([...html.matchAll(/data-menu="([^"]*)"/g)].map(([, mark]) => mark))]
    expect(marks.sort()).toEqual(['scope', 'target'])

    const all = await rules()
    // The rules a row with this mark matches.
    const styling = (mark: string) =>
      all.filter(
        (rule) =>
          rule.selector.includes('.shui-ctree-row') &&
          (rule.selector.includes('[data-menu]') || rule.selector.includes(`[data-menu="${mark}"]`)) &&
          /background/.test(rule.body),
      )
    for (const mark of marks) expect(styling(mark), mark).not.toEqual([])
    // What such a row ends up with: the value of the last of them that sets it.
    const value = (mark: string, property: string) =>
      styling(mark)
        .map((rule) => new RegExp(`(?:^|[\\s;])${property}:([^;]*)`).exec(rule.body)?.[1]?.trim())
        .filter(Boolean)
        .pop()
    expect(value('scope', 'background')).not.toBe(value('target', 'background'))
    expect(value('scope', 'outline')).toBe('none')
  })
})
